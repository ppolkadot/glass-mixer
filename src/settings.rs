//! User preferences, persisted as JSON in `%APPDATA%\GlassMixer\settings.json`.
//!
//! Audio levels are deliberately *not* persisted: Windows stays authoritative for
//! device and session volumes. "Launch at login" is not stored here either; the
//! registry Run key is its single source of truth (see `platform::startup`).

use serde::{Deserialize, Deserializer, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const SETTINGS_VERSION: u32 = 2;
pub const DEFAULT_WIDTH: f32 = 480.0;
pub const DEFAULT_HEIGHT: f32 = 720.0;
pub const MIN_WIDTH: f32 = 360.0;
pub const MIN_HEIGHT: f32 = 440.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    /// Translucent, blurred backdrop (`DWMSBT_TRANSIENTWINDOW`).
    #[default]
    Acrylic,
    /// Subtle wallpaper-tinted backdrop (`DWMSBT_MAINWINDOW`).
    Mica,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Violet,
    Blue,
    Teal,
    Green,
    Amber,
    Coral,
    Pink,
}

impl Accent {
    pub const ALL: [Accent; 7] =
        [Accent::Violet, Accent::Blue, Accent::Teal, Accent::Green, Accent::Amber, Accent::Coral, Accent::Pink];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Violet => "Violet",
            Accent::Blue => "Blue",
            Accent::Teal => "Teal",
            Accent::Green => "Green",
            Accent::Amber => "Amber",
            Accent::Coral => "Coral",
            Accent::Pink => "Pink",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WindowGeometry {
    /// Returns a geometry with a sane size, or `None` if the values are unusable.
    pub fn sanitized(self) -> Option<Self> {
        let finite = [self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite());
        if !finite {
            return None;
        }
        Some(Self { width: self.width.clamp(MIN_WIDTH, 4000.0), height: self.height.clamp(MIN_HEIGHT, 4000.0), ..self })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(deserialize_with = "lenient")]
    pub version: u32,
    #[serde(deserialize_with = "lenient")]
    pub theme: ThemeMode,
    #[serde(deserialize_with = "lenient_true")]
    pub glass: bool,
    #[serde(deserialize_with = "lenient")]
    pub backdrop: Backdrop,
    #[serde(deserialize_with = "lenient")]
    pub accent: Accent,
    #[serde(deserialize_with = "lenient")]
    pub density: Density,
    #[serde(deserialize_with = "lenient")]
    pub start_minimized: bool,
    #[serde(deserialize_with = "lenient_true")]
    pub close_to_tray: bool,
    #[serde(deserialize_with = "lenient")]
    pub minimize_to_tray: bool,
    #[serde(deserialize_with = "lenient")]
    pub show_inactive_apps: bool,
    #[serde(deserialize_with = "lenient")]
    pub show_meters: bool,
    #[serde(deserialize_with = "lenient")]
    pub window: Option<WindowGeometry>,
    /// Version 1 stored egui logical points; version 2 stores screen pixels.
    #[serde(deserialize_with = "lenient")]
    pub window_position_physical: bool,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            theme: ThemeMode::System,
            glass: true,
            backdrop: Backdrop::Acrylic,
            accent: Accent::Violet,
            density: Density::Comfortable,
            start_minimized: false,
            close_to_tray: true,
            minimize_to_tray: false,
            show_inactive_apps: true,
            show_meters: true,
            window: None,
            window_position_physical: false,
            extra: serde_json::Map::new(),
        }
    }
}

/// Deserializes a field, falling back to its default when the stored value is
/// invalid (e.g. an accent name from a newer version), instead of discarding
/// every other preference.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn lenient_true<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or(true))
}

#[derive(Debug)]
pub enum LoadOutcome {
    /// Settings were read successfully.
    Loaded(Settings),
    /// No settings file exists yet (first launch).
    FirstRun(Settings),
    /// The file existed but could not be used; defaults are returned.
    Recovered { settings: Settings, error: String },
}

impl LoadOutcome {
    pub fn settings(&self) -> &Settings {
        match self {
            LoadOutcome::Loaded(s) | LoadOutcome::FirstRun(s) => s,
            LoadOutcome::Recovered { settings, .. } => settings,
        }
    }

