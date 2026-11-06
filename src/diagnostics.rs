//! A diagnostic report for bug reports, made on this PC and sent nowhere.
//!
//! It holds versions, states, counts and recent warnings, never audio or files.
//! App, device and profile names and the user's profile path are replaced before
//! anything is written, so the report can be shared without further editing.

use crate::{
    audio::{AudioSnapshot, NoticeLevel, ServiceStatus},
    platform::{hotkeys::HotkeyStatus, sysprefs::SystemPrefs},
    settings::{SETTINGS_VERSION, Settings},
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// How many saved reports are kept; older ones are deleted when a new one is saved.
pub const KEEP_REPORTS: usize = 10;
/// Warnings and errors remembered for the report.
pub const MAX_NOTICES: usize = 30;
const LOG_LINES: usize = 80;
/// Never replaced: they would turn the placeholders themselves into garbage.
const PLACEHOLDERS: [&str; 4] = ["<app>", "<device>", "<profile>", "%USERPROFILE%"];

static LOG_FILE: OnceLock<PathBuf> = OnceLock::new();

/// Tells the report where the application log is (set once at startup).
pub fn set_log_file(path: PathBuf) {
    let _ = LOG_FILE.set(path);
}

/// A warning or error the user was shown, kept for the report.
#[derive(Clone, Debug, PartialEq)]
pub struct RecentNotice {
    pub level: NoticeLevel,
    pub text: String,
}

/// Everything a report is built from.
pub struct Inputs<'a> {
    pub settings: &'a Settings,
    pub snapshot: &'a AudioSnapshot,
    pub prefs: &'a SystemPrefs,
    /// How the window is drawn, as logged at startup.
    pub renderer: &'a str,
    pub software_renderer: bool,
    /// `None` when the global shortcut service couldn't start.
    pub hotkeys: Option<&'a HotkeyStatus>,
    pub notices: &'a [RecentNotice],
    pub log_tail: &'a [String],
    pub unix_seconds: u64,
}

/// The names to hide, each with its placeholder. Short names are skipped (they
/// would also match inside unrelated words), and longer ones go first so
/// "Zen Browser" is replaced before "Zen".
fn private_names(i: &Inputs) -> Vec<(String, &'static str)> {
    let s = i.settings;
    let mut names: Vec<(String, &'static str)> = Vec::new();
    let mut add = |name: &str, placeholder: &'static str| {
        let usable =
            name.chars().count() >= 3 && !PLACEHOLDERS.iter().any(|p| p.to_lowercase().contains(&name.to_lowercase()));
        if usable && !names.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            names.push((name.to_owned(), placeholder));
        }
    };
    for app in &i.snapshot.apps {
        add(&app.name, "<app>");
    }
    for d in i.snapshot.outputs.iter().chain(&i.snapshot.inputs) {
        add(&d.name, "<device>");
    }
    for endpoint in i.snapshot.output.iter().chain(&i.snapshot.input) {
        add(&endpoint.name, "<device>");
    }
    for pin in &s.pinned_apps {
        add(&pin.name, "<app>");
    }
    for custom in &s.app_names {
        add(&custom.name, "<app>");
    }
    for pin in &s.pinned_devices {
        add(&pin.name, "<device>");
    }
    for rule in &s.focus_rules {
        add(&rule.name, "<app>");
    }
    for profile in &s.presets {
        add(&profile.name, "<profile>");
        for entry in &profile.entries {
            add(&entry.name, "<app>");
            if let Some(device) = &entry.output {
                add(&device.name, "<device>");
            }
        }
        for device in profile.output_device.iter().chain(&profile.input_device) {
            add(&device.name, "<device>");
        }
    }
    for binding in &s.hotkeys {
        match &binding.target {
            Some(crate::platform::hotkeys::HotkeyTarget::App { name, .. }) => add(name, "<app>"),
            Some(crate::platform::hotkeys::HotkeyTarget::Profile { name }) => add(name, "<profile>"),
            None => {}
        }
    }
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.chars().count()));
    names
}

/// Replaces every occurrence of `needle`, ignoring ASCII case.
fn replace_ignoring_case(text: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return text.to_owned();
    }
    // ASCII lowercasing keeps every byte offset, so positions carry over.
    let (haystack, needle) = (text.to_ascii_lowercase(), needle.to_ascii_lowercase());
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(found) = haystack[from..].find(&needle) {
        let at = from + found;
        out.push_str(&text[from..at]);
        out.push_str(with);
        from = at + needle.len();
    }
    out.push_str(&text[from..]);
    out
}

