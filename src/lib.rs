//! Glass Mixer — a native Windows audio mixer that lives in the system tray.
//!
//! Module responsibilities:
//! - [`audio`]: Core Audio engine thread, commands and state snapshots.
//! - [`platform`]: Windows integration (tray, backdrop, icons, startup, single instance).
//! - [`ui`]: egui views, widgets and the design system.
//! - [`settings`]: persisted user preferences.
//! - [`app`]: application lifecycle tying everything together.

pub mod app;
pub mod audio;
pub mod icon_art;
pub mod platform;
pub mod settings;
pub mod ui;
