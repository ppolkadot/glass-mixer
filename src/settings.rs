//! User preferences, persisted as JSON in `%APPDATA%\GlassMixer\settings.json`.
//!
//! Audio levels are persisted only in user-saved presets. Windows stays authoritative
//! for live device and session volumes. "Launch at login" is not stored here either; the
//! registry Run key is its single source of truth (see `platform::startup`).

use serde::{Deserialize, Deserializer, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const SETTINGS_VERSION: u32 = 4;
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

/// Whether `new` differs enough from the saved geometry to be worth saving.
/// A geometry saved in the old logical-point format is always replaced once.
pub fn geometry_changed(saved: Option<WindowGeometry>, saved_is_physical: bool, new: WindowGeometry) -> bool {
    !saved_is_physical
        || saved.is_none_or(|old| {
            (old.x - new.x).abs() > 0.5
                || (old.y - new.y).abs() > 0.5
                || (old.width - new.width).abs() > 0.5
                || (old.height - new.height).abs() > 0.5
        })
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
    #[serde(deserialize_with = "lenient_true")]
    pub hidden_notifications: bool,
    #[serde(deserialize_with = "read_pins")]
    pub pinned_apps: Vec<crate::audio::preferences::PinnedApp>,
    #[serde(deserialize_with = "read_presets")]
    pub presets: Vec<crate::audio::preferences::VolumePreset>,
    #[serde(deserialize_with = "read_hotkeys")]
    pub hotkeys: Vec<crate::platform::hotkeys::HotkeyBinding>,
    #[serde(deserialize_with = "lenient_step")]
    pub hotkey_step: u8,
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
            hidden_notifications: true,
            pinned_apps: Vec::new(),
            presets: Vec::new(),
            hotkeys: Vec::new(),
            hotkey_step: 5,
            window: None,
            window_position_physical: false,
            extra: serde_json::Map::new(),
        }
    }
}

/// Deserializes a field, falling back to its default when the stored value is
/// invalid, so one bad value doesn't discard every other preference.
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

fn list_items<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect())
}

fn read_pins<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<crate::audio::preferences::PinnedApp>, D::Error> {
    Ok(crate::audio::preferences::normalize_pins(list_items(d)?))
}

fn read_presets<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<crate::audio::preferences::VolumePreset>, D::Error> {
    Ok(list_items::<_, crate::audio::preferences::VolumePreset>(d)?.into_iter().filter_map(|p| p.sanitize()).collect())
}

fn read_hotkeys<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<crate::platform::hotkeys::HotkeyBinding>, D::Error> {
    list_items(d)
}

fn lenient_step<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<u8>(value).ok().filter(|v| (1..=20).contains(v)).unwrap_or(5))
}

/// Decodes a settings file: UTF-8 with or without a byte-order mark, or UTF-16
/// with one, as Notepad and PowerShell write when the file is edited by hand.
fn decode_text(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let (units, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return Err("the UTF-16 text is truncated".to_owned());
        }
        String::from_utf16(&units.iter().map(|pair| unit(*pair)).collect::<Vec<_>>())
            .map_err(|_| "the file is not valid UTF-16 text".to_owned())
    };
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8(rest.to_vec()).map_err(|_| "the file is not valid UTF-8 text".to_owned())
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, u16::from_le_bytes)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, u16::from_be_bytes)
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|_| "the file is not UTF-8 or UTF-16 text".to_owned())
    }
}

/// Decodes and parses a settings file.
fn parse_settings(bytes: &[u8]) -> Result<Settings, String> {
    serde_json::from_str::<Settings>(&decode_text(bytes)?).map_err(|e| e.to_string())
}

#[derive(Debug)]
pub enum LoadOutcome {
    /// Settings were read successfully.
    Loaded(Settings),
    /// Settings were migrated from an older version, but the untouched backup file could not be written.
    MigratedWithWarning { settings: Settings, warning: String },
    /// No settings file exists yet (first launch).
    FirstRun(Settings),
    /// The file existed but could not be used; defaults are returned.
    Recovered { settings: Settings, error: String },
}

