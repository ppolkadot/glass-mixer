# Glass Mixer

Control the volume of every app and audio device on Windows from one small
window in the system tray. Glass Mixer lets you set each app's volume, mute
and balance separately, switch your speakers or microphone, and keeps
everything in sync with changes you make anywhere else in Windows. On
Windows 11 it uses the native Acrylic or Mica glass look.

It is a native app written in Rust on the Windows Core Audio APIs: no web
view, no background service, no administrator rights and no network access.

[![Latest release](https://img.shields.io/github/v/release/ppolkadot/glass-mixer)](https://github.com/ppolkadot/glass-mixer/releases/latest)
[![CI](https://github.com/ppolkadot/glass-mixer/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ppolkadot/glass-mixer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/github/license/ppolkadot/glass-mixer)](LICENSE)

## Download

Get Glass Mixer from the
[latest release](https://github.com/ppolkadot/glass-mixer/releases/latest).
Each release has two downloads:

| Download | Choose it if | How to start |
| --- | --- | --- |
| `glass-mixer-<version>-setup.exe` | You want a normal installation: a Start menu entry, an uninstaller, and optional launch at sign-in. | Run the installer. It installs for your user account only, to `%LOCALAPPDATA%\Programs\Glass Mixer`, and needs no administrator rights. |
| `glass-mixer-<version>-portable.zip` | You don't want to install anything. | Unzip it anywhere and run `glass-mixer.exe`. Settings are still kept in `%APPDATA%\GlassMixer`. |

The downloads are not code-signed, so Windows SmartScreen may warn the first
time you run them. Choose **More info**, then **Run anyway** if you trust the
download. Recent release notes list the SHA-256 checksum of each download.

To build it yourself, see [Build from source](#build-from-source).

## Features

**Audio controls**

- Volume, mute and stereo balance for every app that plays sound.
- Volume, mute and balance for your default speakers and microphone, and
  switching which device is the default.
- A **Devices** view that controls every active output and input separately.
- **Presets** that save app volumes and mutes and apply them on request, or
  once whenever a saved app starts.
- Optional global keyboard shortcuts for mute, microphone mute, volume up and
  down, and showing the mixer. None are assigned until you choose them.

**Apps in the mixer**

- Each app shows its own icon, its name and whether it is Playing, Idle or
  Muted (in text as well as colour).
- An app with several audio streams appears as one row.
- Filter the list by name (Ctrl+F), hide idle apps, and pin apps so they keep
  their place even when they aren't running.
- Live level meters under the sliders (optional).

**Windows integration**

- Lives in the system tray: closing the window keeps it running there. The
  tray icon shows the volume and output device in its tooltip, and its menu
  has **Open Mixer**, **Mute** and **Exit**.
- Follows changes made with volume keys, Windows Settings or other apps.
- Notices devices being added, removed or switched, and reconnects to Windows
  audio after sleep or an audio service restart (not yet verified on real
  hardware; see [Project status](#project-status-and-documentation)).
- Optional launch at sign-in and start minimized; only one copy runs at a
  time.
- While hidden, warnings and errors appear as Windows notifications; Windows
  may hold them back, for example while you are presenting or in a
  full-screen app.

**Appearance**

- Light, dark or follow Windows; seven accent colours.
- Comfortable or compact layout.
- Acrylic or Mica glass on Windows 11 (see [Requirements](#requirements-and-compatibility)).

**Accessibility**

- Full keyboard navigation with a visible focus ring. Sliders respond to the
  arrow keys, Page Up/Down, Home/End and the mouse wheel.
- Every control is named for screen readers (UI Automation), and sliders
  report their values.
- Follows Windows' high-contrast, transparency and animation settings.

## Quick start

1. Install or unzip Glass Mixer, then start it from the Start menu or by
   running `glass-mixer.exe`. The mixer window opens and its icon appears in
   the system tray (it may be under the **^** overflow arrow on the taskbar).
2. Play something in an app, such as a video in your browser. The app appears
   under **Apps**.
3. Drag the app's slider to change its volume, or click the speaker button
   next to it to mute it. Click the app's name to show its balance and the
   **Pin** option.
4. Use the drop-downs at the top (output) and bottom (input) to change your
   default speakers or microphone. The **Devices** tab controls every device.
5. Close the window with **X**: Glass Mixer keeps running in the tray.
   Left-click the tray icon to bring the window back.
6. To quit, right-click the tray icon and choose **Exit**.

Appearance, startup, shortcut and behaviour options are under the
**Settings** tab.

### Command-line options

| Option | Effect |
| --- | --- |
| `--minimized` | Start with only the tray icon. |
| `--renderer=wgpu` | Use the opaque Direct3D 12 / Vulkan renderer instead of OpenGL. |
| `--exit` | Ask the copy running from the same folder to exit (used by the uninstaller). |

## Requirements and compatibility

- 64-bit Windows 10 version 1809 or later, or Windows 11. The installer
  enforces this minimum.
- A graphics driver with OpenGL, or with Direct3D 12 or Vulkan. Glass Mixer
  tries OpenGL first and falls back to an opaque window on Direct3D 12 or
  Vulkan.

The audio controls work the same on every supported version. The glass
effect is optional and depends on Windows:

| Windows version | Glass effect |
| --- | --- |
| Windows 11 22H2 or later | Acrylic (default) or Mica |
| Windows 11 21H2 | Mica only |
| Windows 10 | Not available; a matching solid background is used |

Glass also turns off when Windows transparency effects are off, when high
contrast is on, or when the fallback renderer is in use. Settings shows the
reason. Acrylic turns solid while the window isn't focused; that is Windows'
own behaviour.

Development and testing have been done on Windows 11. Windows 10 is supported
by the build but has not been tested; see [TESTING.md](docs/TESTING.md).

## Limitations

- **Only the default output's apps are listed.** As in Windows' own volume
  mixer, apps playing to another device don't appear, and Glass Mixer can't
  send an app to a different device. Change the default output, or route the
  app in Windows' own sound settings (on Windows 11: Settings > System >
  Sound > Volume mixer).
- **Some audio can't be controlled separately**: sounds Windows doesn't expose
  as their own session, such as exclusive-mode streams, some protected (DRM)
  playback and hardware mixers. Some Windows sounds share the System sounds row.
- **Default-device switching uses an undocumented Windows interface**
  (`IPolicyConfig`), because Windows has no public API for it. It is the same
  interface the Sound control panel uses. If it is unavailable, the device
  drop-downs are disabled and say why.
- **An app with several streams shows the loudest one**, and changing its
  volume sets all of its streams to the same level.
- **Balance needs a stereo device or app**; elsewhere it says it isn't
  available.
- **Sound formats, enhancements, spatial sound and mono audio are read-only.**
  The Devices view shows them and opens Windows Settings to change them.
- **Automatic presets apply only to apps that start at 100 % and unmuted.**
  Windows remembers app volumes, so an app it restores to a different level is
  left alone. Use **Apply now** on the preset instead.
- **Without graphics acceleration** (for example in some virtual machines or
  remote sessions), level meters are paused, because every frame would be
  drawn by the processor.
- **Launch at sign-in follows the last copy started.** If you run copies from
  several folders, the sign-in entry points to the most recent one.

## Troubleshooting

| Problem | What to do |
| --- | --- |
| An app is missing from the list | It has no audio session on the default output yet. Play something in it, or press the refresh button (↻) next to the filter. Apps playing to another device aren't listed. |
| Volumes look stuck after sleep or after plugging in a device | Glass Mixer rebuilds automatically. Press ↻ or **Settings > Refresh audio devices** to force it, and check the log file. |
| The device drop-downs are greyed out | Default-device switching isn't available on this system. Use Windows Settings > System > Sound. |
| There's no glass effect | The status under **Settings > Glass effect** explains why (Windows version, transparency effects, high contrast or the fallback renderer). |
| Level meters don't move | Check **Settings > Level meters**. Without graphics acceleration they are paused, and the setting says so. |
| A global shortcut says "Not active" | Windows or another app already uses that key combination. Choose another one, or free it and click **Retry failed shortcuts**. |
| Launch at sign-in doesn't work | It may be disabled in Task Manager > Startup apps; Settings shows this. Turn the option off and on to re-enable it. |
| "Glass Mixer couldn't start its window" | Neither OpenGL nor Direct3D 12 / Vulkan could start. Update the graphics driver. The log's `renderer:` line shows what was tried. |
| The window is too small | Use **Settings > Appearance > Use recommended window size**. |

If a problem persists, check the [log file](#where-are-settings-and-logs-stored)
and [report an issue](#contributing-and-reporting-issues).

## FAQ

### Does closing the window exit Glass Mixer?

No. By default it keeps running in the tray; use **Exit** in the tray menu to
quit. You can change this with **Settings > Keep running when closed**.

### Does it need administrator rights?

No. The installer installs for your user only, and the app runs with standard
rights. Apps running as administrator or protected by Windows may show
without an icon or with a generic name such as "Process 1234", because Glass
Mixer can't read their details.

### Does it access the network?

No. Glass Mixer has no networking code and none of its components make
network connections. It collects no telemetry or analytics.

### Where are settings and logs stored?

| What | Where |
| --- | --- |
| Settings | `%APPDATA%\GlassMixer\settings.json` |
| Log | `%LOCALAPPDATA%\GlassMixer\glass-mixer.log` (once it passes 1 MB, the next launch moves it to `glass-mixer.old.log` and starts a new one) |
| Launch at sign-in | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `GlassMixer` |

Glass Mixer never stores live volume levels: Windows remains the source of
truth. Only presets you save keep app volumes. If you edit `settings.json` by
hand, UTF-8 and UTF-16 are both accepted. Damaged contents are copied to
`settings.json.bad` before defaults can replace them. If the original cannot
be read, saving stays disabled until it can be loaded successfully; resolve
the file-access problem and restart Glass Mixer. A failed backup leaves the
original untouched and reports the problem. Uninstalling asks whether to remove
your settings.

### Can I go back to an older version?

Version 0.1.0 can't keep the pins, presets and shortcuts added later. When a
newer version first reads a 0.1.0 settings file, it saves an untouched copy as
`settings.v1.backup.json`. To return to 0.1.0, exit Glass Mixer and rename
that backup to `settings.json` first.

## Build from source

You need:

- [Rust](https://www.rust-lang.org/tools/install) (stable, MSVC toolchain:
  `rustup default stable-msvc`). The minimum version is 1.95.
- [Visual Studio](https://visualstudio.microsoft.com/downloads/) 2022 or later,
  or its Build Tools, with the "Desktop development with C++" workload and a
  Windows SDK.
- Optional, for the installer: [Inno Setup 6](https://jrsoftware.org/isinfo.php)
  (`winget install JRSoftware.InnoSetup`).

```powershell
cargo build --release   # target\release\glass-mixer.exe
cargo run               # debug build; also opens a console window
powershell -ExecutionPolicy Bypass -File scripts\package.ps1
```

`scripts\package.ps1` builds the release executable and writes the portable
zip, plus the installer when Inno Setup is installed, to `dist\`. Both include
`LICENSE` and `THIRD-PARTY-NOTICES.txt`. Code signing and the development
scripts are described in [scripts/README.md](scripts/README.md).

## Tests and diagnostics

```powershell
cargo test   # unit and UI-layout tests; no audio hardware needed
```

These tests need real audio devices, change real volumes, and restore them
when they finish:

```powershell
# Changes volumes by about 2 %, mostly on the idle System sounds session,
# and plays silence from a helper process.
cargo test --test audio_integration -- --ignored --test-threads=1
# Changes the volume and balance of every active device, then restores them.
cargo test --test devices_integration -- --ignored --test-threads=1
```

These diagnostic tools only read state and change nothing:

```powershell
cargo run --example audio_probe -- 30      # devices, apps and meters; watches for changes for 30 s
cargo run --example sound_properties_probe # device formats, effects and spatial-audio support
```

The release build logs at the info level, and debug builds at the debug level.
Both use the [log file](#where-are-settings-and-logs-stored).
[TESTING.md](docs/TESTING.md) lists what is covered automatically, recorded
results and the manual checklist.

## Project status and documentation

Glass Mixer is a Windows-only project, released as described in
[CHANGELOG.md](CHANGELOG.md). It has been developed and tested on Windows 11;
Windows 10, multi-device hot-plug, sleep and resume, and audio service
restarts are implemented but not yet verified on real hardware.

| Document | For | Contents |
| --- | --- | --- |
| [CHANGELOG.md](CHANGELOG.md) | Everyone | What changed in each version, upgrade notes and known limitations |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Developers | Threads, data flow and the Windows APIs used |
| [docs/TESTING.md](docs/TESTING.md) | Contributors | Automated tests, recorded results and the manual checklist |
| [docs/PROJECT_CONTEXT.md](docs/PROJECT_CONTEXT.md) | Maintainers | The full project record: original specification, decisions, gaps and known risks |
| [docs/SOUND_PROPERTIES_SPIKE.md](docs/SOUND_PROPERTIES_SPIKE.md) | Developers | Why sound-property controls open Windows Settings |
| [scripts/README.md](scripts/README.md) | Contributors | Packaging, signing and development test scripts |

## Contributing and reporting issues

Report bugs and request features in
[GitHub Issues](https://github.com/ppolkadot/glass-mixer/issues). For an audio
problem, please include:

- Your Windows version and the Glass Mixer version (shown under
  **Settings > About**).
- The audio device involved (for example USB headset or HDMI monitor) and the
  affected app.
- The steps that reproduce the problem.
- The relevant part of the log file. It contains device names and app paths,
  which can include your user name; remove anything private before sharing.

Before opening a pull request, run the checks that CI runs:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

## License

Glass Mixer is released under the [MIT License](LICENSE). The licenses of the
open-source components compiled into it are in `THIRD-PARTY-NOTICES.txt`,
included with each download from version 0.4.0.