/// Hides `names` and the profile folder, in the form it has in plain text and in
/// the escaped form Rust's `Debug` output gives backslashes in the log.
fn redact(text: &str, names: &[(String, &'static str)], profile: Option<&str>) -> String {
    let mut out = text.to_owned();
    if let Some(profile) = profile.filter(|p| p.len() >= 3) {
        out = replace_ignoring_case(&out, &profile.replace('\\', "\\\\"), "%USERPROFILE%");
        out = replace_ignoring_case(&out, profile, "%USERPROFILE%");
    }
    for (name, placeholder) in names {
        out = replace_ignoring_case(&out, name, placeholder);
    }
    out
}

/// UTC date and time from Unix seconds (proleptic Gregorian calendar).
pub fn utc(unix_seconds: u64) -> (u64, u64, u64, u64, u64, u64) {
    let (days, secs) = (unix_seconds / 86_400, unix_seconds % 86_400);
    // Civil-from-days, after Howard Hinnant.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u64;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u64;
    let year = (year_of_era + era * 400 + i64::from(month <= 2)) as u64;
    (year, month, day, secs / 3600, secs % 3600 / 60, secs % 60)
}

pub fn file_name(unix_seconds: u64) -> String {
    let (y, mo, d, h, mi, s) = utc(unix_seconds);
    format!("glass-mixer-diagnostics-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}.txt")
}

fn yes(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// The report text.
pub fn build(i: &Inputs) -> String {
    let profile = std::env::var("USERPROFILE").ok();
    build_with_profile(i, profile.as_deref())
}

fn build_with_profile(i: &Inputs, profile: Option<&str>) -> String {
    let names = private_names(i);
    let hide = |text: &str| redact(text, &names, profile);
    let (s, snap, prefs) = (i.settings, i.snapshot, i.prefs);
    let (y, mo, d, h, mi, sec) = utc(i.unix_seconds);
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push_str("\r\n");
    };

    line("Glass Mixer diagnostic report".into());
    line(format!("Generated (UTC): {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{sec:02}"));
    line("Made on this PC and not sent anywhere. App, device and profile names and your profile folder are".into());
    line("replaced with <app>, <device>, <profile> and %USERPROFILE%. Audio and files are never included.".into());

    line(String::new());
    line("== Application ==".into());
    line(format!("Version: {}", env!("CARGO_PKG_VERSION")));
    line(format!("Build: {}", if cfg!(debug_assertions) { "debug" } else { "release" }));
    line(format!("Settings schema version: {} (current {SETTINGS_VERSION})", s.version));
    line(format!("Renderer: {}", hide(i.renderer)));
    line(format!("Software rendering: {}", yes(i.software_renderer)));

    line(String::new());
    line("== Windows ==".into());
    line(format!("Build: {}", prefs.build));
    line(format!(
        "Dark apps: {}; transparency effects: {}; animation effects: {}; high contrast: {}",
        yes(prefs.dark),
        yes(prefs.transparency),
        yes(prefs.animations),
        yes(prefs.high_contrast)
    ));

    line(String::new());
    line("== Audio ==".into());
    line(match &snap.status {
        ServiceStatus::Starting => "Service: starting".into(),
        ServiceStatus::Ok => "Service: running".into(),
        ServiceStatus::Unavailable(why) => format!("Service: unavailable ({})", hide(why)),
    });
    line(format!(
        "Outputs: {}; inputs: {}; default output present: {}; default input present: {}",
        snap.outputs.len(),
        snap.inputs.len(),
        yes(snap.output.is_some()),
        yes(snap.input.is_some())
    ));
    line(format!(
        "Default device switching: {}; per-app devices: {}",
        yes(snap.can_switch_default),
        yes(snap.can_route_apps)
    ));
    line(format!(
        "App rows: {}; audio sessions: {}",
        snap.apps.len(),
        snap.apps.iter().map(|a| a.session_count).sum::<usize>()
    ));
    line(format!(
        "Solo: {}; microphone ducking active: {}; apps lowered in the background: {}",
        yes(snap.solo.is_some()),
        yes(snap.mic_ducking),
        snap.background_lowered.len()
    ));
    let device_errors: Vec<&String> = snap.device_states.iter().filter_map(|d| d.state.as_ref().err()).collect();
    if device_errors.is_empty() {
        line("Device read errors: none".into());
    } else {
        line(format!("Device read errors: {}", device_errors.len()));
        for e in device_errors {
            line(format!("  - {}", hide(e)));
        }
    }

    line(String::new());
    line("== Shortcuts ==".into());
    match i.hotkeys {
        None => line("Service: unavailable".into()),
        Some(status) => {
            line(format!("Active: {}; not active: {}", status.active.len(), status.failed.len()));
            for f in &status.failed {
                line(format!("  - {:?} ({}): {}", f.action, f.chord, hide(&f.reason)));
            }
            for e in &status.errors {
                line(format!("  - {}", hide(e)));
            }
        }
    }

    line(String::new());
    line("== Settings ==".into());
    line(format!(
        "Theme: {:?}; typeface: {:?}; glass: {} ({:?}); accent: {:?}; density: {:?}",
        s.theme,
        s.typeface,
        yes(s.glass),
        s.backdrop,
        s.accent,
        s.density
    ));
    line(format!(
        "Start minimized: {}; keep running when closed: {}; minimize to tray: {}; level meters: {}",
        yes(s.start_minimized),
        yes(s.close_to_tray),
        yes(s.minimize_to_tray),
        yes(s.show_meters)
    ));
    line(format!("Volume step: {}%; mouse wheel step: {}%", s.hotkey_step, s.wheel_step));
    line(format!(
        "Pinned apps: {}; renamed apps: {}; pinned outputs: {}; profiles: {} ({} with an automatic rule); shortcuts: {}",
        s.pinned_apps.len(),
        s.app_names.len(),
        s.pinned_devices.len(),
        s.presets.len(),
        s.presets.iter().filter(|p| p.apply_on_start).count(),
        s.hotkeys.len()
    ));
    line(format!(
        "Background lowering rules: {}; microphone ducking: {} ({} apps)",
        s.focus_rules.len(),
        if s.mic_ducking.enabled { "on" } else { "off" },
        s.mic_ducking.targets.len()
    ));

    line(String::new());
    line(format!("== Recent warnings and errors (up to {MAX_NOTICES}) =="));
    if i.notices.is_empty() {
        line("none".into());
    }
    for n in i.notices {
        let level = match n.level {
            NoticeLevel::Info => "info",
            NoticeLevel::Warning => "warning",
            NoticeLevel::Error => "error",
        };
        line(format!("[{level}] {}", hide(&n.text)));
    }

    line(String::new());
    line(format!("== Log (last {LOG_LINES} lines) =="));
    if i.log_tail.is_empty() {
        line("no log available".into());
    }
    for l in i.log_tail {
        line(hide(l));
    }
    out
}

/// The last `max` lines of the application log; empty if there is none.
pub fn read_log_tail(max: usize) -> Vec<String> {
    LOG_FILE.get().map(|path| tail_of_file(path, max)).unwrap_or_default()
}

pub fn log_lines() -> usize {
    LOG_LINES
}

fn tail_of_file(path: &Path, max: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else { return Vec::new() };
    // The tail of a log is within its last few hundred KB.
    let len = file.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(256 * 1024);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // a partial first line
    }
    lines[lines.len().saturating_sub(max)..].iter().map(|l| (*l).to_owned()).collect()
}

/// `diagnostics` beside the log (`%LOCALAPPDATA%\GlassMixer\diagnostics`), so a
/// development profile keeps its reports with its own logs.
pub fn reports_dir() -> Option<PathBuf> {
    let beside_log = LOG_FILE.get().and_then(|log| log.parent()).map(|dir| dir.join("diagnostics"));
    beside_log
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("GlassMixer").join("diagnostics")))
}

