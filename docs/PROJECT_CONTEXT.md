# Glass Mixer — Project Context

The whole project in one document. It records what was asked, what was
built and why, how it was checked, what is known to be missing or risky,
and the context needed to continue the work.

Last updated: 2026-10-15, after Phase 2 on branch
`codex/glass-mixer-features`. The v0.1.0 release record remains below.

- Repository: https://github.com/ppolkadot/glass-mixer (public)
- Release: https://github.com/ppolkadot/glass-mixer/releases/tag/v0.1.0

Related documents:

- [README.md](../README.md): user-facing features, install, build,
  limitations and troubleshooting.
- [ARCHITECTURE.md](ARCHITECTURE.md): design and the Windows API strategy,
  in short form.
- [TESTING.md](TESTING.md): automated tests, recorded results and the
  manual checklist.

---

## 1. Summary

Glass Mixer is a native Windows system-tray audio mixer written in Rust.
Through the Windows Core Audio APIs it controls:

- The default output and input devices: volume, mute and default-device
  switching.
- Every app with an audio session: volume, mute and balance, shown with
  the app's own icon and status.

Its UI is built on eframe/egui (OpenGL renderer) and uses a Windows 11
Acrylic/Mica glass backdrop, which is on by default.

Status: **v0.1.0 is functional and released.** The core requirements are
implemented and checked on one Windows 11 machine. Windows 10, microphones,
switching between several outputs, and sleep/resume have **not** been tested
(no suitable hardware or OS was available). Section 10 lists the remaining
gaps and section 11 the risks.

| Item | Value |
| --- | --- |
| Executable | `glass-mixer.exe`, 8.05 MB (release, stripped, thin LTO) |
| Installer | `glass-mixer-0.1.0-setup.exe`, 4.83 MB (Inno Setup, per user, no admin) |
| Portable | `glass-mixer-0.1.0-portable.zip`, 3.46 MB |
| Source | about 5,900 lines of Rust in 27 `src/` files, plus build code, examples, tests, docs and scripts |
| License | MIT (copyright 2026 ppolkadot) |
| Toolchain | Rust 1.99.0 stable MSVC, Rust edition 2024, MSRV 1.95 |

---

## 2. The request

