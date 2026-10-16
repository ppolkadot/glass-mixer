//! Application icon (and packaged-app display name) resolution.
//!
//! Runs on its own STA thread, because shell image extraction expects a
//! single-threaded apartment and can touch the disk; the UI never blocks on it.
//! Resolution order:
//! 1. The icon the app set on its audio session (`IAudioSessionControl::GetIconPath`).
//! 2. Packaged apps: the AppsFolder shell item for the AUMID (package resources).
//! 3. Desktop apps: the executable's icon via `IShellItemImageFactory`.
//! 4. Otherwise none: the UI draws a consistent generic tile.
//!
//! Successful results are cached per app and pixel size. Failed lookups can be
//! retried on Refresh or when an app returns with a new session.

use crate::audio::com::{ComGuard, take_pwstr};
use crate::audio::{AppEntry, AppKey};
use crossbeam_channel::{Receiver, Sender};
use egui::ColorImage;
use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    thread::JoinHandle,
};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDC, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::UI::Shell::{
    FOLDERID_AppsFolder, IShellItem, IShellItemImageFactory, KF_FLAG_DONT_VERIFY, SHCreateItemFromParsingName,
    SHCreateItemInKnownFolder, SHDefExtractIconW, SIGDN_NORMALDISPLAY, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
use windows::core::{HSTRING, Interface};

pub type IconKey = (AppKey, u32);

#[derive(Clone, Debug, Default)]
pub struct IconResult {
    pub image: Option<ColorImage>,
    /// Display name from the shell (used for packaged apps).
    pub display_name: Option<String>,
}

struct Request {
    key: IconKey,
    exe_path: Option<PathBuf>,
    aumid: Option<String>,
    icon_resource: Option<String>,
}

enum WorkerRequest {
    Resolve(Request),
    InvalidateFailed(Option<AppKey>),
    Retain { live: HashSet<AppKey>, pixel_size: Option<u32> },
}

pub struct IconService {
    tx: Option<Sender<WorkerRequest>>,
    ready: Arc<Mutex<Vec<(IconKey, IconResult)>>>,
    requested: HashSet<IconKey>,
    failed: HashSet<IconKey>,
    active: HashSet<AppKey>,
    pixel_size: Option<u32>,
    thread: Option<JoinHandle<()>>,
}

impl IconService {
    /// A service with no worker behind it, for UI tests: requests are remembered, never resolved.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Self {
            tx: None,
            ready: Arc::new(Mutex::new(Vec::new())),
            requested: HashSet::new(),
            failed: HashSet::new(),
            active: HashSet::new(),
            pixel_size: None,
            thread: None,
        }
    }

    /// `on_ready` is called from the worker thread whenever a result is available.
    pub fn start(on_ready: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<WorkerRequest>();
        let ready = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let ready = ready.clone();
            std::thread::Builder::new().name("icon-resolver".into()).spawn(move || worker(rx, ready, on_ready)).ok()
        };
        Self {
            tx: Some(tx),
            ready,
            requested: HashSet::new(),
            failed: HashSet::new(),
            active: HashSet::new(),
            pixel_size: None,
            thread,
        }
    }

    /// Requests resolution for an app at a pixel size, once.
    pub fn request(&mut self, app: &AppEntry, px: u32) {
        let key = (app.key.clone(), px);
        if app.key == AppKey::System && app.icon_resource.is_none() {
            return;
        }
        if !self.requested.insert(key.clone()) {
            return;
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(WorkerRequest::Resolve(Request {
                key,
                exe_path: app.exe_path.clone(),
                aumid: app.aumid.clone(),
                icon_resource: app.icon_resource.clone(),
            }));
        }
    }

    /// Results completed since the last call.
    pub fn take_ready(&mut self) -> Vec<(IconKey, IconResult)> {
        let ready = std::mem::take(&mut *self.ready.lock());
        for (key, result) in &ready {
            if result.image.is_none() {
                self.failed.insert(key.clone());
            } else {
                self.failed.remove(key);
            }
        }
        ready
    }

    pub fn retry_failed(&mut self) {
        for key in self.failed.drain() {
            self.requested.remove(&key);
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(WorkerRequest::InvalidateFailed(None));
        }
    }

    pub fn sync_live_apps(&mut self, keys: impl IntoIterator<Item = AppKey>) {
        let live: HashSet<AppKey> = keys.into_iter().collect();
        if live == self.active {
            return;
        }
        for key in live.difference(&self.active) {
            let failed: Vec<_> = self.failed.iter().filter(|icon| &icon.0 == key).cloned().collect();
            if !failed.is_empty() {
                for icon in failed {
                    self.failed.remove(&icon);
                    self.requested.remove(&icon);
                }
                if let Some(tx) = &self.tx {
                    let _ = tx.send(WorkerRequest::InvalidateFailed(Some(key.clone())));
                }
            }
        }
        self.requested.retain(|(key, _)| live.contains(key));
        if let Some(tx) = &self.tx {
            let _ = tx.send(WorkerRequest::Retain { live: live.clone(), pixel_size: self.pixel_size });
        }
        self.active = live;
    }

    pub fn prepare_pixel_size(&mut self, px: u32) {
        if self.pixel_size == Some(px) {
            return;
        }
        self.pixel_size = Some(px);
        self.requested.retain(|(_, size)| *size == px);
        self.failed.retain(|(_, size)| *size == px);
        if let Some(tx) = &self.tx {
            let _ = tx.send(WorkerRequest::Retain { live: self.active.clone(), pixel_size: Some(px) });
        }
    }

    pub fn shutdown(&mut self) {
        self.tx = None; // closes the channel; the worker exits
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for IconService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker(rx: Receiver<WorkerRequest>, ready: Arc<Mutex<Vec<(IconKey, IconResult)>>>, on_ready: impl Fn()) {
    let _com = ComGuard::apartment();
    let mut cache: HashMap<IconKey, IconResult> = HashMap::new();
    for message in rx {
        let req = match message {
            WorkerRequest::Resolve(req) => req,
            WorkerRequest::InvalidateFailed(key) => {
                cache.retain(|(app, _), result| {
                    result.image.is_some() || key.as_ref().is_some_and(|wanted| wanted != app)
                });
                continue;
            }
            WorkerRequest::Retain { live, pixel_size } => {
                cache.retain(|(key, size), _| live.contains(key) && pixel_size.is_none_or(|px| *size == px));
                continue;
            }
        };
        let result = match cache.get(&req.key) {
            Some(r) => r.clone(),
            None => {
                let r = resolve(&req);
                cache.insert(req.key.clone(), r.clone());
                r
            }
        };
        ready.lock().push((req.key, result));
        on_ready();
    }
}

fn resolve(req: &Request) -> IconResult {
    let px = req.key.1.clamp(16, 256) as i32;
    let mut result = IconResult::default();

    if let Some(res) = &req.icon_resource {
        result.image = icon_from_resource(res, px);
    }
    if let Some(aumid) = &req.aumid
        && let Ok(item) = unsafe {
            SHCreateItemInKnownFolder::<_, IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DONT_VERIFY, &HSTRING::from(aumid))
        }
    {
        result.display_name =
            unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.ok().map(take_pwstr).filter(|s| !s.is_empty());
        if result.image.is_none() {
            result.image = shell_item_image(&item, px);
        }
    }
    if result.image.is_none()
        && let Some(path) = &req.exe_path
        && let Ok(item) =
            unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path.as_os_str()), None) }
    {
        result.image = shell_item_image(&item, px);
    }
    if result.image.is_none() {
        log::debug!("no icon for {:?}; using fallback", req.key.0);
    }
    result
}