    pub fn into_settings(self) -> Settings {
        match self {
            LoadOutcome::Loaded(s) | LoadOutcome::FirstRun(s) => s,
            LoadOutcome::Recovered { settings, .. } => settings,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { path: dir.into().join("settings.json") }
    }

    /// `%APPDATA%\GlassMixer` (roaming, per user).
    pub fn default_dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("GlassMixer"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> LoadOutcome {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return LoadOutcome::FirstRun(Settings::default());
            }
            Err(e) => {
                return LoadOutcome::Recovered {
                    settings: Settings::default(),
                    error: format!("Couldn't read settings ({e}). Using defaults."),
                };
            }
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                s.window = s.window.and_then(WindowGeometry::sanitized);
                s.version = s.version.max(SETTINGS_VERSION);
                LoadOutcome::Loaded(s)
            }
            Err(e) => {
                // Keep the unreadable file for inspection instead of silently overwriting it.
                let backup = self.path.with_extension("json.bad");
                let _ = fs::rename(&self.path, &backup);
                LoadOutcome::Recovered {
                    settings: Settings::default(),
                    error: format!(
                        "Settings file was damaged ({e}); defaults restored. The old file was kept as {}.",
                        backup.display()
                    ),
                }
            }
        }
    }

    /// Writes atomically: a temporary file is written and then renamed over the target.
    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(
            format!("glass-mixer-test-{name}-{}-{:?}", std::process::id(), std::time::SystemTime::now())
                .replace([':', ' ', '{', '}', ','], "_"),
        );
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_enable_glass_and_tray() {
        let s = Settings::default();
        assert!(s.glass, "glass must be enabled by default");
        assert!(s.close_to_tray);
        assert_eq!(s.theme, ThemeMode::System);
        assert_eq!(s.backdrop, Backdrop::Acrylic);
    }

    #[test]
    fn first_run_when_missing() {
        let store = SettingsStore::new(temp_dir("missing"));
        assert!(matches!(store.load(), LoadOutcome::FirstRun(_)));
    }

    #[test]
    fn round_trip() {
        let dir = temp_dir("roundtrip");
        let store = SettingsStore::new(&dir);
        let s = Settings {
            theme: ThemeMode::Light,
            glass: false,
            backdrop: Backdrop::Mica,
            accent: Accent::Teal,
            density: Density::Compact,
            start_minimized: true,
            window: Some(WindowGeometry { x: 10.0, y: 20.0, width: 500.0, height: 700.0 }),
            ..Settings::default()
        };
        store.save(&s).unwrap();
        match store.load() {
            LoadOutcome::Loaded(loaded) => assert_eq!(loaded, s),
            other => panic!("unexpected {other:?}"),
        }
        store.save(&s).expect("updating an existing settings file");
        assert!(!dir.join("settings.json.tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_values_fall_back_per_field() {
        let dir = temp_dir("lenient");
        fs::write(dir.join("settings.json"), r#"{"accent":"neon","glass":false,"theme":"dark","future_option":42}"#)
            .unwrap();
        let s = SettingsStore::new(&dir).load().into_settings();
        assert_eq!(s.accent, Accent::Violet);
        assert!(!s.glass);
        assert_eq!(s.theme, ThemeMode::Dark);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_boolean_preserves_other_fields_and_newer_schema_fields() {
        let dir = temp_dir("future");
        fs::write(dir.join("settings.json"), r#"{"version":99,"glass":"bad","theme":"dark","future_option":{"x":1}}"#)
            .unwrap();
        let store = SettingsStore::new(&dir);
        let s = store.load().into_settings();
        assert_eq!(s.version, 99);
        assert!(s.glass);
        assert_eq!(s.theme, ThemeMode::Dark);
        store.save(&s).unwrap();
        let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
        assert_eq!(value["future_option"]["x"], 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_is_backed_up_and_defaults_used() {
        let dir = temp_dir("corrupt");
        fs::write(dir.join("settings.json"), "{ not json").unwrap();
        let outcome = SettingsStore::new(&dir).load();
        assert!(matches!(outcome, LoadOutcome::Recovered { .. }));
        assert_eq!(outcome.settings(), &Settings::default());
        assert!(dir.join("settings.json.bad").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn window_geometry_is_sanitized() {
        let g = WindowGeometry { x: 0.0, y: 0.0, width: 10.0, height: 99999.0 }.sanitized().unwrap();
        assert_eq!(g.width, MIN_WIDTH);
        assert_eq!(g.height, 4000.0);
        assert!(WindowGeometry { x: f32::NAN, y: 0.0, width: 400.0, height: 400.0 }.sanitized().is_none());
    }
}