The project began from a detailed specification pasted into the session:
"Project Specification: Native Windows Audio Mixer". It is reproduced in
full in [Appendix A](#appendix-a--original-specification-verbatim). In short:

- A real, lightweight, native Windows tray application in Rust using
  MMDevice, `IAudioEndpointVolume`, `IAudioSessionManager2` and
  `ISimpleAudioVolume`. It must not be a mockup or a web app.
- Master and device volume and mute, device switching, and a per-app mixer
  with icons, activity status, automatic discovery and removal, grouping
  by executable or package identity, balance, a filter, and refresh.
- Tray behaviour: icon, menu (Open Mixer / Mute / Exit), close to tray,
  start minimized, launch at login, a single instance and a tooltip.
- A minimal, colourful Windows 11-style design. The glass effect must be
  on by default, use native Mica/Acrylic, and fall back to an opaque
  surface where unsupported.
- Light, dark and system themes, accent colours, density, and persisted
  preferences.
- An event-driven, reliable engine: correct COM handling, no drift from
  Windows, feedback-loop prevention, recovery, low idle cost.
- Accessibility, error handling, and an honest testing record.
- Deliverables: source, docs, tests, a manual checklist, release build,
  installer and README.
- The final instruction was to present the architecture first, then
  implement it in increments and keep going until it's done.

Follow-up requests in the same session:

1. "Yes, install Inno Setup and build the installer, then create a GitHub
   repository and upload the project." The repo was initially private by the
   owner's choice and MIT-licensed by the owner's choice. It is public now.
2. "Create a doc that summarizes everything about the project …, add it to
   the repo and commit it." That is this file.

An earlier, unrelated task in the same session fixed malformed JSON (a
trailing comma) in the user's Claude Code `~/.claude/settings.json`. It
isn't part of the app.

---

## 3. Development history

1. **Environment check.** The machine had no Rust and no MSVC linker.
   With the owner's approval, rustup (stable MSVC) and Visual Studio 2022
   Build Tools (C++ workload, Windows SDK 10.0.26100) were installed with
   winget.
2. **Architecture proposal.** The architecture, libraries, API strategy and
   limitations were presented before any code was written, as the spec asks.
3. **Glass spike** (`examples/glass_spike.rs`). This proved that a
   transparent eframe/glow window plus `DwmExtendFrameIntoClientArea(-1)`
   plus `DWMWA_SYSTEMBACKDROP_TYPE` renders a real DWM backdrop behind
   egui.
   - With the backdrop off, the window was fully see-through.
   - Acrylic and Mica each produced a visibly different backdrop.
   - The first, ambiguous screenshots were explained by the spike's own
     console window sitting behind it.
   - Screen capture was stopped once it risked capturing the owner's
     other windows.
4. **API verification.** Exact signatures were read from the crate sources
   (windows-rs 0.62, eframe 0.36, tray-icon 0.26) before writing code.
   - The key discovery: eframe 0.36's `App::logic()` still runs while the
     window is hidden, which shaped the tray design.
5. **Phase 1–2: audio.** Settings, the audio model, the engine, callbacks,
   `IPolicyConfig`, process identity and exit watching, and power events.
   - The library compiled on the first try.
   - `examples/audio_probe.rs` checked it against real hardware: it found
     the HDMI output, the System sounds session and Zen Browser (playing).
6. **Phase 3: platform.** Window helpers, single instance, launch at
   login, backdrop, system preferences.
   - **The session was interrupted at this point.** It resumed from the
     project files, then the icon resolver, fonts and tray were added.
7. **Phase 4: UI.** Design tokens, widgets, the mixer view, the settings
   view, the app lifecycle and `main.rs`.
   - Only two compile errors: egui 0.36 renamed its scroll input, and
     `show_inside` was deprecated.
8. **Verification and fixes.** Unit tests, real-device integration tests,
   lifecycle scripts and UI Automation scripts. Bugs found and fixed:
   - `--minimized` still showed the window, because eframe always shows it
     after the first frame. Fixed with DWM cloaking.
   - Panel separator lines were visible.
   - The ✓ on the selected accent rendered as an empty box (Segoe UI has
     no such glyph). It now uses the icon-font checkmark.
   - Long setting descriptions overlapped the switches. They now wrap.
   - Custom sliders ignored screen-reader actions. They now handle
     SetValue, Increment and Decrement.
9. **Phase 5: hardening and delivery.**
   - A 15-round soak test showed no leaks.
   - `cargo clippy --fix` and `rustfmt` were applied, then everything was
     re-verified.
   - Docs and the packaging script were written.
   - Inno Setup 6.7.3 was installed and the installer built and tested.
     One fix came out of it: a silent uninstall no longer defaults to
     deleting settings.
10. **Publishing.**
    - GitHub CLI 2.102.0 was installed and the owner signed in as
      `ppolkadot`.
    - The initially private repo was created, `main` pushed, and release v0.1.0
      published with both binaries.
11. **Phase 1 maintenance on a feature branch.** Text encoding, backdrop
    status, icon retry, cache pruning, System sounds visibility, window
    geometry and sizing, and the two clippy lints were addressed. The
    settings schema was hardened before later phases add fields. No Phase 2
    feature was included in that maintenance commit.
12. **Phase 2 device controls and properties.** Added independent active-device
    controls, stereo balance, qualified sound-property readouts and Windows
    Settings shortcuts, and hidden warning/error tray notifications. A read-only
    probe on six endpoints ruled out assuming enhancement, spatial-provider or
    mono values. See `SOUND_PROPERTIES_SPIKE.md`. Settings schema is now 3.

---

## 4. Environment the app was built and tested on

| Item | Value |
| --- | --- |
| OS | Windows 11 Pro 25H2, build 26200 (dark mode on, transparency on, animations on, high contrast off) |
| Hardware | H610 motherboard, NVIDIA GeForce GT 340 display adapter |
| Audio | One output: "HDMI-1 (NVIDIA High Definition Audio)". **No input device.** |
| Live sessions during tests | System sounds (idle) and Zen Browser (playing) |
| Toolchain | rustup 1.29.1, rustc and cargo 1.99.0, VS 2022 Build Tools, Windows SDK 10.0.26100 |
| Tools | Inno Setup 6.7.3 (`%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe`), GitHub CLI 2.102.0, Git 2.55 |

Dependencies, as locked in `Cargo.lock`:

| Crate | Version | Use |
| --- | --- | --- |
| eframe / egui | 0.36.2 | UI. Features: `accesskit`, `default_fonts`, `glow`. wgpu is disabled. |
| windows / windows-core | 0.62.2 | All Win32/COM bindings, `#[implement]`, `#[interface]` |
| tray-icon (muda 0.21.2) | 0.26.1 | Tray icon and context menu |
| winit (via eframe) | 0.30.13 | Windowing and event loop |
| raw-window-handle | 0.6.2 | Getting the HWND |
| serde / serde_json | 1.0.229 / 1.0.154 | Settings |
| crossbeam-channel | 0.5.17 | Command and event channels |
| parking_lot | 0.12.5 | Shared state |
| log / simplelog | 0.4.34 / 0.12.2 | File logging |
| embed-resource (build) | 3.0.12 | Embeds the icon, manifest and version info |

---

## 5. Features as implemented

### 5.1 Output and input

- **Header (output):**
  - A device drop-down lists the active render endpoints; the current
    default is selected.
  - A mute button whose icon reflects the level (Vol0–Vol3, or Mute).
  - A volume slider with a live peak meter.
  - The percentage, or "Muted".
- **Footer (input):** the same controls for the default capture device.
  If there's no input device, it says "No microphone or other input device
  was found."
- **Device switching:** sets the console, multimedia and communications
  roles through `IPolicyConfig`. If that interface is unavailable, the
  drop-down is disabled with an explanatory tooltip. When the default
  changes (by us or anyone else), an "Output switched to …" notice appears.
- **Missing output:** "No output device is available. Connect speakers or
  headphones."
- Both default-device cards expose stereo balance; the Devices view controls
  all active endpoints independently and disambiguates duplicate names.
- Expandable sound properties show selected/default versus internal mix formats.
  Unverified format, spatial, enhancements and mono setters open Windows Settings.
  No new undocumented setter or elevation path is used.
- Hidden warnings/errors use quiet-policy-aware tray notifications, configurable
  in Behavior; 10-second throttle and 60-second text deduplication.
- **Starting up:** "Connecting to Windows audio…"
- **Audio service unreachable:** a warning banner with the error, "keeps
  retrying automatically", and a **Retry now** button.

### 5.2 Per-app mixer

- **Row layout:** the app icon, then its name and status, a mute button,
  the slider with meter, and the percentage.
  - Status reads "Playing" (with a green dot), "Idle" or "Muted", plus
    "· N streams" when an app has several sessions.
  - A muted app's icon is dimmed.
  - Status is always given as text, never by colour alone.
- **Expandable details** (click or activate the name):
  - A balance slider (L/R; double-click recentres), or "Not available for
    this app".
  - The source: exe path, package AUMID, or "Windows system sounds".
- **Grouping:** one row per `AppKey`:
  - `System` for the System sounds session.
  - `Package(AUMID)` for packaged apps.
  - `Exe(lower-cased path)` for desktop apps.
  - `Pid` as a fallback.

  A group shows the **loudest** session's volume, and counts as muted only
  if **all** its sessions are. Commands apply to every session in the group.
- **Order:** System sounds first, then by name (case-insensitive), so rows
  don't jump around.
- Hiding idle apps leaves System sounds visible so its control remains
  reachable even when Windows reports the session idle.
- **Naming:**
  1. The session's display name (indirect `@dll,-id` strings are
     resolved).
  2. The executable's `FileDescription`.
  3. The exe file stem, capitalised.
  4. Otherwise "Process N".

  Packaged apps get their shell display name from the icon thread.
- **Icons:**
  1. The session's `GetIconPath`.
  2. Packaged apps: the AppsFolder shell item for the AUMID.
  3. The exe through `IShellItemImageFactory`.
  4. Otherwise a coloured tile with the app's initial (a speaker glyph for
     System sounds).

  Icons are rendered at the exact pixel size (icon points × DPI scale) and
  cached per (app, size). Missing apps and obsolete sizes are pruned; failed
  lookups retry on Refresh or when the app returns.
- **Filter box:** Ctrl+F focuses it, Esc clears it. An "Apps N" count is
  shown. The ↻ button rebuilds every audio object.
- **Empty list:** says "No apps match …" or "No apps have audio sessions
  right now…".

### 5.3 Tray

