//! System tray icon and menu.
//!
//! The tray icon lives on the UI thread (it isn't `Send`). Its event handlers run
//! inside the UI thread's message loop and act immediately: showing the window
//! via Win32 and sending mute commands straight to the audio engine. Exit is
//! forwarded to the app so shutdown goes through one path.

use super::window::{self, Hwnd};
use crate::audio::{AudioHandle, AudioSnapshot, Command, tray_tooltip};
use crate::icon_art::render_app_icon;
use crossbeam_channel::Sender;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Exit,
}

pub struct Tray {
    icon: TrayIcon,
    mute_item: CheckMenuItem,
    normal: Icon,
    muted: Icon,
    last_tooltip: String,
    last_muted: Option<bool>,
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
        let open = MenuItem::with_id("open", "Open Mixer", true, None);
        let mute_item = CheckMenuItem::with_id("mute", "Mute", true, false, None);
        let exit = MenuItem::with_id("exit", "Exit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &mute_item, &PredefinedMenuItem::separator(), &exit]).map_err(|e| e.to_string())?;

        let normal = make_icon(false)?;
        let muted = make_icon(true)?;
        let icon = TrayIconBuilder::new()
            .with_id("glass-mixer")
            .with_icon(normal.clone())
            .with_tooltip("Glass Mixer")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;

        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::show(hwnd);
            }
        }));

        {
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
                "open" => window::show(hwnd),
                "mute" => audio.send(Command::ToggleOutputMute),
                "exit" => {
                    let _ = actions.send(TrayAction::Exit);
                    ctx.request_repaint();
                }
                _ => {}
            }));
        }

        Ok(Self { icon, mute_item, normal, muted, last_tooltip: String::new(), last_muted: None })
    }

    /// Reflects the current master state in the tooltip, icon and menu.
    pub fn update(&mut self, snapshot: &AudioSnapshot) {
        let tooltip = tray_tooltip(snapshot);
        if tooltip != self.last_tooltip {
            let _ = self.icon.set_tooltip(Some(&tooltip));
            self.last_tooltip = tooltip;
        }
        let muted = snapshot.output.as_ref().is_some_and(|o| o.muted);
        // The check item toggles itself when clicked; always re-sync it to Windows' state.
        if self.mute_item.is_checked() != muted {
            self.mute_item.set_checked(muted);
        }
        self.mute_item.set_enabled(snapshot.output.is_some());
        if self.last_muted != Some(muted) {
            let icon = if muted { self.muted.clone() } else { self.normal.clone() };
            let _ = self.icon.set_icon(Some(icon));
            self.last_muted = Some(muted);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        let _ = self.icon.set_visible(false);
    }
}
