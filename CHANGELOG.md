# Changelog

## 0.2.0-beta.1 (unreleased, branch `experiments`)

A beta of the next feature release. It has been checked locally on one
Windows 11 machine (section "Verification" below). It has **not** had a hosted
CI run, is **not** merged into `main` and is **not** published.

### Added

- **Devices view:** independent volume, mute and stereo balance for every
  active output and input, not just the defaults. Duplicate device names get a
  numbered suffix; the full device ID is in the tooltip.
- **Sound properties (read-only):** per-device selected format, internal mix
  format, legacy system-effects state and dynamic spatial-audio availability.
  Changing format, enhancements, spatial audio or mono audio opens Windows
  Settings; Glass Mixer does not change these itself.
- **Global shortcuts (opt-in, none by default):** output mute, microphone mute,
  output volume up/down with a configurable step, and show/hide the mixer.
  Each shortcut is registered on its own: one that Windows rejects (for
  example because another app owns it) is shown as "Not active" with the
  reason, the others keep working, and **Retry failed shortcuts** tries again.
- **Pins:** keep an app's row in the mixer by executable or package identity.
  A pinned app that isn't running shows "Not running" with disabled controls.
- **Presets:** save the current app volumes and mutes under a name and apply
  them later. An optional automatic rule applies a preset once to a newly
  launched process (see "Known limitations").
- **Tray notifications** for warnings and errors while the window is hidden,
  with throttling and duplicate suppression; Windows notification settings
  apply. Can be turned off in Settings.
- **Renderer fallback:** if OpenGL cannot start, Glass Mixer opens an opaque
  window on Direct3D 12 or Vulkan instead of failing. The log names the
  renderer and graphics adapter in use.
- Settings > Appearance > **Use recommended window size** (480 × 720, also the
  new default size for first launch).

### Changed

- Navigation is a row of tabs: **Mixer, Devices, Presets, Settings**. The
  separate Settings button in the mixer header was removed.
- Balance and sound properties moved from the mixer header and footer into
  the Devices view; the mixer header links to them.
- Window positions are saved in physical screen pixels, so the window returns
  to the right monitor when monitors use different scaling. A position saved
  by 0.1.0 that could belong to more than one monitor is centered instead.
- Settings file schema version 4. Each value is read on its own, so one
  invalid entry no longer resets the others; unknown fields from newer
  versions are kept.
- System sounds stays visible when idle apps are hidden.
- Executable size grew from 8,442,880 to 15,789,056 bytes (about 8.4 MB to
  15.8 MB) because the Direct3D 12/Vulkan fallback renderer is now included.

### Fixed

- Garbled characters (for example "â€¦") in some interface text.
- Failed app-icon lookups can be retried with Refresh or when the app returns,
  and icon caches no longer grow without bound.
- Found and fixed while auditing this beta:
  - One invalid or conflicting global shortcut disabled every shortcut.
  - Pressing the show/hide shortcut while Glass Mixer was exiting could hang
    the exit, leaving the process running with no tray icon.
  - With the fallback renderer, exit intermittently deadlocked inside the
    OpenGL driver (the process stayed running after "shutdown complete").
  - At the minimum window width, the output percentage and Settings button
    were cut off; long names in offline pins, the shortcut editor and the
    settings path ran past the window edge; the sound-properties panel was
    squeezed into a small nested scroll area.
  - Several controls (device lists, filter box, preset name, volume step) had
    no name for screen readers, toggle descriptions such as the reason glass
    is off were not exposed, and a disabled **Save** gave no reason.
  - When two processes of one app started together, an automatic preset rule
    could skip the older one.

### Upgrading and downgrading

- **From 0.1.0:** settings are migrated automatically. The first time 0.2.0
  reads the old file it keeps an untouched copy as `settings.v1.backup.json`
  next to `settings.json` (`%APPDATA%\GlassMixer`).
- **Back to 0.1.0:** 0.1.0 cannot keep 0.2.0's pins, presets and shortcuts. On
  its next save it drops them and rewrites the file in the old format (this
  was reproduced). To go back cleanly, exit Glass Mixer and rename
  `settings.v1.backup.json` to `settings.json` before starting 0.1.0.
- The installer upgrades in place (same per-user install, no admin rights).
  Installing this beta over 0.1.0 has **not** been tested yet.

### Known limitations

- **Automatic preset rules may rarely fire.** A rule applies only when a newly
  launched process's audio starts at 100% and unmuted with no change seen
  first. Windows remembers per-app levels between launches, so an app whose
  level was ever changed may never qualify. Not verified on real hardware.
- **Keyboard focus is not scrolled into view** inside scrolling views, so a
  control reached with Tab can be off-screen until you scroll.
- The fallback renderer has only been exercised by forcing it in a debug
  build. On the test machine (GeForce GT 340, no Direct3D 12 or Vulkan
  driver) it uses Windows' software (WARP) adapter.
- Sound properties are read-only by design. Default-device switching still
  uses the undocumented `IPolicyConfig` interface.
- Builds are unsigned.

### Verification

Checked locally on Windows 11 Pro 25H2 (build 26200), NVIDIA GeForce GT 340,
one HDMI output and no microphone. Details are in `docs/TESTING.md`.

- **Automated:** formatting, clippy with warnings denied, the full test suite
  (unit tests, headless layout and accessibility tests, settings migration
  tests) and the release build, all with `--locked`.
- **Manual on this machine:** repeated isolated start/exit runs with both
  renderers, global-shortcut smoke and exit-stress tests, UI Automation and
  screenshots of every view at the default and minimum width.
- **Not verified:** a hosted CI run, Windows 10, microphones, several outputs,
  hot-plug, sleep/resume, audio-service restart, 125–200% scaling, light and
  high-contrast themes, Narrator speech and real Tab traversal, automatic
  preset rules with live apps, the installer upgrade from 0.1.0, and the
  fallback on a machine without OpenGL or over Remote Desktop.

## 0.1.0 (2026-10-15)

First release: master and per-app volume, mute and balance, default-device
switching, tray icon and menu, glass backdrop, themes, accent colours and
density, launch at login, single instance, and an installer and portable zip.
See `docs/PROJECT_CONTEXT.md` for the full record.