fn shell_item_image(item: &IShellItem, px: i32) -> Option<ColorImage> {
    let factory: IShellItemImageFactory = item.cast().ok()?;
    let hbmp = unsafe { factory.GetImage(SIZE { cx: px, cy: px }, SIIGBF_ICONONLY) }.ok()?;
    let image = hbitmap_to_image(hbmp);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
    }
    image
}

/// Parses `path,index` (optionally prefixed with `@`, with environment variables).
fn icon_from_resource(resource: &str, px: i32) -> Option<ColorImage> {
    let resource = resource.trim().trim_start_matches('@');
    let (path, index) = match resource.rsplit_once(',') {
        Some((p, i)) => (p, i.trim().parse::<i32>().unwrap_or(0)),
        None => (resource, 0),
    };
    let src = HSTRING::from(path.trim().trim_matches('"'));
    let mut buf = vec![0u16; 1024];
    let n = unsafe { ExpandEnvironmentStringsW(&src, Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return None;
    }
    let expanded = HSTRING::from_wide(&buf[..n.saturating_sub(1)]);
    let mut hicon = HICON::default();
    let hr = unsafe { SHDefExtractIconW(&expanded, index, 0, Some(&mut hicon), None, px as u32) };
    if hr.is_err() || hicon.is_invalid() {
        return None;
    }
    let image = hicon_to_image(hicon, px);
    unsafe {
        let _ = DestroyIcon(hicon);
    }
    image
}

fn top_down_info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn hbitmap_to_image(hbmp: HBITMAP) -> Option<ColorImage> {
    let mut bm = BITMAP::default();
    let got = unsafe {
        GetObjectW(HGDIOBJ(hbmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut _))
    };
    if got == 0 || bm.bmWidth <= 0 || bm.bmHeight == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    let mut info = top_down_info(w, h);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let lines = unsafe {
        let hdc = GetDC(None);
        let lines = GetDIBits(hdc, hbmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        lines
    };
    if lines == 0 {
        return None;
    }
    Some(bgra_to_image(buf, w as usize, h as usize))
}

fn hicon_to_image(hicon: HICON, px: i32) -> Option<ColorImage> {
    unsafe {
        let hdc = CreateCompatibleDC(None);
        let info = top_down_info(px, px);
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(hdc), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(hdc, HGDIOBJ(dib.0));
        let drawn = DrawIconEx(hdc, 0, 0, hicon, px, px, 0, None, DI_NORMAL);
        let buf = if drawn.is_ok() && !bits.is_null() {
            Some(std::slice::from_raw_parts(bits as *const u8, (px * px * 4) as usize).to_vec())
        } else {
            None
        };
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(hdc);
        buf.map(|b| bgra_to_image(b, px as usize, px as usize))
    }
}

/// Converts GDI BGRA to an egui image. Bitmaps without any alpha are treated as
/// opaque; bitmaps whose color exceeds alpha are straight (not premultiplied).
fn bgra_to_image(mut buf: Vec<u8>, w: usize, h: usize) -> ColorImage {
    let has_alpha = buf.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    let mut premultiplied = true;
    for p in buf.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
        if !has_alpha {
            p[3] = 255;
        } else if p[0] > p[3] || p[1] > p[3] || p[2] > p[3] {
            premultiplied = false;
        }
    }
    if premultiplied {
        ColorImage::from_rgba_premultiplied([w, h], &buf)
    } else {
        ColorImage::from_rgba_unmultiplied([w, h], &buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_icon_requests_can_retry_and_reappearing_apps_retry() {
        let key = AppKey::Exe("a.exe".into());
        let icon_key = (key.clone(), 28);
        let mut service = IconService {
            tx: None,
            ready: Arc::new(Mutex::new(Vec::new())),
            requested: HashSet::new(),
            failed: HashSet::new(),
            active: HashSet::new(),
            pixel_size: None,
            thread: None,
        };
        service.requested.insert(icon_key.clone());
        service.failed.insert(icon_key.clone());
        service.retry_failed();
        assert!(!service.requested.contains(&icon_key));
        service.requested.insert(icon_key.clone());
        service.failed.insert(icon_key.clone());
        service.sync_live_apps([key.clone()]);
        service.sync_live_apps([]);
        service.sync_live_apps([key]);
        assert!(!service.requested.contains(&icon_key));
    }

    #[test]
    fn returned_apps_and_dpi_changes_can_request_icons_again() {
        let key = AppKey::Exe("a.exe".into());
        let icon_key = (key.clone(), 28);
        let mut service = IconService {
            tx: None,
            ready: Arc::new(Mutex::new(Vec::new())),
            requested: HashSet::from([icon_key.clone()]),
            failed: HashSet::new(),
            active: HashSet::new(),
            pixel_size: None,
            thread: None,
        };
        service.sync_live_apps([key.clone()]);
        service.sync_live_apps([]);
        assert!(!service.requested.contains(&icon_key));
        service.requested.insert(icon_key.clone());
        service.prepare_pixel_size(42);
        assert!(!service.requested.contains(&icon_key));
    }

    #[test]
    fn opaque_bitmaps_get_full_alpha() {
        let img = bgra_to_image(vec![10, 20, 30, 0, 40, 50, 60, 0], 2, 1);
        assert_eq!(img.pixels[0], egui::Color32::from_rgb(30, 20, 10));
    }

    #[test]
    fn straight_alpha_is_detected() {
        // Color 200 > alpha 100 => straight alpha.
        let img = bgra_to_image(vec![0, 0, 200, 100], 1, 1);
        assert_eq!(img.pixels[0].a(), 100);
    }
}
