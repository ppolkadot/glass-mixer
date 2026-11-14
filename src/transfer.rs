//! Exporting and importing settings.
//!
//! An export is versioned JSON of the preferences, without anything specific to
//! this PC (where the window sits). An import is checked before anything changes:
//! the file must be a Glass Mixer export (or a plain `settings.json`), from this
//! version or an older one, and what can't be read is reported instead of being
//! quietly replaced with defaults. The caller keeps a copy of the current settings
//! before replacing them.

use crate::{
    diagnostics,
    settings::{SETTINGS_VERSION, Settings, decode_text},
};
use serde_json::Value;
use std::{
    io,
    io::Write,
    path::{Path, PathBuf},
};

/// Version of the export wrapper, separate from the settings' own version.
pub const EXPORT_FORMAT: u64 = 1;
const MAX_IMPORT_BYTES: usize = 1024 * 1024;
const TOO_LARGE: &str = "The file is too large to be a Glass Mixer export (over 1 MB).";
/// Where this window sits belongs to this PC: it is neither exported nor imported.
const MACHINE_FIELDS: [&str; 2] = ["window", "window_position_physical"];
/// Automatic copies made before an import replaces the settings; older ones are deleted.
pub const KEEP_BACKUPS: usize = 5;

pub const EXPORT_PREFIX: &str = "glass-mixer-settings";
pub const BACKUP_PREFIX: &str = "glass-mixer-before-import";

pub fn export(settings: &Settings, app_version: &str) -> String {
    let mut value = serde_json::to_value(settings).expect("settings are serializable");
    if let Value::Object(map) = &mut value {
        for field in MACHINE_FIELDS {
            map.remove(field);
        }
        map.insert("version".into(), SETTINGS_VERSION.into());
    }
    let document = serde_json::json!({
        "glass_mixer_export": EXPORT_FORMAT,
        "app_version": app_version,
        "settings": value,
    });
    serde_json::to_string_pretty(&document).expect("an export is serializable")
}

/// A checked import: the settings it holds, and what couldn't be read.
#[derive(Debug)]
pub struct Import {
    pub settings: Settings,
    pub notes: Vec<String>,
}

/// Checks and reads an export, or a plain settings file. The error says why a
/// file was refused.
pub fn import(bytes: &[u8]) -> Result<Import, String> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(TOO_LARGE.into());
    }
    let text = decode_text(bytes).map_err(|e| format!("The file can't be read as text: {e}."))?;
    let document: Value = serde_json::from_str(&text).map_err(|e| format!("The file isn't valid JSON ({e})."))?;
    let Value::Object(mut root) = document else {
        return Err("This isn't a Glass Mixer export.".into());
    };
    let settings_value = match root.get("glass_mixer_export") {
        Some(format) => {
            let format = format.as_u64().ok_or("The export's format number isn't valid.")?;
            if format > EXPORT_FORMAT {
                return Err(format!(
                    "This export was made by a newer Glass Mixer (format {format}). Update Glass Mixer to import it."
                ));
            }
            root.remove("settings").ok_or("The export has no settings in it.")?
        }
        // A plain settings.json, or one of its backups, is accepted as well.
        None if root.get("version").is_some_and(Value::is_number) => Value::Object(root),
        None => return Err("This isn't a Glass Mixer export or settings file.".into()),
    };
    let Value::Object(input) = &settings_value else {
        return Err("The export's settings aren't valid.".into());
    };
    let version = input.get("version").and_then(Value::as_u64).ok_or("The settings have no version number.")?;
    if version > u64::from(SETTINGS_VERSION) {
        return Err(format!(
            "These settings are from a newer Glass Mixer (version {version}). Update Glass Mixer to import them."
        ));
    }
    let mut settings: Settings =
        serde_json::from_value(settings_value.clone()).map_err(|e| format!("The settings can't be read: {e}."))?;
    let kept = serde_json::to_value(&settings).expect("settings are serializable");
    let mut notes = Vec::new();
    for (field, wanted) in input {
        if field == "version" || MACHINE_FIELDS.contains(&field.as_str()) {
            continue;
        }
        let Some(read) = kept.get(field) else { continue }; // not a setting this version knows
        import_notes(field, wanted, read, &mut notes);
    }
    settings.version = SETTINGS_VERSION;
    settings.window = None;
    settings.window_position_physical = false;
    Ok(Import { settings, notes })
}