impl LoadOutcome {
    pub fn settings(&self) -> &Settings {
        match self {
            LoadOutcome::Loaded(s) | LoadOutcome::FirstRun(s) => s,
            LoadOutcome::MigratedWithWarning { settings, .. } | LoadOutcome::Recovered { settings, .. } => settings,
        }
    }

    pub fn warning(&self) -> Option<&str> {
        match self {
            LoadOutcome::MigratedWithWarning { warning, .. } => Some(warning),
            LoadOutcome::Recovered { error, .. } => Some(error),
            LoadOutcome::Loaded(_) | LoadOutcome::FirstRun(_) => None,
        }
    }

    pub fn into_settings(self) -> Settings {
        match self {
            LoadOutcome::Loaded(s) | LoadOutcome::FirstRun(s) => s,
            LoadOutcome::MigratedWithWarning { settings, .. } | LoadOutcome::Recovered { settings, .. } => settings,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
    /// Shared by renderer/startup clones: fallback defaults must never replace
    /// preferences that this store failed to read. A successful reload clears it.
    load_failed: Arc<AtomicBool>,
    #[cfg(test)]
    fail_backup_commit: bool,
}

impl SettingsStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            path: dir.into().join("settings.json"),
            load_failed: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_backup_commit: false,
        }
    }

    /// `%APPDATA%\GlassMixer` (roaming, per user).
    pub fn default_dir() -> Option<PathBuf> {
        #[cfg(debug_assertions)]
        if let Some(dir) = std::env::var_os("GLASS_MIXER_DEV_PROFILE").map(PathBuf::from)
            && dir.is_absolute()
        {
            return Some(dir);
        }
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("GlassMixer"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the untouched copy of an older-format file is kept, e.g.
    /// `settings.v1.backup.json` for a file written by Glass Mixer 0.1.
    pub fn migration_backup_path(&self, version: u32) -> PathBuf {
        self.path.with_file_name(format!("settings.v{version}.backup.json"))
    }

    fn is_valid_backup_file(path: &Path, expected_version: u32) -> bool {
        let Ok(meta) = fs::metadata(path) else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        parse_settings(&bytes).is_ok_and(|s| s.version == expected_version)
    }

    /// Copies an older-format file byte for byte before it is rewritten in the
    /// current format, so it can be restored when going back to an older release.
    /// An existing valid backup is never overwritten.
    fn back_up_before_migration(&self, version: u32, original: &[u8]) -> io::Result<()> {
        let backup = self.migration_backup_path(version);
        if let Ok(meta) = fs::metadata(&backup) {
            if !meta.is_file() {
                return Err(io::Error::other(format!(
                    "backup path {} exists and is not a regular file",
                    backup.display()
                )));
            }
            if Self::is_valid_backup_file(&backup, version) {
                return Ok(());
            }
            log::warn!(
                "existing backup {} is incomplete or invalid for version {version}; replacing it",
                backup.display()
            );
        }
        if let Some(dir) = backup.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = backup.with_extension("json.tmp");
        let write_result = (|| -> io::Result<()> {
            let mut file = fs::File::create(&tmp)?;
            #[cfg(test)]
            if self.fail_backup_commit {
                let partial = &original[..original.len().min(8)];
                io::Write::write_all(&mut file, partial)?;
                file.sync_all()?;
                return Err(io::Error::other("simulated backup commit failure after partial write"));
            }
            io::Write::write_all(&mut file, original)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &backup)
        })();
        if let Err(err) = write_result {
            let _ = fs::remove_file(&tmp);
            log::warn!("couldn't back up version {version} settings before migration: {err}");
            return Err(err);
        }
        log::info!("settings version {version} backed up to {} before migration", backup.display());
        Ok(())
    }

    /// Preserve damaged input before replacing it, without removing the source.
    fn back_up_damaged_file(&self, original: &[u8]) -> io::Result<()> {
        let backup = self.path.with_extension("json.bad");
        if fs::read(&backup).is_ok_and(|saved| saved == original) {
            return Ok(());
        }
        let tmp = self.path.with_extension("json.bad.tmp");
        let result = (|| -> io::Result<()> {
            let mut file = fs::File::create(&tmp)?;
            io::Write::write_all(&mut file, original)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &backup)?;
            if fs::read(&backup)? != original {
                return Err(io::Error::other("the damaged settings backup did not match the original bytes"));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    pub fn load(&self) -> LoadOutcome {
        let bytes = match fs::read(&self.path) {
            Ok(b) => {
                self.load_failed.store(false, Ordering::Relaxed);
                b
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.load_failed.store(false, Ordering::Relaxed);
                return LoadOutcome::FirstRun(Settings::default());
            }
            Err(e) => {
                self.load_failed.store(true, Ordering::Relaxed);
                log::warn!("couldn't read settings {}: {e}; using defaults", self.path.display());
                return LoadOutcome::Recovered {
                    settings: Settings::default(),
                    error: format!(
                        "Couldn't read settings ({e}). Using defaults for this session. Saving is disabled until settings can be loaded successfully; restart Glass Mixer after resolving the problem."
                    ),
                };
            }
        };
        match parse_settings(&bytes) {
            Ok(mut s) => {
                s.window = s.window.and_then(WindowGeometry::sanitized);
                if s.version < SETTINGS_VERSION {
                    let old_version = s.version;
                    match self.back_up_before_migration(old_version, &bytes) {
                        Ok(()) => {
                            s.version = SETTINGS_VERSION;
                            LoadOutcome::Loaded(s)
                        }
                        Err(e) => LoadOutcome::MigratedWithWarning {
                            settings: s,
                            warning: format!(
                                "Preferences were loaded, but the backup copy ({}) could not be created ({e}). The original settings file will not be overwritten until the backup succeeds.",
                                self.migration_backup_path(old_version).display()
                            ),
                        },
                    }
                } else {
                    LoadOutcome::Loaded(s)
                }
            }
            Err(e) => {
                let backup = self.path.with_extension("json.bad");
                let preservation = match self.back_up_damaged_file(&bytes) {
                    Ok(()) => format!("The old file was kept as {}.", backup.display()),
                    Err(error) => format!(
                        "The backup copy ({}) could not be created ({error}). The original file is unchanged and will not be overwritten until the backup succeeds.",
                        backup.display()
                    ),
                };
                let error = format!("Settings file was damaged ({e}); using defaults. {preservation}");
                log::warn!("{error}");
                LoadOutcome::Recovered { settings: Settings::default(), error }
            }
        }
    }

    /// Writes atomically through a synced temporary file. An older-format file is
    /// only replaced once its migration backup exists.
    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        if self.load_failed.load(Ordering::Relaxed) {
            return Err(io::Error::other(
                "refusing to replace settings after a failed load; reload them successfully before saving",
            ));
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let existing = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            // Locked or access denied: replacing it could destroy preferences that were never loaded.
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("the existing settings file couldn't be read, so it was not replaced ({e})"),
                ));
            }
        };
        if let Some(existing_bytes) = &existing {
            match parse_settings(existing_bytes) {
                Ok(existing_settings) if existing_settings.version < SETTINGS_VERSION => {
                    self.back_up_before_migration(existing_settings.version, existing_bytes)?;
                }
                Ok(_) => {}
                Err(_) => self.back_up_damaged_file(existing_bytes)?,
            }
        }
        if settings.version < SETTINGS_VERSION
            && !Self::is_valid_backup_file(&self.migration_backup_path(settings.version), settings.version)
        {
            return Err(io::Error::other(format!(
                "refusing to overwrite settings before version {} backup ({}) is created",
                settings.version,
                self.migration_backup_path(settings.version).display()
            )));
        }
        let mut to_save = settings.clone();
        if to_save.version < SETTINGS_VERSION {
            to_save.version = SETTINGS_VERSION;
        }
        let json = serde_json::to_string_pretty(&to_save).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        let write_result = (|| -> io::Result<()> {
            let mut file = fs::File::create(&tmp)?;
            io::Write::write_all(&mut file, json.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &self.path)
        })();
        if let Err(err) = write_result {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_is_saved_only_for_real_changes_or_legacy_files() {
        let saved = WindowGeometry { x: 100.0, y: 80.0, width: 480.0, height: 720.0 };
        assert!(geometry_changed(None, true, saved), "nothing saved yet");
        assert!(!geometry_changed(Some(saved), true, saved), "identical geometry");
        let nudged = WindowGeometry { x: 100.4, width: 480.3, ..saved };
        assert!(!geometry_changed(Some(saved), true, nudged), "sub-pixel jitter is not saved");
        for moved in [
            WindowGeometry { x: 101.0, ..saved },
            WindowGeometry { y: 79.0, ..saved },
            WindowGeometry { width: 500.0, ..saved },
            WindowGeometry { height: 700.0, ..saved },
        ] {
            assert!(geometry_changed(Some(saved), true, moved), "{moved:?} should be saved");
        }
        assert!(geometry_changed(Some(saved), false, saved), "a version-1 (logical point) file is rewritten once");
    }

    #[test]
    fn saved_pins_presets_and_shortcuts_parse_independently() {
        let text = serde_json::json!({
            "version": 4,
            "pinned_apps": [
                {"key":{"exe":"C:\\Apps\\A.EXE"},"name":"A","exe_path":null,"aumid":null},
                {"key":{"pid":10},"name":"transient","exe_path":null,"aumid":null},
                {"key":{"exe":"c:\\apps\\a.exe"},"name":"duplicate","exe_path":null,"aumid":null},
                "invalid"
            ],
            "presets": [
                {"name":"One","entries":[{"key":"system","name":"System sounds","volume":0.4,"muted":false}]},
                {"name":"Partial","entries":[
                    {"key":{"exe":"C:\\Apps\\B.EXE"},"name":"B","volume":0.6,"muted":true},
                    {"key":"system","name":"bad","volume":"oops","muted":false}
                ],"apply_on_start":"oops"}
            ],
            "hotkeys": [
                {"action":"VolumeUp","chord":"Ctrl+Alt+F23"},
                {"action":"VolumeDown","chord":7}
            ],
            "hotkey_step": 99,
            "future": {"known":true}
        })
        .to_string();
        let settings: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(settings.pinned_apps.len(), 1);
        assert_eq!(settings.presets.len(), 2);
        assert_eq!(settings.hotkeys.len(), 1);
        assert_eq!(settings.hotkey_step, 5);
        assert!(settings.extra.contains_key("future"));
        assert_eq!(settings.presets[0].entries[0].volume, 0.4);
        assert_eq!(settings.presets[1].name, "Partial");
        assert_eq!(settings.presets[1].entries.len(), 1);
        assert_eq!(settings.presets[1].entries[0].volume, 0.6);
        assert!(!settings.presets[1].apply_on_start);
    }

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

    /// A settings file as written by Glass Mixer 0.1.0.
    const V1_FILE: &str = r#"{
  "version": 1,
  "theme": "dark",
  "glass": true,
  "backdrop": "acrylic",
  "accent": "coral",
  "density": "compact",
  "start_minimized": false,
  "close_to_tray": true,
  "minimize_to_tray": false,
  "show_inactive_apps": true,
  "show_meters": true,
  "window": { "x": 465.0, "y": 145.0, "width": 511.0, "height": 640.0 }
}"#;

    #[test]
    fn an_older_format_file_is_backed_up_untouched_before_migration() {
        let dir = temp_dir("migrate");
        let store = SettingsStore::new(&dir);
        fs::write(store.path(), V1_FILE).unwrap();
        let loaded = store.load().into_settings();
        assert_eq!(loaded.version, SETTINGS_VERSION);
        assert_eq!((loaded.theme, loaded.accent, loaded.density), (ThemeMode::Dark, Accent::Coral, Density::Compact));
        assert!(!loaded.window_position_physical, "version 1 stored logical points");
        let backup = store.migration_backup_path(1);
        assert_eq!(backup.file_name().unwrap(), "settings.v1.backup.json");
        assert_eq!(fs::read_to_string(&backup).unwrap(), V1_FILE, "the backup is the original text");

        // Saving in the new format leaves the backup alone, and a later load of an
        // old file never replaces the first backup.
        store.save(&loaded).unwrap();
        assert!(fs::read_to_string(store.path()).unwrap().contains("\"version\": 4"));
        fs::write(store.path(), V1_FILE.replace("coral", "blue")).unwrap();
        store.load();
        assert_eq!(fs::read_to_string(&backup).unwrap(), V1_FILE, "the first backup is kept");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn current_and_newer_files_are_not_backed_up() {
        for (name, text) in
            [("current", r#"{"version":4,"theme":"dark"}"#), ("newer", r#"{"version":9,"theme":"dark"}"#)]
        {
            let dir = temp_dir(name);
            let store = SettingsStore::new(&dir);
            fs::write(store.path(), text).unwrap();
            let outcome = store.load();
            assert!(outcome.warning().is_none());
            let loaded = outcome.into_settings();
            store.save(&loaded).unwrap();
            let backups: Vec<_> = fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().contains("backup"))
                .collect();
            assert!(backups.is_empty(), "{name}: unexpected backup {backups:?}");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn backup_failure_prevents_overwriting_original_until_resolved() {
        let dir = temp_dir("backup-fail");
        let store = SettingsStore::new(&dir);
        fs::write(store.path(), V1_FILE).unwrap();
        let backup = store.migration_backup_path(1);
        // 1. A directory at the backup path makes creating the backup file fail.
        fs::create_dir_all(&backup).unwrap();
        let outcome = store.load();
        let warning = outcome.warning().expect("backup failure must surface a warning");
        assert!(warning.contains("settings.v1.backup.json"), "unexpected warning: {warning}");
        let loaded = outcome.into_settings();
        assert_eq!(loaded.version, 1, "schema must not promote before backup is preserved");
        assert_eq!((loaded.theme, loaded.accent, loaded.density), (ThemeMode::Dark, Accent::Coral, Density::Compact));

        // Saving while the backup path is still obstructed must fail and leave the v1 source untouched.
        assert!(store.save(&loaded).is_err(), "save must fail while backup is obstructed by a directory");
        assert_eq!(fs::read_to_string(store.path()).unwrap(), V1_FILE, "original v1 file must remain untouched");

        // 2. Remove the directory obstruction, but simulate a partial write/commit failure.
        fs::remove_dir_all(&backup).unwrap();
        let failing_store = SettingsStore { fail_backup_commit: true, ..store.clone() };
        assert!(failing_store.load().warning().is_some());
        assert!(failing_store.save(&loaded).is_err(), "simulated partial backup commit failure must block save");
        assert!(!backup.exists(), "failed partial backup commit must not leave a backup file");
        assert!(!backup.with_extension("json.tmp").exists(), "temporary backup file must be cleaned up");
        assert_eq!(fs::read_to_string(store.path()).unwrap(), V1_FILE, "original v1 file must remain untouched");

        // 3. An incomplete/corrupt existing regular file at the backup path is not accepted as a valid backup,
        // and once writing is unobstructed, retry replaces it with a complete backup and completes migration.
        fs::write(&backup, "{\"version\":1,").unwrap();
        assert!(!SettingsStore::is_valid_backup_file(&backup, 1));
        store.save(&loaded).expect("retry after obstruction removal should back up and save");
        assert_eq!(fs::read_to_string(&backup).unwrap(), V1_FILE, "complete backup must be created on retry");
        assert!(fs::read_to_string(store.path()).unwrap().contains("\"version\": 4"));
        let _ = fs::remove_dir_all(dir);

        // 4. Leniently recovered version-0 file (e.g. malformed `"version": "bad"`) also blocks overwrite
        // while `settings.v0.backup.json` is obstructed, and retries cleanly once the obstruction is removed.
        let v0_dir = temp_dir("backup-v0-fail");
        let v0_store = SettingsStore::new(&v0_dir);
        let v0_text = r#"{"version":"bad","theme":"dark"}"#;
        fs::write(v0_store.path(), v0_text).unwrap();
        let v0_backup = v0_store.migration_backup_path(0);
        fs::create_dir_all(&v0_backup).unwrap();
        let v0_outcome = v0_store.load();
        assert!(v0_outcome.warning().is_some(), "version-0 backup failure must warn");
        let v0_loaded = v0_outcome.into_settings();
        assert_eq!(v0_loaded.version, 0);
        assert_eq!(v0_loaded.theme, ThemeMode::Dark);
        assert!(v0_store.save(&v0_loaded).is_err(), "save must fail while settings.v0.backup.json is a directory");
        assert_eq!(fs::read_to_string(v0_store.path()).unwrap(), v0_text, "source must remain untouched");
        fs::remove_dir_all(&v0_backup).unwrap();
        v0_store.save(&v0_loaded).expect("removing settings.v0.backup.json obstruction must let save retry");
        assert_eq!(fs::read_to_string(&v0_backup).unwrap(), v0_text, "v0 backup must preserve original text");
        assert!(SettingsStore::is_valid_backup_file(&v0_backup, 0));
        assert!(fs::read_to_string(v0_store.path()).unwrap().contains("\"version\": 4"));
        let _ = fs::remove_dir_all(v0_dir);
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

    fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
        let mut bytes = if little_endian { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
        for unit in text.encode_utf16() {
            bytes.extend(if little_endian { unit.to_le_bytes() } else { unit.to_be_bytes() });
        }
        bytes
    }

    #[test]
    fn hand_edited_files_with_a_byte_order_mark_or_utf16_load_normally() {
        let text = r#"{"version":4,"theme":"dark","accent":"teal","show_meters":false}"#;
        let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
        utf8_bom.extend_from_slice(text.as_bytes());
        for (name, bytes) in [("utf8-bom", utf8_bom), ("utf16le", utf16(text, true)), ("utf16be", utf16(text, false))] {
            let dir = temp_dir(name);
            let store = SettingsStore::new(&dir);
            fs::write(store.path(), &bytes).unwrap();
            let outcome = store.load();
            assert!(matches!(outcome, LoadOutcome::Loaded(_)), "{name}: {outcome:?}");
            let s = outcome.into_settings();
            assert_eq!((s.theme, s.accent, s.show_meters), (ThemeMode::Dark, Accent::Teal, false), "{name}");
            assert!(!dir.join("settings.json.bad").exists(), "{name}: a valid file must not be set aside");
            // The next save rewrites it as plain UTF-8 that still loads.
            store.save(&s).unwrap();
            assert!(fs::read_to_string(store.path()).unwrap().starts_with('{'), "{name}");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn undecodable_bytes_are_kept_aside_like_damaged_json() {
        let dir = temp_dir("bad-encoding");
        let store = SettingsStore::new(&dir);
        let garbage = [0x7B, 0xC3, 0x28, 0xFF, 0x7D];
        fs::write(store.path(), garbage).unwrap();
        let outcome = store.load();
        assert!(matches!(outcome, LoadOutcome::Recovered { .. }), "{outcome:?}");
        assert_eq!(fs::read(dir.join("settings.json.bad")).unwrap(), garbage, "the original bytes are kept");
        let truncated_utf16 = [0xFF, 0xFE, 0x7B];
        fs::write(store.path(), truncated_utf16).unwrap();
        assert!(matches!(store.load(), LoadOutcome::Recovered { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_migration_backup_keeps_the_exact_bytes_of_a_bom_file() {
        let dir = temp_dir("migrate-bom");
        let store = SettingsStore::new(&dir);
        let mut original = vec![0xEF, 0xBB, 0xBF];
        original.extend_from_slice(V1_FILE.as_bytes());
        fs::write(store.path(), &original).unwrap();
        let loaded = store.load().into_settings();
        assert_eq!((loaded.version, loaded.accent), (SETTINGS_VERSION, Accent::Coral));
        let backup = store.migration_backup_path(1);
        assert_eq!(fs::read(&backup).unwrap(), original, "the backup is byte-for-byte the original");
        assert!(SettingsStore::is_valid_backup_file(&backup, 1), "a BOM backup is still recognised as valid");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_existing_file_that_cannot_be_read_is_never_replaced() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_DELETE: u32 = 0x4;
        let dir = temp_dir("unreadable");
        let store = SettingsStore::new(&dir);
        let original = r#"{"version":4,"theme":"dark","accent":"pink"}"#;
        fs::write(store.path(), original).unwrap();
        // Held open without read sharing: reading fails, but a rename over it would succeed.
        let lock = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_DELETE).open(store.path()).unwrap();
        let outcome = store.load();
        assert!(matches!(outcome, LoadOutcome::Recovered { .. }), "{outcome:?}");
        assert!(store.save(&outcome.into_settings()).is_err(), "save must refuse to replace an unreadable file");
        drop(lock);
        assert_eq!(fs::read_to_string(store.path()).unwrap(), original, "the unread original must survive");
        assert!(!dir.join("settings.json.tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_transient_load_failure_blocks_saves_until_a_successful_reload() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = temp_dir("transient-read-failure");
        let store = SettingsStore::new(&dir);
        let original = br#"{"version":4,"theme":"dark","accent":"pink","show_meters":false}"#;
        fs::write(store.path(), original).unwrap();
        let lock = fs::OpenOptions::new().read(true).share_mode(4).open(store.path()).unwrap();
        let outcome = store.load();
        assert!(matches!(outcome, LoadOutcome::Recovered { .. }));
        let mut defaults = outcome.into_settings();
        // Geometry changes can cause a save without any settings interaction.
        defaults.window = Some(WindowGeometry { x: 40.0, y: 50.0, width: 480.0, height: 720.0 });
        drop(lock);
        assert!(store.clone().save(&defaults).is_err(), "a clone must retain failed-load protection after unlocking");
        assert_eq!(fs::read(store.path()).unwrap(), original);
        assert!(!dir.join("settings.json.tmp").exists());

        let mut recovered = store.load().into_settings();
        assert_eq!((recovered.theme, recovered.accent), (ThemeMode::Dark, Accent::Pink));
        recovered.show_meters = true;
        store.save(&recovered).expect("successful reload permits intentional changes");
        assert!(store.load().settings().show_meters);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_damaged_file_backup_blocks_overwrite_and_retries_safely() {
        use std::os::windows::fs::OpenOptionsExt;
        for locked in [false, true] {
            let dir = temp_dir(if locked { "bad-backup-locked" } else { "bad-backup-directory" });
            let store = SettingsStore::new(&dir);
            let original = [0x7b, 0xc3, 0x28, 0xff, 0x7d];
            fs::write(store.path(), original).unwrap();
            let backup = dir.join("settings.json.bad");
            let obstruction = if locked {
                fs::write(&backup, b"previous damaged contents").unwrap();
                Some(fs::OpenOptions::new().read(true).share_mode(0).open(&backup).unwrap())
            } else {
                fs::create_dir(&backup).unwrap();
                None
            };
            let outcome = store.load();
            let warning = outcome.warning().unwrap();
            assert!(!warning.contains("old file was kept"), "must not claim a failed backup succeeded: {warning}");
            let defaults = outcome.into_settings();
            assert!(store.save(&defaults).is_err(), "save must refuse while backup is obstructed");
            assert_eq!(fs::read(store.path()).unwrap(), original);
            drop(obstruction);
            if locked {
                fs::remove_file(&backup).unwrap();
            } else {
                fs::remove_dir(&backup).unwrap();
            }
            store.save(&defaults).expect("retry preserves the original before saving defaults");
            assert_eq!(fs::read(&backup).unwrap(), original);
            assert!(matches!(store.load(), LoadOutcome::Loaded(_)));
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn window_geometry_is_sanitized() {
        let g = WindowGeometry { x: 0.0, y: 0.0, width: 10.0, height: 99999.0 }.sanitized().unwrap();
        assert_eq!(g.width, MIN_WIDTH);
        assert_eq!(g.height, 4000.0);
        assert!(WindowGeometry { x: f32::NAN, y: 0.0, width: 400.0, height: 400.0 }.sanitized().is_none());
    }
}
