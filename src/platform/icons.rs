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
//! Results (including failures) are cached per app and pixel size for the
//! lifetime of the process, so each executable is read at most once per size.

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

pub struct IconService {
    tx: Option<Sender<Request>>,
    ready: Arc<Mutex<Vec<(IconKey, IconResult)>>>,
    requested: HashSet<IconKey>,
    thread: Option<JoinHandle<()>>,
}

impl IconService {
    /// `on_ready` is called from the worker thread whenever a result is available.
    pub fn start(on_ready: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Request>();
        let ready = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let ready = ready.clone();
            std::thread::Builder::new().name("icon-resolver".into()).spawn(move || worker(rx, ready, on_ready)).ok()
        };
        Self { tx: Some(tx), ready, requested: HashSet::new(), thread }
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
            let _ = tx.send(Request {
                key,
                exe_path: app.exe_path.clone(),
                aumid: app.aumid.clone(),
                icon_resource: app.icon_resource.clone(),
            });
        }
    }

    /// Results completed since the last call.
    pub fn take_ready(&self) -> Vec<(IconKey, IconResult)> {
        std::mem::take(&mut *self.ready.lock())
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

fn worker(rx: Receiver<Request>, ready: Arc<Mutex<Vec<(IconKey, IconResult)>>>, on_ready: impl Fn()) {
    let _com = ComGuard::apartment();
    let mut cache: HashMap<IconKey, IconResult> = HashMap::new();
    for req in rx {
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
