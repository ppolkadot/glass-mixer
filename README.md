# Glass Mixer

A lightweight, native Windows audio mixer that lives in the system tray. It
controls active output and input devices and apps with sessions on the
default output, with a Windows 11 glass look.

Written in Rust and built on the Windows Core Audio APIs. It doesn't use a
web view, needs no administrator rights for normal use, and makes no
network access.

## Features

- **Output and input devices:** volume, mute and stereo balance, plus
  default-device switching. The Devices view controls other active endpoints
  independently. Duplicate names have a numbered suffix and full ID tooltip.
- **Sound properties:** expandable device sections show the selected format
  and internal mix format. Format, enhancements, spatial and mono controls
  open Windows Settings. See [probe findings](docs/SOUND_PROPERTIES_SPIKE.md).
- **Tray notifications:** warnings and errors while hidden, with throttling,
  duplicate suppression and a Settings toggle. Windows notification policy applies.
- **Optional global shortcuts:** assign chords for output/microphone mute,
  output volume up/down and showing the mixer. No chord is assigned by default.
- **Pins and presets:** pin apps by executable or package identity; an offline
  pin says "Not running" and exposes no volume. Presets save app volumes and
  mutes only when you press Save, and apply only when you press Apply or enable
  a visible once-per-new-process rule. Automatic rules skip already-running
  processes and observed user changes.
- **Per-app mixer:** independent volume, mute and balance for every app
  with a Windows audio session.
  - Each row shows the app's own icon, its name, and whether it is
    Playing, Idle or Muted. Status is shown as text as well as colour.
  - An app's sessions are grouped into one row. Apps are matched by
    executable path or package identity (AUMID), not by display name.
  - Live peak meters (optional).
  - Filter by name (Ctrl+F). Idle apps can be hidden; System sounds stays visible.
- **Always in sync with Windows:** changes made with the volume keys,
  Windows Settings, hardware buttons or other apps appear in the mixer
  within milliseconds. Windows is the only source of truth; Glass Mixer
  never stores or restores volume levels automatically except when you
  explicitly save and enable a preset rule.
- **Event-driven:** device add/remove, default-device changes, apps starting
  and quitting, sleep/resume, and Windows Audio service restarts are all
  detected and recovered from automatically.
- **System tray:**
  - Click the tray icon to open the mixer.
  - Right-click for a menu with Open Mixer, Mute and Exit.
  - The tooltip shows the volume and output device.
  - The close button keeps the app running in the tray, and only one
    copy runs at a time.
- **Appearance:**
  - Acrylic or Mica glass backdrop, on by default on Windows 11.
  - Light, dark, or follow Windows.
  - Seven accent colours.
  - Comfortable or compact density.
  - A 480×720 starting window and a Settings action to return to that size.
- **Accessibility:**
  - Full keyboard navigation with a visible focus ring.
  - Sliders: arrow keys (1 %), Shift+arrows or Page Up/Down (larger
    steps), Home/End, and the mouse wheel.
  - Screen-reader support through UI Automation. Every control has a
    name, and sliders expose their value.
  - Respects Windows' reduced-motion, transparency and high-contrast
    settings.
- **Launch at login** is optional. It registers per user and needs no
  admin rights.

## Requirements

- Windows 10 1809 or later, or Windows 11 (the primary target).
- A graphics driver supporting OpenGL or a wgpu backend. Glass Mixer tries
  OpenGL first, then an opaque wgpu window if OpenGL initialization fails.

## Install

- **Installer:** run `glass-mixer-<version>-setup.exe`. It installs per user
  to `%LOCALAPPDATA%\Programs\Glass Mixer` and needs no admin rights.
- **Portable:** unzip `glass-mixer-<version>-portable.zip` and run
  `glass-mixer.exe`.

Settings are stored in `%APPDATA%\GlassMixer\settings.json`. The log is
`%LOCALAPPDATA%\GlassMixer\glass-mixer.log`.

Command line: `glass-mixer.exe --minimized` starts with only the tray icon.

## Build from source

Prerequisites:

- Rust stable with the MSVC toolchain (`rustup default stable-msvc`).
- Visual Studio 2022 or later with the "Desktop development with C++"
  workload and a Windows SDK.

```powershell
cargo build --release                 # target\release\glass-mixer.exe
cargo run                             # debug build with a console for logs
powershell -ExecutionPolicy Bypass -File scripts\package.ps1   # dist\ zip (+ installer)
```

`scripts\package.ps1` always produces a portable zip. It also builds the
installer when Inno Setup 6 is installed
(`winget install JRSoftware.InnoSetup`). Optional signing and the development
lifecycle, UI Automation and window-capture scripts are documented in
[scripts/README.md](scripts/README.md).

## Tests

```powershell
cargo test                                                         # unit tests (no hardware)
cargo test --test audio_integration -- --ignored --test-threads=1  # real audio devices
```

The integration tests change real volumes by ±2 %, mostly on the idle System
sounds session, and always restore them. See [docs/TESTING.md](docs/TESTING.md)
for what is covered automatically and the manual Windows checklist.

Diagnostics: `cargo run --example audio_probe -- 30` prints what the audio
engine sees and watches for changes for 30 seconds. It changes nothing.

## Limitations

- **Default-device switching** uses the undocumented `IPolicyConfig`
  interface, as EarTrumpet, SoundSwitch and the Sound control panel do.
  Windows has no public API for this. If the interface is missing, the
  device menu is disabled with an explanation.
- **Session scope:** only sessions on the current default output device
  are listed, as in Windows' own mixer. Per-app output routing isn't
  supported.
- **What can't be controlled:** sounds Windows doesn't expose as separate
  sessions, such as exclusive-mode streams, some protected or DRM paths,
  and hardware mixers. Some system sounds share the System sounds session.
- **Balance** needs a stereo device or session. Elsewhere it is shown as
  unavailable rather than as a value. Double-click a balance slider to center it.
- **Glass:**
  - Acrylic and Mica need Windows 11 22H2 or later.
  - Windows 11 21H2 gets Mica only.
  - Windows 10 uses a matching solid background.
  - Glass also turns off automatically when Windows transparency effects
    or high contrast are on.
- **Notifications** appear in the app while it is visible; while hidden,
  warnings and errors use tray notifications subject to Windows quiet policy.

## Project documentation

- [docs/PROJECT_CONTEXT.md](docs/PROJECT_CONTEXT.md): the full project record. It covers
  the original specification, development history, features, decisions,
  test results, gaps and known risks.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): design and the Windows API strategy.
- [docs/TESTING.md](docs/TESTING.md): tests, results and the manual checklist.

## License

MIT. See [LICENSE](LICENSE).

## Troubleshooting

| Symptom | What to do |
| --- | --- |
| "Glass Mixer couldn't start its window" | Both OpenGL and the opaque wgpu fallback failed. Update the GPU driver or run it locally. |
| An app is missing from the list | It has no audio session on the default output yet. Play something in it, or press ↻ Refresh. Apps playing to another device aren't listed. |
| The device menu is greyed out | Default switching isn't available on this system. Use Windows Settings > System > Sound. |
| No glass effect | Check Settings > Glass effect. The status line explains why it's off (Windows version, transparency effects, high contrast). |
| Launch at login doesn't work | It may be disabled in Task Manager > Startup apps; Settings shows this. Turn the toggle off and on to re-enable it. |
| Volumes look stuck after sleep | The app rebuilds automatically after resume. Press ↻ or Settings > Refresh audio devices to force it, and check the log file. |
| The window is too small | Use Settings > Appearance > Use recommended window size. Existing saved sizes are preserved. |
