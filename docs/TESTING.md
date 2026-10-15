# Testing

## Automated

### Unit tests (no hardware)

```powershell
cargo test
```

| Area | Covered |
| --- | --- |
| Settings | Defaults (glass on), first run, round trip, per-field fallback for unknown values, corrupt file backed up to `.json.bad`, window geometry sanitizing, atomic write |
| Audio model | Session grouping (multi-session apps, loudest wins, muted only if all are), stable ordering, balance math round trip, percent rounding, filter matching, exe-name fallback, tray tooltip incl. 127-unit truncation |
| Engine | Coalescing of rapid slider commands per control, meter change threshold |
| Icons | BGRA→RGBA conversion, opaque bitmaps, straight-alpha detection |
| Artwork | Icon rendering, ICO encoding |
| Backdrop | Support level per Windows build |

### Integration tests (real audio devices)

```powershell
cargo test --test audio_integration -- --ignored --test-threads=1 --nocapture
```

These need an active output device. Every change is restored. Volume changes
are ±2 %, mostly on the idle System sounds session; session tests use a helper
process that renders silence.

| Test | Spec item |
| --- | --- |
| `master_volume_set_and_restore` | 1. Master volume |
| `external_master_volume_change_is_reflected` | 3. Sync with external changes (prints the latency) |
| `system_sounds_volume_and_mute` | 2. Per-app volume and mute, without touching the device volume |
| `new_sessions_appear_and_exited_ones_disappear` | 4–5. New and terminated sessions (prints the latencies) |
| `missing_app_reports_error_instead_of_success` | 12/15. Errors are reported, never shown as success |
| `refresh_rebuilds_and_recovers` | 13/15. Full rebuild and recovery |

### Results on the development machine (2026-10-15)

- **Machine:** Windows 11 25H2 (build 26200), NVIDIA HDMI output, no
  input device.
- **Unit tests:** 22 of 22 passed.
- **Integration tests:** 7 of 7 passed (6 tests plus the helper).
  - External change reflected: about 20 ms.
  - New session detected: about 20 ms.
  - Session removed after process exit: about 20 ms.
- **UI Automation** (release and debug builds):
  - Every control exposes a name and role.
  - Invoking "Mute System sounds" muted the Windows session, and invoking
    it again unmuted it.
  - Setting the System sounds slider to 90 changed the session to 90 %,
    and it was then restored.
- **Lifecycle** (release build):
  - The close button hides the window and the process keeps running.
  - A second launch exits and restores the first instance's window.
  - `--minimized` starts hidden, and settings are saved.
- **Resources** (release build):
  - CPU over 10 s: 0 ms while hidden, about 78 ms while visible.
  - Memory: about 44 MB private (mostly the OpenGL driver), 8 MB exe.
  - Soak test (15 rounds of session create/destroy plus volume and mute
    changes): private memory flat after warm-up (+0.3 MB over 10 rounds),
    handles flat at about 504.
- **Not tested here:**
  - Windows 10 (no Windows 10 machine was available).
  - Input-device controls (this machine has no microphone).
  - Device switching between several outputs (only one output device).
  - Physical hot-plug and sleep/resume.

  These are covered by the manual checklist below.

## Manual Windows checklist

Run on a real desktop session, ideally on both Windows 11 and Windows 10,
with at least two output devices and one microphone.

### Master and devices
- [ ] Drag the master slider: the volume changes immediately and the Windows volume flyout matches.
- [ ] Press the keyboard volume keys or a headset dial: the slider and % follow within a moment.
- [ ] Mute and unmute from the mixer, then from the Windows flyout: both stay in sync, and the tray icon shows a slash when muted.
- [ ] Switch the output device from the drop-down: audio moves, a notice appears, and the app list shows the new device's sessions.
- [ ] Change the default device in Windows Settings: the mixer follows.
- [ ] Unplug the active output (USB/Bluetooth): no crash; the mixer moves to the new default or shows "No output device". Plug it back in.
- [ ] Microphone: switch input, change volume, mute; Windows Settings > Sound > Input matches.
- [ ] With no microphone attached: the footer says so, and there are no errors.

### Per-app mixer
- [ ] Start Spotify or a browser playing audio: it appears within about a second with its own icon and "Playing".
- [ ] Two apps playing: changing one doesn't change the other or the master.
- [ ] Mute an app: it goes silent, the row shows "Muted", and its icon dims.
- [ ] Quit the app: its row disappears.
- [ ] A browser with several tabs playing appears as a single row ("· N streams").
- [ ] Store app (e.g. Media Player): correct name and icon.
- [ ] Expand a row (click its name): the balance slider pans the app left and right; double-click recentres it.
- [ ] Filter (Ctrl+F) narrows the list; Esc clears it.
- [ ] An app with no extractable icon gets a coloured letter tile.

### Tray and lifecycle
- [ ] Left-click the tray icon: the mixer opens and is focused.
- [ ] Right-click: Open Mixer, Mute (checked state matches) and Exit all work, with the window hidden too.
- [ ] Hovering the tray icon shows the volume and device name.
- [ ] The close button hides to the tray (default). With "Keep running when closed" off, it exits.
- [ ] "Minimize to tray" on: minimizing removes the taskbar button.
- [ ] Launching the exe again brings the existing window forward; there is never a second tray icon.
- [ ] Launch at login: enable, sign out and in; the app starts in the tray. Disable it in Task Manager; Settings shows the note.
- [ ] Exit from the tray: the process ends, and the tray icon disappears without needing a hover.

### Appearance and accessibility
- [ ] First launch on Windows 11: glass (Acrylic) is on, and text is readable over a busy wallpaper or window.
- [ ] Switch to Mica, then glass off: a solid background with the same layout.
- [ ] Light, Dark and System themes, and changing the Windows theme while open: the app follows within about 2 s.
- [ ] Windows Settings > Accessibility > Transparency effects off: glass turns off and Settings explains why.
- [ ] High contrast theme on: opaque high-contrast palette; everything is still distinguishable without colour.
- [ ] Animation effects off: no animated transitions.
- [ ] 100 %, 150 % and 200 % display scaling, and moving between monitors with different DPI: crisp text and icons.
- [ ] Keyboard only: Tab through every control with a visible focus ring; arrows, Page Up/Down and Home/End adjust sliders; Space/Enter press buttons.
- [ ] Narrator: controls are announced with names; sliders announce their values and can be adjusted.
- [ ] Resize the window to its minimum and to large sizes: no overlapping or clipped controls.

### Reliability
- [ ] Sleep and wake the PC: volumes and the app list are correct afterwards.
- [ ] Restart the Windows Audio service (`net stop audiosrv`, then `net start audiosrv`, as admin): the mixer shows "unavailable", then recovers by itself.
- [ ] Leave it running for 24 h: memory and handle count in Task Manager stay flat.
- [ ] Delete or corrupt `%APPDATA%\GlassMixer\settings.json`: the app starts with defaults and a notice, and keeps the bad file as `.json.bad`.