- Left-click shows and focuses the window, immediately, through Win32.
- Right-click opens the menu: **Open Mixer**, **Mute** (a check item that
  always reflects Windows' state), and **Exit**.
- The tooltip reads "Glass Mixer — 46% · <device>", "Muted · <device>",
  "no output device" or "audio service unavailable". It is truncated to
  127 UTF-16 units.
- The tray icon is generated in code at the system small-icon size, with a
  separate muted version (grey with a slash).
- If the tray can't be created, the window is never hidden and closing it
  exits. A notice says so.

### 5.4 Window and lifecycle

- **Close button:** hides to the tray by default ("Keep running when
  closed"). With that setting off, it exits.
- **Optional minimize to tray:** hides the window when it's minimized.
- **`--minimized`, or "Start minimized":** starts with only the tray icon,
  with no flash (DWM cloak, then hide, then uncloak).
- **Single instance:** a `Local\GlassMixer.SingleInstance` mutex. A second
  launch finds the first window (same title and exe name), shows it, and
  exits.
- **Window position and size:** saved, and restored only if the saved
  position is still on a monitor. New positions use physical screen pixels;
  version-1 logical positions are checked against each monitor's DPI. If an
  old position is ambiguous, the window centers instead of guessing. The
  new default is 480×720, centred; the minimum remains 360×440. Existing
  saved sizes are preserved and Appearance offers a recommended-size action.
- **Exit:** tray **Exit**, or close with close-to-tray off. Shutdown is a
  single path:
  1. Save settings.
  2. Remove the tray icon.
  3. Stop the engine, unregistering every callback.
  4. Join the icon thread.

### 5.5 Appearance

- **Glass:** Acrylic (default) or Mica on Windows 11 22H2 and later; Mica
  only on 21H2 (undocumented attribute 1029). It turns off automatically
  when Windows transparency effects or high contrast are on, or on
  Windows 10. The settings page explains why it's off.
- **Theme:** System (default), Light or Dark. The title bar follows
  through `DWMWA_USE_IMMERSIVE_DARK_MODE`, and windows get rounded corners.
- **Accents:** Violet (default), Blue, Teal, Green, Amber, Coral and Pink.
  Each has a dark-theme and a light-theme value; the light values are
  darker to keep contrast.
- **Density:** Comfortable (52 pt rows, 28 pt icons) or Compact (40 pt
  rows, 22 pt icons; the status line is hidden).
- **Palettes:**
  - Dark glass: a translucent tint (rgba 16,16,22,96) with white
    surfaces at about 5 % alpha.
  - Light glass: rgba 250,250,253,110.
  - Opaque fallbacks for both.
  - A separate black/white/yellow high-contrast palette.
- **Fonts:** Segoe UI regular and semibold from `%WINDIR%\Fonts`. Icons use
  Segoe Fluent Icons, or Segoe MDL2 Assets on Windows 10. Without an icon
  font, text fallbacks are drawn.
- **Windows preferences** are re-read every 2 s while the window is visible
  (theme, transparency, animations, high contrast). Reduced motion sets
  egui's animation time to 0.
- **Notices** are toasts above the bottom edge: info 4 s, warning 7 s,
  error 9 s, at most three. Click one to dismiss it.

### 5.6 Accessibility

- AccessKit exposes everything to UI Automation, so Narrator and other
  screen readers work. Verified: the window exposes 16 elements, each with
  a meaningful name and role. Examples:
  - "Mute System sounds"
  - "System sounds volume" (a slider with a 0–100 RangeValue)
  - "Zen Browser, Playing. Show details"
- **Sliders:**
  - Arrow keys: ±1 %. Shift+arrow: ±5 %. Page Up/Down: ±10 %.
    Home/End: 0 or 100 %.
  - Mouse wheel: 2 % per notch, without scrolling the list.
  - UIA SetValue, Increment and Decrement are supported.
- Every custom widget draws a visible 2 px focus ring. Icon-only buttons
  have tooltips.

### 5.7 Settings page

Settings is a separate page with a back button (Esc also goes back). It has
four sections:

| Section | Contents |
| --- | --- |
| **Appearance** | Theme, Glass effect (with a status line), Backdrop, Accent, Density, Level meters, recommended window size |
| **Behavior** | Launch at login (shows when Task Manager has disabled it), Start minimized, Keep running when closed, Minimize to tray, Show idle apps |
| **Audio** | A "Refresh audio devices" button, plus a note about default-device switching |
| **About** | The version, "No telemetry…", the settings path, and "Open settings folder" |

---

## 6. Files, data and system footprint

| Path or key | Purpose |
| --- | --- |
| `%APPDATA%\GlassMixer\settings.json` | Preferences. Written atomically through `settings.json.tmp`. A corrupt file is renamed to `settings.json.bad`. |
| `%LOCALAPPDATA%\GlassMixer\glass-mixer.log` | Log: debug level in debug builds, info in release. Rotated to `glass-mixer.old.log` above 1 MB. |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\GlassMixer` | Launch at login: `"<exe>" --minimized`. This registry value is the only record of the setting. |
| `HKCU\...\Explorer\StartupApproved\Run\GlassMixer` | Task Manager's enabled/disabled flag. Read to show the "disabled" note; written as enabled when the user turns the setting on. |
| Mutex `Local\GlassMixer.SingleInstance` | Single instance per logon session |
| `%LOCALAPPDATA%\Programs\Glass Mixer\` | Installer target, plus a Start-menu shortcut and an HKCU uninstall entry |

`settings.json` schema (version 3). Invalid known values fall back to their
default field by field. Unknown fields and newer version numbers are retained
when the file is saved:

```json
{
  "version": 3,
  "theme": "system | light | dark",
  "glass": true,
  "backdrop": "acrylic | mica",
  "accent": "violet | blue | teal | green | amber | coral | pink",
  "density": "comfortable | compact",
  "start_minimized": false,
  "close_to_tray": true,
  "minimize_to_tray": false,
  "show_inactive_apps": true,
  "show_meters": true,
  "hidden_notifications": true,
  "window": { "x": 0.0, "y": 0.0, "width": 480.0, "height": 720.0 },
  "window_position_physical": true
}
```

`window` is normally omitted until the first window geometry is saved. A
version-1 file lacks `window_position_physical`; its coordinates are treated
as egui logical points. An ambiguous legacy position is not restored.

Volume levels are deliberately **never** stored; Windows stays
authoritative. There is no network access, telemetry or analytics, and no
admin rights or services are needed.

---

## 7. Architecture

### Threads

| Thread | Role |
| --- | --- |
| UI (`main`) | Runs the winit/eframe event loop. Owns `MixerApp`, the tray icon (not `Send`), the DWM backdrop and textures. `logic()` runs every frame and also while hidden, on repaint requests. |
| `audio-engine` | COM MTA. Owns every Core Audio object. Loop: `maintain()` (rebuild / structural refresh / publish), wait on the channel (blocking indefinitely when idle), drain and coalesce messages, apply commands, poll meters. |
| Core Audio callback threads | Owned by the audio service. Callbacks only `send()` a `Msg`. |
| Thread pool | `RegisterWaitForSingleObject` process-exit callbacks, and power callbacks. |
| `icon-resolver` | COM STA. Shell icon and name extraction, with a per-process cache. |

### Data flow

```
UI action ─► AudioHandle::send(Command) ─► engine applies (with OWN_EVENT_CONTEXT)
         ◄─ snapshot published (only if different) ◄─ engine re-reads Windows
Windows change ─► callback ─► Msg::*Changed ─► engine re-reads ─► snapshot ─► repaint
```

### Source map

| File | Lines | Responsibility |
| --- | --- | --- |
| `src/main.rs` | ~197 | Logging, panic hook, single-instance check, settings load, monitor-aware physical window restore, window options, OpenGL failure message box |
| `src/app.rs` | ~590 | `MixerApp`: lifecycle, tray actions, close and minimize to tray, start-hidden cloak, prefs polling, sticky backdrop status, settings debounce (800 ms), toasts, shutdown |
| `src/settings.rs` | ~356 | `Settings`, enums, lenient versioned parsing, preservation of future fields, `SettingsStore` (load outcomes, atomic save), with tests |
| `src/icon_art.rs` | ~187 | Procedural app icon (SDF, 4×4 supersampled) and ICO encoder. Shared with `build.rs`. |
| `build.rs` | ~72 | Generates `app.ico` and the `.rc`; embeds the icon, manifest (PerMonitorV2, asInvoker, UTF-8, Common Controls 6) and VERSIONINFO |
| `src/audio/mod.rs` | ~239 | `Command`, `Notice`, internal `Msg`, `Shared` (snapshot, meters, notices, listeners), `AudioHandle` (start / send / snapshot / shutdown with a 2 s timeout) |
| `src/audio/model.rs` | ~384 | Snapshot types, `AppKey`, `group_sessions`, balance math, `to_percent`, filter including System sounds, names, tooltip, with tests |
| `src/audio/engine.rs` | ~919 | `Runner` (dirty flags, backoff, delayed resume rebuild, meters), `Engine`, `Endpoint`, `SessionSet`, `Session`, command application, error mapping, with tests |
| `src/audio/callbacks.rs` | ~173 | `IMMNotificationClient`, `IAudioEndpointVolumeCallback`, `IAudioSessionNotification`, `IAudioSessionEvents`; `OWN_EVENT_CONTEXT` |
| `src/audio/policy_config.rs` | ~52 | The undocumented `IPolicyConfig` vtable and `set_default` (all three roles) |
| `src/audio/process.rs` | ~185 | `OpenProcess` (limited), image path, AUMID, `FileDescription`, `ProcessWatch` |
| `src/audio/power.rs` | ~59 | Suspend/resume subscription |
| `src/audio/com.rs` | ~71 | `ComGuard`, `take_pwstr`, fatal and not-found HRESULT classification |
| `src/platform/backdrop.rs` | ~115 | `Support::for_build`, `apply()` (dark title bar, round corners, frame extension, backdrop type), with tests |
| `src/platform/tray.rs` | ~109 | Tray icon, menu, event handlers, `update(snapshot)` |
| `src/platform/icons.rs` | ~418 | `IconService` (request / retry / prune / take_ready / shutdown), resolution chain, HBITMAP/HICON to RGBA, with tests |
| `src/platform/window.rs` | ~133 | `Hwnd`, show / hide / cloak / visibility, physical window position, `activate_existing_instance` |
| `src/platform/startup.rs` | ~101 | Run key state, enable/disable, path repair |
| `src/platform/sysprefs.rs` | ~64 | Dark mode, transparency, animations, high contrast, build number |
| `src/platform/fonts.rs` | ~50 | Segoe UI, semibold and icon-font families |
| `src/platform/single_instance.rs` | ~36 | Mutex guard |
| `src/platform/mod.rs` | ~53 | Registry read helpers |
| `src/ui/theme.rs` | ~211 | `Palette`, accents, `Metrics`, `apply()` (same style for both egui theme slots; egui's window-theme sync disabled) |
| `src/ui/widgets.rs` | ~417 | Glyphs, `Slider`, `icon_button`, `toggle_row`, `segmented`, `fallback_tile`, `card`, `percent_label`, focus ring |
| `src/ui/mixer.rs` | ~520 | Mixer view, `MixerState` (filter, expanded rows, value holds, pruned textures, names) |
| `src/ui/settings_view.rs` | ~224 | Settings view and recommended-size action |
| `src/ui/mod.rs` | ~37 | `Look`, `GlassStatus` |
| `tests/audio_integration.rs` | ~200 | Real-hardware tests (ignored by default) plus the silent-stream helper |
| `tests/source_encoding.rs` | ~24 | Rust source UTF-8 and mojibake regression scan |
| `examples/audio_probe.rs` | ~97 | Read-only diagnostic: dumps state and watches for changes |
| `examples/glass_spike.rs` | ~85 | Minimal glass test case (`GM_BACKDROP`, `GM_EXTEND`, `GM_DARK`) |
| `scripts/package.ps1` | ~51 | Release build, portable zip, and the installer when ISCC is present |
| `installer/glass-mixer.iss` | ~67 | Inno Setup script |

### Important constants

Phase 2 adds `audio/properties.rs`, `examples/sound_properties_probe.rs` and
`tests/devices_integration.rs`. The earlier approximate source-map counts above
describe Phase 1; the additional code is organized around those modules and
the existing engine, mixer and tray owners.

| Constant | Value |
| --- | --- |
| Meter polling | 33 ms, only while the window is visible and meters are on |
| UI value hold after a slider moves | 450 ms |
| Engine rebuild backoff | 250 ms, doubling to 30 s |
| Rebuild delay after resume | 1.5 s |
| Settings save debounce | 800 ms |
| System preferences re-check | 2 s, only while visible |
| Meter change needed to repaint | 0.01 |
| `AudioHandle::shutdown` wait | 2 s, then detach |

### Errors treated as fatal (rebuild everything)

- `AUDCLNT_E_DEVICE_INVALIDATED` (0x88890004)
- `AUDCLNT_E_SERVICE_NOT_RUNNING` (0x88890010)
- `RPC_E_DISCONNECTED`
- `RPC_S_SERVER_UNAVAILABLE`
- `RPC_S_CALL_FAILED`

`E_NOTFOUND` from `GetDefaultAudioEndpoint` means "no device", not an
error.

---

## 8. Key decisions and why

1. **egui/eframe on glow (OpenGL), not wgpu, iced or Slint.**
   - Immediate mode makes "render from the latest Windows snapshot"
     trivial.
   - AccessKit provides screen-reader support.
   - Transparent windows work.
   - eframe only repaints on events, so idle cost is near zero.
   - glow was chosen because a transparent glow surface was confirmed to
     composite with DWM backdrops in the spike.
   - Cost: OpenGL is required (see section 11).
2. **Our own DWM calls rather than the `window-vibrancy` crate.** This
   means one dependency fewer and full control over fallback detection.
3. **Windows is the single source of truth.** Callbacks carry no values;
   the engine re-reads after every event, and snapshots are compared
   before publishing.
4. **Our own changes are tagged with `OWN_EVENT_CONTEXT`** and their
   echoes ignored, with exactly one re-read after each command batch. In
   the UI, a dragged value is held for 450 ms to prevent jitter.
5. **Commands are coalesced** per control, so a burst of slider events
   collapses into the latest value.
6. **Session-created callbacks trigger a re-enumeration** rather than
   carrying a COM pointer across threads.
7. **Process exit is detected with `RegisterWaitForSingleObject`** instead
   of polling. Windows can keep the session of a closed app around for a
   while.
8. **The tray lives on the UI thread,** and window showing goes through
   Win32 directly. This relies on `App::logic` running while hidden, and
   on focus rights during the tray click.
9. **Starting hidden uses a DWM cloak,** because eframe forcibly shows the
   window after its first frame.
10. **Default switching uses `IPolicyConfig`.** No public API exists. If it
    can't be created, switching degrades to disabled, never to a silent
    failure.
11. **Only sessions on the default output are shown,** as in Windows' own
    mixer.
12. **Launch at login is stored only in the registry.**
13. **Icons are generated in code.** One source (`icon_art.rs`) feeds the
    exe ICO, the window icon and the tray icon, so the repo has no binary
    assets.
14. **Window positions use physical screen pixels.** egui reports logical
    points using the window's current scale. Saving those points without the
    source monitor's DPI made mixed-DPI restore ambiguous. Win32 now stores
    new positions in physical pixels and validates them against physical
    monitor rectangles. An ambiguous version-1 position is centered.
15. **Saved window sizes stay user-owned.** The new 480×720 default applies
    only without saved geometry. Existing sizes are not silently enlarged;
    Settings provides an explicit recommended-size action.
16. **System sounds is exempt from the idle-app filter.** It is normally idle
    yet is a useful control. Textures and icon results are pruned by live app
    and pixel size, while failed lookups can retry.
17. **Properties degrade to Windows Settings.** Stream format support does not
    prove default-format setter compatibility; missing effects/mono properties
    are unknown. Build reliable readouts and shortcuts without guessed setters.
18. **Additional device owners are lazy and RAII-managed.** Opening Devices
    creates callbacks; removal/rebuild drops them. Fatal read errors rebuild.
19. **Shell notifications reuse the tray icon.** No AUMID/shortcut deployment
    is needed. The GUID includes the executable path for unsigned portable builds.

---

## 9. Verification record

All on the machine in section 4, on 2026-10-15.

- **Unit tests:** `cargo test`, 22 of 22 passed.
- **Integration tests:** `cargo test --test audio_integration -- --ignored --test-threads=1`,
  7 of 7 passed. Latencies:
  - External master-volume change reflected: about 20 ms.
  - New session detected: about 20 ms.
  - Session removed after its process exits: about 20 ms.

  These latencies are bounded by the test's 20 ms polling, so the real
  values may be lower.
- **UI Automation** (script, release build): found the window and 16
  named elements.
  - Invoking "Mute System sounds" made the engine report it muted, and
    invoking it again unmuted it.
  - `RangeValue.SetValue(90)` on "System sounds volume" set the Windows
    session to 90 %, and it was restored to 100 %.
- **Lifecycle** (script, debug and release builds), all passing:
  - The window shows on a normal start.
  - WM_CLOSE hides it and the process keeps running.
  - A second launch exits and restores the first window; only one
    instance runs.
  - `--minimized` starts hidden and keeps running.
  - `settings.json` is written with geometry, and glass is on by default.
- **Resources** (release build):
  - CPU over 10 s: **0 ms hidden**, about 78 ms visible (the 2 s prefs
    re-check plus meters while Zen Browser was playing).
  - Memory: about 44 MB private, 56 MB working set, 14 threads at
    start.
- **Soak test:** 15 rounds of session create/destroy (the silent helper)
  plus System sounds volume and mute changes.
  - Private memory went 45.9 → 55.3 MB over the first 5 rounds (warm-up),
    then 55.6 MB after 15 (flat).
  - Handles: 499 → 504, flat.
  - Threads: 16 → 20 (Windows thread pool and RPC workers).
- **Installer** (silent mode):
  - Exit code 0; the exe, Start-menu shortcut and uninstall entry were
    present, and the installed app ran.
  - Uninstall: exit code 0; the app was stopped, its files removed,
    settings kept, and no Run-key entry left behind.
- **Visual checks** (window-only `PrintWindow` captures): the dark mixer,
  dark settings, light mixer and light settings pages.
  - These captures don't include the DWM backdrop. The backdrop itself was
    verified by the spike.
- **Static checks:** `cargo clippy --all-targets` leaves 2 style lints:
  `field_reassign_with_default` in `theme.rs`, and `type_complexity` in
  `audio/mod.rs`.

**Not verified:**

- Windows 10 and Windows 11 21H2.
- Any microphone path.
- Switching between two real outputs.
- USB/Bluetooth hot-plug.
- Sleep/resume.
- Audio service restart.
- High contrast, transparency off, and animations off.
- Display scaling other than this machine's, and multi-monitor DPI changes.
- Narrator speech (only the UIA tree was checked).
- Packaged (Store) app icons and names.
- Elevated or protected processes.
- Tray left-click and menu clicks (only invoked indirectly; the tray
  handlers share code paths that were tested).
- Running for 24 hours or more.

### Phase 1 maintenance verification (2026-10-15)

This is a separate record for the unmerged `codex/glass-mixer-features`
branch. It does not change the v0.1.0 release results above.

- **Ran and passed:** `cargo fmt --check`; `cargo clippy --all-targets --
  -D warnings`; `cargo test` (28 library, 1 binary and 1 source-encoding
  test); and the seven ignored, self-restoring audio integration tests run
  serially. `cargo build` and `git diff --check` also passed.
- **Ran and failed during development:** the new regression tests first
  exposed the unfixed source encoding, settings parsing and missing Phase 1
  behavior. Initial formatting and clippy runs also identified changes that
  were corrected before the passing runs above.
- **Not run:** a fresh launch/tray/shutdown test of this branch, because an
  installed `glass-mixer.exe` already owns the single-instance mutex; 100%,
  150% and 200% visual checks in both densities; a real mixed-DPI restore;
  actual DWM rejection; and Windows 10, microphone and multi-output checks.
  These remain on the manual checklist. A test of the old release is not
  evidence for the branch build.

---

## 10. Gaps against the specification

Phase 2 verification: 36 hardware-independent tests, clippy with warnings denied,
formatting, the read-only properties probe and the six-endpoint self-restoring
device test passed. An initial clippy module-ordering failure was corrected.
GUI lifecycle/visual DPI, Focus Assist, Windows 10, hot-plug, sleep/resume and
service restart were not run. Full categories are in `TESTING.md`.

| Spec item | Status |
| --- | --- |
| Per-device volume and mute (§3.4) | Implemented in Devices; six active endpoints tested. |
| Device balance (§3.4) | Implemented; three stereo outputs tested through master-volume changes. |
| Native notifications (§2) | Implemented while hidden; Focus Assist visual suppression remains unverified. |
| Sound properties | Reliable readouts and Windows Settings alternatives; no unverified setters. |
| Windows 10 testing (§13, §14 Phase 5) | Not done (no machine available). The fallback paths are written but unverified. |
| Integration tests for tray, startup, appearance, sleep, device switching (§13 items 6–9, 12–13) | Covered by scripts outside the repo or by the manual checklist, not by `cargo test`. |
| Mic tests (§13 item 7) | Three input endpoints accepted volume changes; real recording, mute and hot-plug remain unverified. |
| Accessible names (§11) | Done for all custom widgets. The device ComboBox uses egui's default accessibility. |
| Code signing | Not done. The exe and installer are unsigned, so SmartScreen may warn on first run. |

---

## 11. Possible bugs, risks and edge cases

The glass status, failed-icon retry, texture growth, System sounds filter
and system-DPI position check were addressed in Phase 1. The following
risks remain; hardware-dependent paths need the manual checks in section 9.

### Confirmed in code (not yet fixed)

1. **Sound-property setters are unavailable in-app** (see the spike report).
2. **Group volume flattens per-session differences.** The first change
   to a multi-session app sets every session to the same value.
   Intentional, but surprising if an app deliberately runs different
   streams at different volumes.

### Plausible risks

3. **No OpenGL, no window.** In some Remote Desktop sessions, VMs or with
   the Microsoft Basic Display Adapter, eframe/glow can't start. The app
   shows a message box and exits. There's no wgpu/DirectX fallback.
4. **Acrylic goes solid when the window loses focus.** This is Windows'
   own behaviour for `DWMSBT_TRANSIENTWINDOW`, but users may read it as
   glass "turning off".
5. **Single-instance activation needs the same exe name.** If the exe is
   renamed, the mutex still blocks a second instance, but the first
   window isn't found, so the second launch exits silently.
6. **Portable copies fight over launch at login.** `repair_path_if_enabled`
   re-points the Run key to whichever copy started last.
7. **The uninstaller kills every running `glass-mixer.exe`**
    (`taskkill /IM … /F`), including a separate portable copy.
8. **Device names can repeat.** Two devices with the same friendly name
    are indistinguishable in the drop-down.
9. **Legacy positions can be ambiguous.** A version-1 logical position
    can correspond to more than one monitor at different scales. In that
    case the app centers rather than claiming to know the original monitor.
10. **Meters are approximate.** Peaks are pre-mute and may be pre-volume
    depending on the stream. The meter line is clipped to the slider value
    and hidden while muted.
11. **PID reuse.** The process cache is keyed by PID and pruned when no
    session uses it. A reused PID inside that window could briefly show
    the wrong exe identity. This is very unlikely.
12. **Elevated or protected processes:** limited process access may
    fail, giving `AppKey::Pid`, no icon, and a "Process N" name unless the
    session supplies a name.
13. **Untested recovery paths:**
    - The rebuild 1.5 s after resume may come too early for slow drivers.
      The backoff should recover.
    - An audio service restart should show "unavailable" and then
      reconnect.
14. **Prefs polling while visible** causes a repaint every 2 s (the
    visible CPU cost measured above). Listening for `WM_SETTINGCHANGE`
    would avoid this, but needs a message hook that eframe doesn't
    expose.
15. **High contrast uses a fixed palette** (black, white and yellow), not
    the user's chosen system high-contrast colours.
16. **Meter polling continues in silence.** Meters are polled at about
    30 Hz while visible even when nothing plays. Repaints are skipped
    unless values change, but the COM calls continue.
17. **Debug builds open a console** (`windows_subsystem` is only set for
    release). This is expected; mentioned to avoid confusion.
18. **Hidden-window latency.** While hidden, eframe throttles repaints to
    10 Hz. Tray clicks act immediately (direct Win32), but processing
    `Exit` from the tray can take up to about 100 ms.

---

## 12. How to work on it

### Prerequisites

- Rust stable MSVC.
- VS 2022 Build Tools with the C++ workload and Windows SDK.
- Optional: Inno Setup 6 (`winget install JRSoftware.InnoSetup`).

### Commands

```powershell
cargo run                                   # debug build, console + debug log
cargo build --release                       # target\release\glass-mixer.exe
cargo test                                  # unit tests
cargo test --test audio_integration -- --ignored --test-threads=1 --nocapture
cargo run --example audio_probe -- 30       # read-only diagnostics, watch 30 s
$env:GM_BACKDROP="mica"; cargo run --example glass_spike
powershell -ExecutionPolicy Bypass -File scripts\package.ps1   # dist\ zip + installer
cargo clippy --all-targets; cargo fmt       # rustfmt.toml: max_width 120
```

### Releasing

1. Bump `version` in `Cargo.toml`.
2. Run `scripts\package.ps1`.
3. Commit, then run `gh release create vX.Y.Z dist\* --title …`.

### Test scripts not in the repo

These were used during development and live in the session scratchpad:

| Script | What it does | Built on |
| --- | --- | --- |
| Lifecycle | Close-to-tray, second instance, `--minimized`, CPU and memory, settings | `EnumWindows` + `PostMessage(WM_CLOSE)` |
| UI Automation | Invokes "Mute System sounds", calls `RangeValue.SetValue` on "System sounds volume", and checks the result with `audio_probe` | `System.Windows.Automation` |
| Window capture | Captures only the app's own window | `PrintWindow(PW_RENDERFULLCONTENT)` |

Worth adding to `scripts/` as a follow-up.

### Conventions

- Windows calls are wrapped in small owners whose `Drop` unregisters or
  closes them. Callbacks only send messages.
- User-facing errors go through `Shared::push_notice` (engine) or
  `MixerApp::toast` (UI). Messages are short, actionable, and never
  claim success.

### Suggested next steps, in priority order

1. Validate the expanded UI and device lifecycle on additional hardware.
2. Run the manual checklist on Windows 10 and on a machine with a
   microphone and two outputs.
3. Verify notification suppression under Focus Assist.
4. Verify hot-plug and external changes for independent device controls.
5. Revisit sound-property setters only with reproducible cross-build evidence.
6. Add opt-in hotkeys, app pins and explicit presets.
7. Add a wgpu fallback renderer for machines without OpenGL.
8. Sign the binaries when the owner provides a certificate.
9. Commit the lifecycle and UIA scripts, and run unit tests in CI on a
   Windows runner.

---

## Appendix A — Original specification (verbatim)

# Project Specification: Native Windows Audio Mixer

## 1. Project Overview

Build a fully functional, lightweight, native Windows audio mixer that runs in the system tray and provides a centralized interface for controlling audio across the computer.

The application should combine the functionality of the Windows Volume Mixer with a more modern, visually appealing, customizable interface.

**Primary objective:** Give users fast, reliable control over every audio device and application volume exposed by Windows, without needing to navigate Windows Settings.

This is a real desktop application, not a web app, mockup, or static UI prototype. All controls must interact with the actual Windows audio system.

## 2. Technology Requirements

- **Primary language:** Rust.
- **Operating system:** Windows 10 and Windows 11, with Windows 11 as the primary design target.
- **Audio integration:** Windows Core Audio APIs, particularly the MMDevice API, `IAudioEndpointVolume`, `IAudioSessionManager2`, and `ISimpleAudioVolume`.
- **Windows integration:** Native system tray, native notifications where appropriate, startup integration, and Windows window-management APIs.
- **UI:** Choose a mature Rust desktop GUI framework that supports custom styling, native Windows integration, responsive controls, and transparent or translucent window effects.
- **Distribution:** Produce a standalone Windows executable and, where practical, a Windows installer.

Choose compatible, actively maintained libraries. Verify that the selected framework can support the required native glass effects before committing to the UI architecture.

Avoid unnecessary web technologies, heavyweight runtimes, and background services. Prefer a lightweight user-mode application that does not require administrator privileges for normal operation.

If Rust proves impractical for a specific Windows integration, explain the limitation and propose a concrete alternative rather than silently replacing the primary language.

## 3. Core Audio Functionality

### 3.1 Master volume

Provide controls for:

- System output volume.
- Master mute and unmute.
- Current default output device.
- Available output devices and switching between them.
- Available input devices and switching between microphones.
- Microphone input volume and mute, where supported.
- Clear identification of the currently selected devices.

Volume changes must take effect immediately and remain synchronized with changes made through Windows Settings, keyboard volume keys, hardware controls, and other applications.

### 3.2 Per-application volume mixer

Detect applications with active Windows audio sessions and display them individually.

Each application entry must support:

- Independent volume adjustment.
- Mute and unmute.
- Current volume percentage.
- Application name and icon.
- A clear indication of whether the application is producing audio, is muted, or has an inactive audio session.
- Automatic discovery of newly created audio sessions.
- Removal or updating of sessions when applications close or their audio sessions change.

Volume controls must affect the selected application's Windows audio session rather than changing the entire system volume.

Support applications that use multiple audio sessions when Windows exposes them. Avoid creating duplicate entries for the same application unless there is a meaningful reason to separate its sessions.

Where appropriate, identify applications by their executable or package identity instead of relying exclusively on display names.

### 3.3 Application icons

Every application in the mixer should display its own recognizable icon.

Use this resolution strategy:

1. Extract the icon from the application's executable or package resources.
2. Use available Windows application identity information when executable extraction is insufficient.
3. Fall back to a consistent generic application icon if no suitable icon can be retrieved.

Cache extracted icons to avoid repeated disk access and unnecessary processing.

Handle missing executables, packaged applications, permission restrictions, and applications that do not expose a conventional executable path.

Icons should be crisp, correctly scaled, and consistent with the overall interface.

### 3.4 Additional audio controls

Include the following where the Windows APIs expose the necessary functionality:

- Per-device volume and mute.
- Microphone volume and mute.
- Device switching.
- Audio-session balance controls, if supported by the chosen API and session.
- A searchable or filterable application list.
- A refresh or recovery action for audio-session discovery.

Clearly distinguish unavailable functionality from a volume value of zero.

Do not promise independent control of sounds that Windows does not expose as separate audio sessions. Some system sounds, exclusive-mode streams, protected audio paths, and hardware-specific controls may behave differently.

## 4. System Tray Behavior

The application must behave like a native Windows utility.

### Required behavior

- Display a tray icon while running.
- Keep running when the main window is closed using its close button, according to the configured close behavior.
- Provide a tray menu with Open Mixer, Mute/Unmute, and Exit.
- Restore the window when the tray icon is activated.
- Minimize to the tray instead of terminating when appropriate.
- Support starting minimized.
- Support launching at Windows login through an optional, user-controlled setting.
- Exit completely when the user explicitly selects Exit.
- Avoid duplicate application instances unless multi-instance operation is intentionally supported.

Provide a useful tray tooltip, such as the current master volume and output device, when technically feasible.

Tray interactions must work even when the main window is not visible.

## 5. Design Direction

Create a minimalist, colorful, polished desktop interface inspired by modern Windows 11 design, contemporary audio utilities, and high-quality desktop applications.

The interface should feel native, fast, and visually refined rather than resembling an outdated control panel.

### Visual principles

- Minimalist layout with a clear hierarchy.
- Rounded corners and restrained spacing.
- Subtle shadows and depth.
- Colorful but tasteful accent colors.
- Smooth, short transitions.
- High-contrast text and readable volume values.
- Clear icons and understandable mute states.
- No unnecessary decoration, oversized headers, or excessive gradients.
- Consistent spacing, typography, and control sizing.
- Support for different window sizes without breaking the layout.

Use color to communicate meaningful states, such as active audio, muted sessions, selected devices, and interactive controls. Do not rely on color alone to communicate status.

### Main mixer layout

The main window should prioritize immediate access to the most frequently used controls.

**Header**
- Application name and icon.
- Current output device selector.
- Master volume and mute control.
- Settings access.

**Application list**
- One row or compact card per application.
- Application icon and name.
- Volume slider.
- Numeric volume percentage.
- Mute button.
- Optional activity indicator.
- Consistent alignment across all entries.

**Footer or secondary section**
- Input device and microphone controls.
- Optional search and filtering.
- Access to advanced settings.

Avoid cramming every feature into the main window. Secondary controls can live in a compact settings or device panel.

## 6. Glass Effect — Enabled by Default

Glass-like translucency is a core design requirement, not an optional theme that is disabled by default.

On Windows 11, use the appropriate native system backdrop, such as **Mica or Acrylic**, depending on which best suits the window and the chosen framework. Use a more translucent Acrylic-style effect when appropriate for a floating mixer.

Requirements:

- Enable the glass effect automatically on first launch.
- Keep text, sliders, icons, and buttons legible over the backdrop.
- Use translucent surfaces with subtle borders and shadows.
- Avoid excessive blur or transparency that reduces usability.
- Ensure the effect works with both light and dark themes.
- Preserve the appearance of the interface during window resizing and movement.
- Use supported Windows APIs rather than simulating glass with a static gradient alone.
- Detect unsupported backdrops and fall back to a visually consistent opaque surface.

The application must remain usable when transparency, animations, or other visual effects are disabled by Windows accessibility settings.

Provide a setting to disable glass effects, but make the effect enabled by default.

## 7. Themes and Appearance

Provide:

- Light theme.
- Dark theme.
- Follow Windows appearance setting.
- Glass effect toggle.
- Accent color selection, with a tasteful default palette.
- Adjustable interface density if practical.

The default appearance should use dark translucent surfaces, vibrant but restrained accents, and clear text contrast. Light mode must be designed intentionally rather than implemented as a simple inversion of dark mode.

Persist user preferences across application restarts.

## 8. Audio Architecture and Reliability

Use an event-driven architecture wherever practical.

The audio subsystem should monitor Windows audio endpoints and sessions, respond to volume changes, and notify the UI when the underlying audio state changes.

Requirements:

- Keep the UI responsive while enumerating audio sessions.
- Avoid polling aggressively when Windows notifications can provide the necessary updates.
- Handle device disconnection and reconnection.
- Handle default-device changes.
- Handle applications starting, stopping, and recreating audio sessions.
- Handle COM initialization and apartment requirements correctly.
- Manage native callbacks and resource lifetimes safely.
- Unregister callbacks and release native resources during shutdown.
- Prevent feedback loops when reflecting volume changes back into Windows.
- Recover gracefully from temporary API errors.
- Avoid blocking the UI thread with native API calls.
- Handle sleep, wake, and audio-device reinitialization.
- Avoid memory leaks, runaway CPU usage, and unnecessary background work.

Changes made outside the mixer must be reflected in its UI promptly.

Use the Windows audio session APIs as the source of truth. Do not maintain an independent volume state that can drift out of sync with Windows.

## 9. Performance and Resource Usage

This application should be suitable for running continuously in the background.

Optimize for:

- Low idle CPU usage.
- Low memory consumption.
- Fast startup.
- Minimal background activity.
- Immediate response to user interactions.
- Efficient application icon caching.
- No unnecessary network access.
- No telemetry or analytics by default.

Do not introduce a permanent elevated-privilege service merely to simplify implementation.

The application should not require internet access to perform its core functions.

## 10. Settings and Persistence

Persist non-sensitive user preferences, including:

- Theme and accent color.
- Glass-effect preference.
- Startup behavior.
- Window dimensions and position.
- Selected interface density.
- Other UI preferences.

Store settings in an appropriate per-user Windows application data directory.

Do not persist application volume levels independently unless there is a clearly defined feature that requires it. Windows should remain authoritative for actual session volumes.

If saving or restoring volume levels is implemented as an additional feature, make its behavior explicit and avoid unexpectedly overriding user changes.

## 11. Accessibility and Usability

- Support keyboard navigation.
- Provide accessible names for controls.
- Ensure visible keyboard focus.
- Support Windows display scaling.
- Keep controls usable at common display resolutions.
- Respect reduced-motion and transparency accessibility preferences.
- Provide sufficient contrast in both themes.
- Ensure sliders can be adjusted precisely using keyboard input.
- Provide clear feedback for device changes and audio API failures.

Use tooltips where they improve clarity, particularly for icon-only buttons.

## 12. Error Handling

Handle errors without crashing or silently presenting incorrect audio state.

Examples include:

- No available output device.
- No available input device.
- Audio service unavailable.
- Application audio session becomes invalid.
- Device disappears while the user is adjusting volume.
- Application icon cannot be extracted.
- Glass effect is unsupported.
- Settings cannot be read or written.
- Windows API calls fail unexpectedly.

Show concise, actionable messages when user intervention is needed.

Never report that a volume change succeeded if the underlying Windows API rejected the change.

## 13. Testing Requirements

Create automated tests wherever practical and include a repeatable Windows test procedure.

Test the following:

1. Master volume and mute control.
2. Per-application volume and mute control.
3. Synchronization with Windows Settings and external volume changes.
4. Detection of newly started audio sessions.
5. Removal of terminated sessions.
6. Device switching and device disconnection.
7. Microphone volume and mute.
8. Tray opening, minimizing, restoring, and exiting.
9. Startup behavior.
10. Icon extraction and fallback handling.
11. Settings persistence.
12. Light, dark, and glass-enabled appearances.
13. Recovery after sleep or audio-device changes.
14. Long-running operation without resource leaks.
15. Error handling when audio devices or sessions become unavailable.

Separate unit tests for state management and settings from integration tests that require actual Windows audio devices.

Include a manual validation checklist for cases that cannot be reliably tested in a headless environment.

Do not claim Windows integration tests passed unless they were actually executed on a suitable Windows environment.

## 14. Implementation Workflow

Build the project incrementally.

### Phase 1 — Technical foundation
- Select the GUI framework and Windows API bindings.
- Establish the Rust project structure.
- Implement master volume control.
- Implement audio-device enumeration.
- Validate bidirectional synchronization with Windows.

### Phase 2 — Per-application mixer
- Enumerate audio sessions.
- Implement independent session volume and mute.
- Monitor session creation and termination.
- Retrieve application names and icons.
- Handle session and device changes.

### Phase 3 — Tray integration
- Implement the native tray icon and menu.
- Add minimize-to-tray and restore behavior.
- Implement proper shutdown and optional startup integration.

### Phase 4 — UI and appearance
- Build the complete mixer interface.
- Apply the minimalist, colorful visual design.
- Implement the default glass backdrop.
- Add light and dark themes and settings.

### Phase 5 — Hardening and delivery
- Test audio synchronization and failure recovery.
- Measure idle CPU and memory usage.
- Test Windows 10 and Windows 11 behavior.
- Fix lifecycle, scaling, and accessibility issues.
- Produce release binaries and packaging instructions.

Prioritize working audio controls before cosmetic polish, but establish the design system early enough to avoid rebuilding the interface.

## 15. Deliverables

Produce:

- Complete Rust source code.
- A clear project directory structure.
- Dependency manifest and lockfile.
- Build and run instructions.
- Windows-specific API and architecture documentation.
- Automated tests and their execution instructions.
- A manual Windows audio validation checklist.
- Release build instructions.
- A standalone Windows executable.
- An installer if practical.
- A concise README describing features, limitations, and troubleshooting.

Keep the implementation modular, with separate responsibilities for audio management, Windows integration, UI, settings, and application lifecycle.

Do not leave core controls as placeholders or replace real audio operations with simulated values.

## 16. Definition of Done

The application is considered complete when:

- It builds and launches successfully on Windows.
- It runs in the system tray without requiring the main window to remain open.
- It can control master volume and mute through native Windows APIs.
- It can independently control the volume and mute state of multiple active application audio sessions.
- It displays recognizable application icons when available and reliable fallbacks otherwise.
- It detects and handles audio-session and device changes.
- It synchronizes correctly with volume changes made elsewhere in Windows.
- It includes functional input-device controls wherever supported.
- Its glass effect is enabled by default on supported systems.
- Its fallback appearance remains usable on unsupported systems.
- Its themes, tray behavior, and settings work correctly.
- It handles common audio failures without crashing.
- The source, build instructions, and test results are delivered honestly and reproducibly.

**Final instruction:** Begin by presenting the proposed architecture, selected libraries, Windows API strategy, and known limitations. Then implement the application in small, testable increments. Build and test the actual Windows application wherever a suitable environment is available. Do not stop after producing a design, architecture document, or nonfunctional prototype.