/// Reads and checks the export at `path`. A file that is missing, isn't a file or
/// is too large is refused without reading it all.
pub fn read_import_file(path: &Path) -> Result<Import, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("Couldn't open the file: {e}."))?;
    if !meta.is_file() {
        return Err("That path isn't a file.".into());
    }
    if meta.len() > MAX_IMPORT_BYTES as u64 {
        return Err(TOO_LARGE.into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read the file: {e}."))?;
    import(&bytes)
}

/// Reports discarded or adjusted values at every nesting level, comparing only
/// fields the file mentions (an omitted field gets its default).
fn import_notes(path: &str, wanted: &Value, read: &Value, notes: &mut Vec<String>) {
    match (wanted, read) {
        (Value::Object(wanted), Value::Object(read)) => {
            for (key, value) in wanted {
                import_notes(&format!("{path}.{key}"), value, read.get(key).unwrap_or(&Value::Null), notes);
            }
        }
        (Value::Array(wanted), Value::Array(read)) if wanted.len() != read.len() => {
            notes.push(format!("{path}: {} of {} entries could be read.", read.len(), wanted.len()));
        }
        (Value::Array(wanted), Value::Array(read)) => {
            for (index, (wanted, read)) in wanted.iter().zip(read).enumerate() {
                import_notes(&format!("{path}[{index}]"), wanted, read, notes);
            }
        }
        (Value::Number(a), Value::Number(b))
            if a.as_f64().zip(b.as_f64()).is_some_and(|(x, y)| (x - y).abs() <= 1e-6) => {}
        _ if wanted != read => notes.push(format!("{path}: the value isn't valid, so it was adjusted or omitted.")),
        _ => {}
    }
}

/// `current` with everything from `imported` except what belongs to this PC.
pub fn merge_keeping_this_pc(current: &Settings, mut imported: Settings) -> Settings {
    imported.window = current.window;
    imported.window_position_physical = current.window_position_physical;
    imported
}

pub fn summary(s: &Settings) -> Vec<String> {
    vec![
        format!(
            "{} pinned app(s), {} renamed app(s), {} pinned output(s)",
            s.pinned_apps.len(),
            s.app_names.len(),
            s.pinned_devices.len()
        ),
        format!(
            "{} profile(s), {} shortcut(s), {} background-lowering rule(s)",
            s.presets.len(),
            s.hotkeys.len(),
            s.focus_rules.len()
        ),
        format!(
            "Theme {:?}, typeface {:?}, microphone ducking {}",
            s.theme,
            s.typeface,
            if s.mic_ducking.enabled { "on" } else { "off" }
        ),
    ]
}

/// `exports` beside the settings file.
pub fn exports_dir(settings_path: &Path) -> PathBuf {
    settings_path.parent().map_or_else(|| PathBuf::from("exports"), |dir| dir.join("exports"))
}

fn file_name(prefix: &str, unix_seconds: u64) -> String {
    let (y, mo, d, h, mi, s) = diagnostics::utc(unix_seconds);
    format!("{prefix}-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}.json")
}

/// Writes `text` as `<prefix>-<date>.json` in `dir`. Automatic backups keep only
/// the newest [`KEEP_BACKUPS`]; exports the user asked for are never deleted.
pub fn save(dir: &Path, prefix: &str, unix_seconds: u64, text: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = file_name(prefix, unix_seconds);
    let stem = name.trim_end_matches(".json");
    // Pruning can remove the original unsuffixed name. Continue after the
    // surviving suffixes so a new backup is never sorted as the oldest one.
    let mut suffix = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let existing = entry.file_name();
        let Some(tail) = existing.to_str().and_then(|n| n.strip_suffix(".json")).and_then(|n| n.strip_prefix(stem))
        else {
            continue;
        };
        let index = if tail.is_empty() { Some(0) } else { tail.strip_prefix('-').and_then(|n| n.parse::<u64>().ok()) };
        if let Some(index) = index {
            suffix = suffix.max(
                index
                    .checked_add(1)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "No backup filename available"))?,
            );
        }
    }
    let (path, mut file) = loop {
        let name = if suffix == 0 { name.clone() } else { format!("{stem}-{suffix:020}.json") };
        let path = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break (path, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => suffix = suffix.checked_add(1).ok_or(e)?,
            Err(e) => return Err(e),
        }
    };
    if let Err(e) = file.write_all(text.as_bytes()).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    drop(file);
    if prefix == BACKUP_PREFIX {
        let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p != &path && p.is_file())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(BACKUP_PREFIX) && n.ends_with(".json"))
            })
            .collect();
        backups.sort_by_key(|p| p.file_stem().map(|s| s.to_owned()));
        // The backup just written always survives; keep the newest older ones beside it.
        let excess = backups.len().saturating_sub(KEEP_BACKUPS.saturating_sub(1));
        for old in &backups[..excess] {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{
        AppKey,
        preferences::{AppName, PinnedApp, PinnedDevice, VolumePreset},
    };
    use crate::settings::{Accent, ThemeMode, Typeface, WindowGeometry};

    fn sample() -> Settings {
        let mut s = Settings {
            theme: ThemeMode::Light,
            accent: Accent::Coral,
            typeface: Typeface::System,
            ..Default::default()
        };
        s.pinned_apps.push(PinnedApp {
            key: AppKey::Exe(r"c:\apps\a.exe".into()),
            name: "A".into(),
            exe_path: Some(r"c:\apps\a.exe".into()),
            aumid: None,
        });
        s.app_names.push(AppName { key: AppKey::Exe(r"c:\apps\a.exe".into()), name: "Music".into() });
        s.pinned_devices.push(PinnedDevice { id: "{out}".into(), name: "Headset".into() });
        s.presets.push(VolumePreset { name: "Night".into(), ..Default::default() });
        s.wheel_step = 5;
        s.window = Some(WindowGeometry { x: 10.0, y: 20.0, width: 480.0, height: 720.0 });
        s.window_position_physical = true;
        s
    }

    #[test]
    fn an_export_is_versioned_and_holds_nothing_about_this_pc() {
        let text = export(&sample(), "9.9.9");
        let doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["glass_mixer_export"], 1);
        assert_eq!(doc["app_version"], "9.9.9");
        assert_eq!(doc["settings"]["version"], SETTINGS_VERSION);
        assert!(doc["settings"].get("window").is_none() && doc["settings"].get("window_position_physical").is_none());
        assert_eq!(doc["settings"]["wheel_step"], 5);
    }

    #[test]
    fn an_exported_file_imports_back_to_the_same_preferences_without_notes() {
        let original = sample();
        let imported = import(export(&original, "1.0").as_bytes()).expect("own export is accepted");
        assert!(imported.notes.is_empty(), "{:?}", imported.notes);
        let merged = merge_keeping_this_pc(&Settings { window: None, ..Default::default() }, imported.settings);
        assert_eq!(merged, Settings { window: None, window_position_physical: false, ..original });
    }

    #[test]
    fn a_plain_settings_file_and_other_text_encodings_are_accepted() {
        let plain = serde_json::to_string(&sample()).unwrap();
        assert_eq!(import(plain.as_bytes()).unwrap().settings.wheel_step, 5);
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(plain.as_bytes());
        assert_eq!(import(&bom).unwrap().settings.wheel_step, 5);
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(plain.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(import(&utf16).unwrap().settings.wheel_step, 5);
    }

    #[test]
    fn files_that_are_not_valid_exports_are_refused_with_a_reason() {
        let refused = |bytes: &[u8]| import(bytes).expect_err("must be refused");
        assert!(refused(b"").contains("isn't valid JSON"));
        assert!(refused(b"not json at all").contains("isn't valid JSON"));
        assert!(refused(b"[1,2,3]").contains("isn't a Glass Mixer export"));
        assert!(refused(br#"{"theme":"dark"}"#).contains("isn't a Glass Mixer export or settings file"));
        assert!(refused(br#"{"glass_mixer_export":1}"#).contains("no settings"));
        assert!(refused(br#"{"glass_mixer_export":1,"settings":[]}"#).contains("settings aren't valid"));
        assert!(refused(br#"{"glass_mixer_export":1,"settings":{"theme":"dark"}}"#).contains("no version"));
        assert!(refused(br#"{"glass_mixer_export":"one","settings":{}}"#).contains("format number"));
        assert!(refused(&[0xC3, 0x28, 0xA0]).contains("can't be read as text"));
        assert!(refused(&vec![b' '; MAX_IMPORT_BYTES + 1]).contains("too large"));
    }

    #[test]
    fn exports_and_settings_from_a_newer_version_are_refused_not_downgraded() {
        let newer_format = format!(r#"{{"glass_mixer_export":{},"settings":{{"version":4}}}}"#, EXPORT_FORMAT + 1);
        assert!(import(newer_format.as_bytes()).unwrap_err().contains("newer Glass Mixer (format 2)"));
        let newer_settings = format!(r#"{{"glass_mixer_export":1,"settings":{{"version":{}}}}}"#, SETTINGS_VERSION + 1);
        assert!(import(newer_settings.as_bytes()).unwrap_err().contains("newer Glass Mixer (version 5)"));
    }

    #[test]
    fn what_cannot_be_read_is_reported_not_silently_replaced() {
        let text = serde_json::json!({
            "glass_mixer_export": 1,
            "settings": {
                "version": 4,
                "wheel_step": 99,
                "typeface": "comic",
                "theme": "dark",
                "pinned_apps": [
                    {"key": {"exe": r"c:\apps\a.exe"}, "name": "A", "exe_path": null, "aumid": null},
                    "garbage",
                    {"key": {"exe": r"c:\apps\b.exe"}, "name": "B", "exe_path": null, "aumid": null}
                ],
                "solo": {"keep_percent": "loud"},
                "window": {"x": 1}
            }
        })
        .to_string();
        let imported = import(text.as_bytes()).unwrap();
        let notes = imported.notes.join("\n");
        assert!(notes.contains("wheel_step: the value isn't valid"), "{notes}");
        assert!(notes.contains("typeface: the value isn't valid"), "{notes}");
        assert!(notes.contains("pinned_apps: 2 of 3 entries could be read"), "{notes}");
        assert!(notes.contains("solo.keep_percent: the value isn't valid"), "{notes}");
        assert!(
            !notes.contains("theme") && !notes.contains("window"),
            "valid and machine fields aren't flagged: {notes}"
        );
        assert_eq!(imported.settings.theme, ThemeMode::Dark, "what is valid is kept");
        assert_eq!(imported.settings.pinned_apps.len(), 2);
        assert_eq!(imported.settings.window, None, "the window isn't imported");
    }

    #[test]
    fn an_import_keeps_this_pcs_window_and_nothing_else_of_the_current_settings() {
        let current = sample();
        let imported =
            import(export(&Settings { wheel_step: 3, ..Default::default() }, "1.0").as_bytes()).unwrap().settings;
        let merged = merge_keeping_this_pc(&current, imported);
        assert_eq!(merged.window, current.window);
        assert!(merged.window_position_physical);
        assert_eq!(merged.wheel_step, 3, "the imported preference wins");
        assert!(merged.pinned_apps.is_empty(), "current pins don't survive an import");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glass-mixer-transfer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn only_the_automatic_backups_are_pruned() {
        let dir = temp_dir("save");
        for n in 0..(KEEP_BACKUPS as u64 + 3) {
            save(&dir, BACKUP_PREFIX, 1_700_000_000 + n * 60, "{}").unwrap();
            save(&dir, EXPORT_PREFIX, 1_700_000_000 + n * 60, "{}").unwrap();
        }
        let count = |prefix: &str| {
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n.starts_with(prefix))
                .count()
        };
        assert_eq!(count(BACKUP_PREFIX), KEEP_BACKUPS, "the newest automatic backups are kept");
        assert_eq!(count(EXPORT_PREFIX), KEEP_BACKUPS + 3, "exports the user asked for are never deleted");
        assert_eq!(file_name(EXPORT_PREFIX, 1_700_000_000), "glass-mixer-settings-20231114-221320.json");
        assert_eq!(
            exports_dir(Path::new(r"C:\Users\x\AppData\Roaming\GlassMixer\settings.json")),
            PathBuf::from(r"C:\Users\x\AppData\Roaming\GlassMixer\exports")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reading_a_file_refuses_missing_paths_folders_and_huge_files_before_reading_them() {
        let dir = temp_dir("read");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_import_file(&dir.join("missing.json")).unwrap_err().contains("Couldn't open the file"));
        assert_eq!(read_import_file(&dir).unwrap_err(), "That path isn't a file.");
        let big = dir.join("big.json");
        std::fs::write(&big, vec![b' '; MAX_IMPORT_BYTES + 1]).unwrap();
        assert_eq!(read_import_file(&big).unwrap_err(), TOO_LARGE);
        let good = dir.join("good.json");
        std::fs::write(&good, export(&sample(), "1.0")).unwrap();
        assert_eq!(read_import_file(&good).unwrap().settings.wheel_step, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_summary_counts_what_an_import_holds() {
        let lines = summary(&sample());
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("1 pinned app(s), 1 renamed app(s), 1 pinned output(s)"), "{lines:?}");
        assert!(lines[1].starts_with("1 profile(s)"), "{lines:?}");
    }

    #[test]
    fn saves_in_the_same_second_preserve_each_backup_and_export() {
        let dir = temp_dir("collisions");
        for prefix in [BACKUP_PREFIX, EXPORT_PREFIX] {
            let a = save(&dir, prefix, 1_700_000_000, "first").unwrap();
            let b = save(&dir, prefix, 1_700_000_000, "second").unwrap();
            assert_ne!(a, b);
            assert_eq!(std::fs::read_to_string(a).unwrap(), "first");
            assert_eq!(std::fs::read_to_string(b).unwrap(), "second");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imports_report_invalid_nested_profile_parts_even_when_the_profile_survives() {
        let imported = import(
            br#"{"version":4,"presets":[{"name":"Night","entries":[],"master":{"volume":"loud","muted":false}}]}"#,
        )
        .unwrap();
        assert_eq!(imported.settings.presets.len(), 1);
        assert!(imported.settings.presets[0].master.is_none());
        assert!(imported.notes.iter().any(|note| note.contains("presets[0].master")), "{:?}", imported.notes);
    }

    #[test]
    fn pruning_same_second_backups_never_removes_the_just_saved_backup() {
        let dir = temp_dir("collision-retention");
        let mut saved = Vec::new();
        for n in 0..KEEP_BACKUPS + 4 {
            let path = save(&dir, BACKUP_PREFIX, 1_700_000_000, &n.to_string()).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), n.to_string());
            saved.push(path);
        }
        for (n, path) in saved.iter().enumerate() {
            if n < saved.len() - KEEP_BACKUPS {
                assert!(!path.exists());
            } else {
                assert_eq!(std::fs::read_to_string(path).unwrap(), n.to_string());
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retention_preserves_the_current_backup_after_a_clock_rollback() {
        let dir = temp_dir("clock-rollback");
        for n in 0..KEEP_BACKUPS {
            save(&dir, BACKUP_PREFIX, 1_700_000_100 + n as u64, "older content").unwrap();
        }
        let current = save(&dir, BACKUP_PREFIX, 1_700_000_099, "current content").unwrap();
        assert_eq!(std::fs::read_to_string(&current).unwrap(), "current content");
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(files, KEEP_BACKUPS);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retention_ignores_directories_that_match_the_backup_pattern() {
        let dir = temp_dir("backup-directory");
        let decoy = dir.join("glass-mixer-before-import-19990101-000000.json");
        std::fs::create_dir_all(&decoy).unwrap();
        for n in 0..=KEEP_BACKUPS as u64 {
            save(&dir, BACKUP_PREFIX, 1_700_000_000 + n, "older content").unwrap();
        }
        assert!(decoy.is_dir(), "a directory is not a backup file");
        let files = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).filter(|e| e.path().is_file()).count();
        assert_eq!(files, KEEP_BACKUPS, "the directory does not consume a file-retention slot");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn imports_plain_json_floats_without_false_precision_notes() {
        let imported =
            import(br#"{"version":4,"presets":[{"name":"Work","entries":[],"master":{"volume":0.4,"muted":false}}]}"#)
                .unwrap();
        assert!(imported.notes.is_empty(), "unexpected notes: {:?}", imported.notes);
    }
}
