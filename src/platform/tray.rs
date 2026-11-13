//! System tray icon and menu.
//!
//! The tray icon lives on the UI thread (it isn't `Send`). Its event handlers run
//! inside the UI thread's message loop and act immediately: showing the window
//! via Win32 and sending mute commands straight to the audio engine. Exit is
//! forwarded to the app so shutdown goes through one path.

use super::tray_menu::{self, TrayMenu};
use super::window::{self, Hwnd};
use crate::audio::NoticeLevel;
use crate::audio::{AudioHandle, AudioSnapshot, Command, tray_tooltip};
use crate::icon_art::render_app_icon;
use crossbeam_channel::Sender;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::Shell::{
    NIF_GUID, NIF_INFO, NIF_REALTIME, NIIF_ERROR, NIIF_INFO, NIIF_NOSOUND, NIIF_RESPECT_QUIET_TIME, NIIF_WARNING,
    NIM_MODIFY, NOTIFY_ICON_INFOTIP_FLAGS, NOTIFYICONDATAW, QUNS_ACCEPTS_NOTIFICATIONS, SHQueryUserNotificationState,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};

fn tray_guid(path: &str) -> u128 {
    // Windows binds unsigned GUID icons to the executable path. Use an explicit
    // stable hash so installed, portable and development builds do not collide.
    path.to_lowercase().bytes().fold(0x6c62272e07bb014262b821756295c58du128, |hash, byte| {
        (hash ^ byte as u128).wrapping_mul(0x0000000001000000000000000000013b)
    })
}

#[derive(Default)]
struct NoticeGate {
    last: Option<Instant>,
    recent: HashMap<String, Instant>,
}

impl NoticeGate {
    fn allow(&mut self, text: &str, now: Instant) -> bool {
        self.recent.retain(|_, at| now.duration_since(*at) < Duration::from_secs(60));
        if self.last.is_some_and(|at| now.duration_since(at) < Duration::from_secs(10))
            || self.recent.contains_key(text)
        {
            return false;
        }
        self.last = Some(now);
        self.recent.insert(text.into(), now);
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Exit,
}

pub struct Tray {
    icon: TrayIcon,
    menu: TrayMenu,
    normal: Icon,
    muted: Icon,
    last_tooltip: String,
    last_muted: Option<bool>,
    notices: NoticeGate,
    guid: u128,
}

fn make_icon(muted: bool) -> Result<Icon, String> {
    // Render at the small-icon size for the system DPI so the tray icon is crisp.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as u32;
    Icon::from_rgba(render_app_icon(size, muted), size, size).map_err(|e| e.to_string())
}

impl Tray {
    pub fn create(
        hwnd: Hwnd,
        audio: AudioHandle,
        actions: Sender<TrayAction>,
        ctx: egui::Context,
    ) -> Result<Self, String> {
        let menu = TrayMenu::new(hwnd)?;

        let normal = make_icon(false)?;
        let muted = make_icon(true)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let guid = tray_guid(&exe.to_string_lossy());
        let icon = TrayIconBuilder::new()
            .with_id("glass-mixer")
            .with_guid(guid)
            .with_icon(normal.clone())
            .with_tooltip("Glass Mixer")
            .with_menu_on_right_click(false)
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;

        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } = event {
                match button {
                    MouseButton::Left => window::show(hwnd),
                    MouseButton::Right => match TrayMenu::show(hwnd) {
                        tray_menu::OPEN => window::show(hwnd),
                        tray_menu::MUTE => audio.send(Command::ToggleOutputMute),
                        tray_menu::EXIT => {
                            let _ = actions.send(TrayAction::Exit);
                            ctx.request_repaint();
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }));

        Ok(Self {
            icon,
            menu,
            normal,
            muted,
            last_tooltip: String::new(),
            last_muted: None,
            notices: NoticeGate::default(),
            guid,
        })
    }

    /// Shell-managed notifications honor Windows suppression policy. No AUMID,
    /// shortcut registration, activation handler or elevation is needed.
    pub fn notify(&mut self, level: NoticeLevel, text: &str) {
        if level == NoticeLevel::Info {
            return;
        }
        if !accepts_notifications() || !self.notices.allow(text, Instant::now()) {
            return;
        }
        self.balloon(if level == NoticeLevel::Error { NIIF_ERROR } else { NIIF_WARNING }, text);
    }

    /// The result of something the user just did while the window is hidden,
    /// such as switching the output with a shortcut. Not throttled: each one
    /// replaces the previous balloon.
    pub fn announce(&mut self, text: &str) {
        if accepts_notifications() {
            self.balloon(NIIF_INFO, text);
        }
    }

    fn balloon(&mut self, kind: NOTIFY_ICON_INFOTIP_FLAGS, text: &str) {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: windows::Win32::Foundation::HWND(self.icon.window_handle()),
            guidItem: windows::core::GUID::from_u128(self.guid),
            uFlags: NIF_GUID | NIF_INFO | NIF_REALTIME,
            dwInfoFlags: kind | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME,
            ..Default::default()
        };
        write_wide(&mut data.szInfoTitle, "Glass Mixer");
        write_wide(&mut data.szInfo, text);
        if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            log::warn!("Windows could not accept a tray notification");
        }
    }

    /// Reflects the current master state in the tooltip, icon and menu.
    pub fn update(&mut self, snapshot: &AudioSnapshot) {
        let tooltip = tray_tooltip(snapshot);
        if tooltip != self.last_tooltip {
            let _ = self.icon.set_tooltip(Some(&tooltip));
            self.last_tooltip = tooltip;
        }
        let muted = snapshot.output.as_ref().is_some_and(|o| o.muted);
        self.menu.update(muted, snapshot.output.is_some());
        if self.last_muted != Some(muted) {
            let icon = if muted { self.muted.clone() } else { self.normal.clone() };
            let _ = self.icon.set_icon(Some(icon));
            self.last_muted = Some(muted);
        }
    }
}

fn accepts_notifications() -> bool {
    unsafe { SHQueryUserNotificationState() }.ok() == Some(QUNS_ACCEPTS_NOTIFICATIONS)
}

fn write_wide<const N: usize>(buf: &mut [u16; N], text: &str) {
    let mut i = 0;
    for c in text.chars().filter(|c| *c != '\0') {
        let mut units = [0; 2];
        let encoded = c.encode_utf16(&mut units);
        if i + encoded.len() >= N {
            break;
        }
        buf[i..i + encoded.len()].copy_from_slice(encoded);
        i += encoded.len();
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        let _ = self.icon.set_visible(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_identity_separates_install_paths() {
        assert_eq!(tray_guid(r"C:\Apps\Mixer.exe"), tray_guid(r"c:\apps\mixer.exe"));
        assert_ne!(tray_guid(r"C:\Apps\Mixer.exe"), tray_guid(r"C:\Other\Mixer.exe"));
    }
    #[test]
    fn notifications_are_throttled_and_duplicates_expire() {
        let now = Instant::now();
        let mut gate = NoticeGate::default();
        assert!(gate.allow("one", now));
        assert!(!gate.allow("two", now + Duration::from_secs(1)));
        assert!(!gate.allow("one", now + Duration::from_secs(11)));
        assert!(gate.allow("two", now + Duration::from_secs(11)));
        assert!(gate.allow("one", now + Duration::from_secs(61)));
    }
    #[test]
    fn notification_text_does_not_split_surrogates() {
        let mut buf = [0; 3];
        write_wide(&mut buf, "x😀");
        assert_eq!(buf, [b'x' as u16, 0, 0]);
    }
}