/// Writes `report` into `dir` and removes all but the newest [`KEEP_REPORTS`].
pub fn save(report: &str, dir: &Path, unix_seconds: u64) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(file_name(unix_seconds));
    std::fs::write(&path, report)?;
    let mut reports: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("glass-mixer-diagnostics-") && n.ends_with(".txt"))
        })
        .collect();
    // Names sort by date because the timestamp is in them.
    reports.sort();
    let excess = reports.len().saturating_sub(KEEP_REPORTS);
    for old in &reports[..excess] {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AppEntry, AppKey, DeviceInfo, DeviceState};

    fn prefs() -> SystemPrefs {
        SystemPrefs { dark: true, transparency: true, animations: false, high_contrast: false, build: 26200 }
    }

    fn app(name: &str) -> AppEntry {
        crate::audio::preferences::PinnedApp {
            key: AppKey::Exe(format!(r"c:\users\someone\{name}.exe")),
            name: name.into(),
            exe_path: None,
            aumid: None,
        }
        .placeholder()
    }

    #[test]
    fn utc_dates_are_correct_across_known_instants() {
        assert_eq!(utc(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(utc(1_000_000_000), (2001, 9, 9, 1, 46, 40));
        assert_eq!(utc(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(utc(951_782_400), (2000, 2, 29, 0, 0, 0), "a leap day in a century year divisible by 400");
        assert_eq!(utc(1_709_251_199), (2024, 2, 29, 23, 59, 59));
        assert_eq!(utc(1_709_251_200), (2024, 3, 1, 0, 0, 0));
        assert_eq!(file_name(1_700_000_000), "glass-mixer-diagnostics-20231114-221320.txt");
    }

    #[test]
    fn redaction_hides_names_paths_and_ignores_case_longest_first() {
        let names = vec![
            ("Zen Browser".to_owned(), "<app>"),
            ("Zen".to_owned(), "<app>"),
            ("Fones de ouvido".to_owned(), "<device>"),
        ];
        let profile = r"C:\Users\Someone";
        assert_eq!(redact("zen browser and Zen", &names, Some(profile)), "<app> and <app>");
        assert_eq!(redact("FONES DE OUVIDO ok", &names, None), "<device> ok", "case-insensitive for ASCII");
        assert_eq!(redact(r"C:\Users\Someone\AppData\x", &names, Some(profile)), r"%USERPROFILE%\AppData\x");
        assert_eq!(
            redact(r#"key=Exe("c:\\users\\someone\\zen\\zen.exe")"#, &names, Some(profile)),
            r#"key=Exe("%USERPROFILE%\\<app>\\<app>.exe")"#,
            "the escaped form in the log is hidden as well"
        );
        assert_eq!(redact("nothing here", &names, Some(profile)), "nothing here");
    }

    #[test]
    fn short_and_placeholder_like_names_are_never_replaced() {
        let settings = Settings::default();
        let snapshot =
            AudioSnapshot { apps: vec![app("ab"), app("App"), app("Device"), app("Real Game")], ..Default::default() };
        let notices = [];
        let i = Inputs {
            settings: &settings,
            snapshot: &snapshot,
            prefs: &prefs(),
            renderer: "OpenGL",
            software_renderer: false,
            hotkeys: None,
            notices: &notices,
            log_tail: &[],
            unix_seconds: 0,
        };
        let names: Vec<_> = private_names(&i).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Real Game"], "two-letter names and names inside the placeholders are left alone");
    }

    #[test]
    fn the_report_has_the_facts_and_none_of_the_private_names() {
        let mut settings = Settings::default();
        settings.pinned_apps.push(
            crate::audio::preferences::PinnedApp::from_app(&AppEntry {
                key: AppKey::Exe(r"c:\games\secretgame.exe".into()),
                ..app("SecretGame")
            })
            .unwrap(),
        );
        settings
            .presets
            .push(crate::audio::preferences::VolumePreset { name: "Night Shift".into(), ..Default::default() });
        settings.app_names.push(crate::audio::preferences::AppName {
            key: AppKey::Exe(r"c:\games\secretgame.exe".into()),
            name: "Dad's Secret Game".into(),
        });
        let device = |id: &str, name: &str| DeviceInfo { id: id.into(), name: name.into(), is_default: false };
        let snapshot = AudioSnapshot {
            status: ServiceStatus::Ok,
            apps: vec![app("SecretGame"), app("Chatty")],
            outputs: vec![device("a", "My Studio Headset")],
            device_states: vec![DeviceState {
                flow: crate::audio::Flow::Output,
                info: device("a", "My Studio Headset"),
                state: Err("My Studio Headset: Device unavailable: 0x88890004".into()),
            }],
            can_route_apps: true,
            ..Default::default()
        };
        let hotkeys = HotkeyStatus {
            failed: vec![crate::platform::hotkeys::FailedBinding {
                action: crate::platform::hotkeys::HotkeyAction::AppMute,
                target: Some(crate::platform::hotkeys::HotkeyTarget::App {
                    key: AppKey::Exe(r"c:\games\secretgame.exe".into()),
                    name: "SecretGame".into(),
                }),
                chord: "Ctrl+Alt+1".into(),
                reason: "Windows could not register this shortcut".into(),
            }],
            ..Default::default()
        };
        let notices =
            [RecentNotice { level: NoticeLevel::Error, text: "Couldn't change the device of SecretGame".into() }];
        let log = vec![
            r#"12:00:00 [INFO] profile Night Shift applied from C:\Users\Someone\AppData"#.to_owned(),
            "12:00:01 [INFO] Dad's Secret Game started".to_owned(),
        ];
        let i = Inputs {
            settings: &settings,
            snapshot: &snapshot,
            prefs: &prefs(),
            renderer: "wgpu fallback, Dx12 on \"Microsoft Basic Render Driver\" (Cpu)",
            software_renderer: true,
            hotkeys: Some(&hotkeys),
            notices: &notices,
            log_tail: &log,
            unix_seconds: 1_700_000_000,
        };
        let report = build_with_profile(&i, Some(r"C:\Users\Someone"));

        for expected in [
            "Generated (UTC): 2023-11-14 22:13:20",
            "Version: ",
            "Settings schema version: 4 (current 4)",
            "Software rendering: yes",
            "Build: 26200",
            "Service: running",
            "Outputs: 1; inputs: 0",
            "per-app devices: yes",
            "App rows: 2",
            "Pinned apps: 1; renamed apps: 1; pinned outputs: 0; profiles: 1",
            "AppMute (Ctrl+Alt+1): Windows could not register this shortcut",
            "[error] Couldn't change the device of <app>",
            "Device read errors: 1",
            "<device>: Device unavailable: 0x88890004",
            "profile <profile> applied from %USERPROFILE%\\AppData",
        ] {
            assert!(report.contains(expected), "missing {expected:?} in:\n{report}");
        }
        for private in ["SecretGame", "Chatty", "My Studio Headset", "Night Shift", "Someone", "Dad's"] {
            assert!(!report.to_lowercase().contains(&private.to_lowercase()), "{private} leaked into:\n{report}");
        }
        assert!(report.contains("\r\n") && !report.contains("\r\r"), "Windows line endings, once");
    }

    #[test]
    fn an_unavailable_service_and_no_shortcut_service_are_reported_not_hidden() {
        let (settings, snapshot) = (
            Settings::default(),
            AudioSnapshot {
                status: ServiceStatus::Unavailable("The audio service is not running".into()),
                ..Default::default()
            },
        );
        let i = Inputs {
            settings: &settings,
            snapshot: &snapshot,
            prefs: &prefs(),
            renderer: "OpenGL (glow)",
            software_renderer: false,
            hotkeys: None,
            notices: &[],
            log_tail: &[],
            unix_seconds: 0,
        };
        let report = build_with_profile(&i, None);
        assert!(report.contains("Service: unavailable (The audio service is not running)"));
        assert!(report.contains("== Shortcuts ==\r\nService: unavailable"));
        assert!(
            report.contains("no log available")
                && report.contains("== Recent warnings and errors (up to 30) ==\r\nnone")
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glass-mixer-diag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_log_tail_is_the_last_lines_and_a_missing_log_is_empty() {
        let dir = temp_dir("tail");
        let log = dir.join("glass-mixer.log");
        assert!(tail_of_file(&log, 5).is_empty());
        let lines: Vec<String> = (0..300).map(|n| format!("line {n}")).collect();
        std::fs::write(&log, lines.join("\n")).unwrap();
        let tail = tail_of_file(&log, 3);
        assert_eq!(tail, ["line 297", "line 298", "line 299"]);
        assert_eq!(tail_of_file(&log, 1000).len(), 300);
        // A big log is read from its end without a partial first line.
        let big: Vec<String> = (0..60_000).map(|n| format!("2026-11-06 12:00:00 [INFO] entry number {n}")).collect();
        std::fs::write(&log, big.join("\n")).unwrap();
        let tail = tail_of_file(&log, 2);
        assert_eq!(
            tail,
            ["2026-11-06 12:00:00 [INFO] entry number 59998", "2026-11-06 12:00:00 [INFO] entry number 59999"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_keeps_only_the_newest_reports_and_leaves_other_files_alone() {
        let dir = temp_dir("save");
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        let mut last = PathBuf::new();
        for n in 0..(KEEP_REPORTS as u64 + 4) {
            last = save("report", &dir, 1_700_000_000 + n * 60).unwrap();
        }
        let mut kept: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("glass-mixer-diagnostics-"))
            .collect();
        kept.sort();
        assert_eq!(kept.len(), KEEP_REPORTS);
        assert_eq!(dir.join(kept.last().unwrap()), last, "the newest is kept");
        assert!(dir.join("notes.txt").exists(), "files that aren't reports are never deleted");
        assert_eq!(std::fs::read_to_string(&last).unwrap(), "report");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
