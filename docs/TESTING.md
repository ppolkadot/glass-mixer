# Testing

## Fallback renderer exit deadlock (F-22) and 0.2.0-beta.1 checks (2026-10-16)

All runs below used the isolated debug profile on the section 4 machine
(Windows 11 build 26200, NVIDIA GeForce GT 340, driver 341.92 / 9.18.13.4192,
no Vulkan driver). Each start/exit run passed only if the process had really
terminated (checked by PID, not just the log), the exit code was 0 and the log
contained both "shutdown complete" and "exited normally".

**Symptom.** With `GLASS_MIXER_DEV_RENDERER=wgpu`, the process sometimes logged
"shutdown complete" but never "exited normally" and stayed running. Before the
fix: 5 of 12, 5 of 12, 4 of 8, 2 of 10 and 1 of 14 runs in separate sessions.

**Cause.** A read-only stack dump of every thread in the hung process (a
scratch `dbghelp` tool, not part of the repository) showed the same three
threads in 13 of 13 captured hangs:

1. The main thread inside `DestroyWindow` → `dwmapi` → `d3d11` →
   `FreeLibrary`/`LdrUnloadDll`, waiting for the loader lock while holding a
   `dwmapi` lock.
2. The "wgpu-hal WGL Instance Thread" (created by wgpu's OpenGL backend)
   exiting: `LdrShutdownThread` → the NVIDIA OpenGL driver's `DllMain`, which
   holds the loader lock and waits for a driver thread to finish.
3. That NVIDIA driver thread, inside its own `DestroyWindow`, blocked on the
   `dwmapi` lock.

egui-wgpu's default backends are `PRIMARY | GL`. On this machine wgpu then
chose its OpenGL adapter ("GeForce GT 340/PCIe/SSE2"), so the fallback was
really OpenGL again. With OpenGL excluded, the process has no WGL instance
thread and does not load the NVIDIA OpenGL driver (`nvoglv64.dll`); it renders
with Direct3D 12 on the WARP software adapter ("Microsoft Basic Render
Driver"), because this GPU has no Direct3D 12 or Vulkan driver.

**Fix.** The wgpu fallback is restricted to Direct3D 12 and Vulkan (`main.rs`,
`fallback_wgpu_options`); `WGPU_BACKEND` still overrides it for diagnostics.
The fallback only runs after OpenGL has failed, so wgpu's GL backend could not
help there anyway. `MixerApp::new` now logs a `renderer:` line with the
renderer, backend and adapter.

**Correction to an earlier note:** the "Vulkan only: 0 hangs" runs never
started ("Failed to create surface for any enabled backend"; no Vulkan driver).
Only the OpenGL-only (0 of 8) and Direct3D 12-only (0 of 8) runs were real.

**Results after the fix**, alternating fixed and old-backend runs in the same
session (old backends forced with `WGPU_BACKEND=vulkan,dx12,gl`):

| Session | Fixed fallback (DX12/Vulkan) | Old backends (incl. GL) | OpenGL (default renderer) |
| --- | --- | --- | --- |
| 1 (pre-version-bump build) | 40 of 40 clean, exit code 0 ×40 | 36 of 40 clean, 4 hung | 40 of 40 clean, exit code 0 ×40 |
| 2 (final build) | 30 of 30 clean, exit code 0 ×30 | 22 of 30 clean, 8 hung | 30 of 30 clean, exit code 0 ×30 |
| Total | **70 of 70** | 58 of 70 (12 hung) | **70 of 70** |

Every hung run had "shutdown complete" without "exited normally", and all 12
showed the stack signature above. The longest clean run took 5.7 s, for a
4-second auto-exit. With the fix, `scripts/verify-ui.ps1 -Renderer wgpu` passed
3 of 3 (it had failed intermittently before) and `scripts/lifecycle.ps1
-Renderer wgpu` passed 2 of 2.

**Release checks on the final tree:** `cargo fmt --check`; `cargo clippy --locked
--all-targets -- -D warnings`; `cargo test --locked` (71 library tests passed with
3 ignored, 2 binary, 1 source-encoding; the 3 ignored shortcut tests passed when
run explicitly; the 8 hardware audio tests were not run); `cargo build --release
--locked` (exe 15,789,056 bytes, version resource `0.2.0-beta.1`, numeric
0.2.0.0). The About section shows "Glass Mixer 0.2.0-beta.1". A copy of the
real 0.1.0 settings file in an isolated profile was migrated to version 4 with
theme, accent, density and window kept, and `settings.v1.backup.json` matched
the original byte for byte. `verify-ui.ps1` (OpenGL), `lifecycle.ps1` (OpenGL)
and both `hotkey-smoke.ps1` modes passed.

**Not verified:** a machine where OpenGL really fails. Replacing
`opengl32.dll` to simulate this makes the exe fail at load, because it imports
OpenGL statically. Also not verified: other GPUs and drivers, Remote Desktop,
a Vulkan-capable machine, release builds of the fallback (the renderer can only
be forced in debug builds), and installing over 0.1.0.

## Layout, accessibility, rules and CI fixes (audit tasks T-03 to T-07)

- **Headless UI tests** (`src/ui/test_support.rs`): egui runs against an in-memory
  screen with AccessKit enabled, and tests assert that no control is drawn past the
  window edge at the minimum width (Mixer, offline pins, Devices, sound properties,
  Presets, Settings with the shortcut editor), that every interactive control has an
  accessible name, that toggle descriptions are exposed, and that the Mixer view has
  one Settings route and no nested properties panel. The harness has its own tests
  proving it detects painter-only overflow, unnamed controls and overflowing widgets.
- **Rule and preset logic:** `StartTracker` (once per new process, arming, interventions,
  launch order), `plan_preset`, preset-name uniqueness, automatic-rule conflicts and
  `geometry_changed` are covered by unit tests. Engine COM calls that apply levels are
  not unit-testable and remain on the manual checklist.
- **Scripts:** `scripts/ui-automation.ps1` now also fails on an unnamed interactive control,
  a duplicate Settings route or a wrong view-selector order.
- **CI:** `.github/workflows/ci.yml` runs on pushes to `main`, `experiments` and
  `codex/**`, on every pull request, and by hand. It runs fmt, clippy (deny warnings),
  tests and a release build, all with `--locked`. The YAML was parsed with a real YAML
  parser and each step was run locally; a hosted run has **not** happened yet.
- **Known limitation:** egui does not scroll a newly focused control into view, so a
  control reached by keyboard inside a scroll area can stay off-screen (programmatic
  focus was observed not to scroll). Real Tab traversal and Narrator were not tested.
- **Exit hang with the forced wgpu renderer:** found during this work and fixed
  afterwards (F-22, see the section at the top of this file).

## Shortcut registration and shutdown fixes (audit tasks T-01 and T-02)

- **Unit tests:** per-binding validation (an invalid or duplicate binding no
  longer rejects the others), shortcut resolution (window shortcuts become a
  queued `HotkeyEvent`; volume step; stale messages ignored), and prompt service
  shutdown. The module also has a source guard that it does not call `window::`.
- **Ignored tests (register real global chords; run with
  `cargo test --lib platform::hotkeys -- --ignored --test-threads=1`):** a chord
  held by another "application" leaves the other bindings active and a retry
  recovers once it is released; shutdown completes while `Ctrl+Alt+F21` is being
  injected into the input queue.
- **End to end (isolated debug profile):** the old build hung on exit in 14 of 14
  runs when `Ctrl+Alt+F23` was pressed repeatedly across the exit deadline; the
  fixed build exited cleanly in 14 of 14. A Windows conflict plus an invalid chord
  leaves the remaining shortcut working, shows each reason, and "Retry failed
  shortcuts" activates the conflicting one after it is released.
- **Not verified:** other keyboard layouts, chords reserved by other software,
  and behavior during real logoff/shutdown.

## Final review verification (2026-10-16)

- **Ran and passed:** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test` (43 library tests passed, one ignored; binary and encoding
  tests passed). The isolated Ctrl+Alt+F23 smoke test passed, as did the glow
  UI Automation check and a forced-wgpu lifecycle run including minimized
  startup and clean shutdown. The smoke test with `-Minimized` also opened,
  hid, and reopened a hidden-start app. The final release package run produced
  the portable zip and Inno installer. The reviewer found no remaining Critical
  or Important issues in the final diff.
- **Ran and failed, corrected:** the first forced-wgpu lifecycle check sent
  WM_CLOSE before eframe had completed tray initialization. The script now
  waits for initialization and polls for the close-to-tray result; the rerun
  passed.
- **Not run:** live hardware preset application, screen-reader speech, a
  GL-less machine, code signing without a certificate, and hosted CI. See
  the manual checklist and Phase 4 limits below.

## Phase 4 verification (2026-10-15)

- **Ran and passed:** `cargo fmt`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test` (43 library tests passed, one ignored; binary and
  encoding tests passed), `cargo build`, `cargo build --release`, and PowerShell
  syntax checks for all scripts. The isolated lifecycle script verified launch,
  second-instance rejection, close-to-tray, `--minimized`, settings/geometry
  save and clean shutdown under both glow and forced opaque wgpu. The UI
  Automation script found Mixer, Devices, Settings and Presets under both
  renderers; 496×759 window captures showed no clipping in the default view.
  The live Ctrl+Alt+F23 smoke test hid and reopened the mixer while hidden.
  `scripts/package.ps1` built a portable zip and Inno installer with no signing
  certificate configured. The wgpu spike launched and rendered.
- **Ran and failed, corrected:** the first hotkey smoke fixture used a
  PowerShell UTF-8 byte order mark. After fixing that, its assertion still
  watched the tray helper window rather than the titled mixer window. The
  scripts now resolve the window by title and PID; the same smoke test passed.
- **Renderer cost:** the prior installed glow-only exe is 8,442,880 bytes;
  the branch release exe is 15,754,240 bytes (+86.6%). Three debug launches
  to first window took 1092/1251/1131 ms on glow and 1536/1134/1139 ms on
  wgpu, with a 100 ms polling resolution. These local measurements include
  OS cache and concurrent activity, and are not a release benchmark.
- **Not run:** Windows-hosted CI (workflow is local until pushed), code signing
  (no certificate configured), a GL-less machine, Windows 10, other DPI
  scales, and full live audio controls/preset rules. `PrintWindow` captures the client drawing
  but cannot establish DWM transparency under wgpu, so the fallback is opaque.

Scripts and optional signing are described in [scripts/README.md](../scripts/README.md).

## Phase 3 verification (2026-10-15)

- **Ran and passed:** formatting, clippy with warnings denied, and unit tests
  for pin identity/deduplication, disabled placeholders, preset sanitization,
  automatic-rule eligibility, hotkey parsing/conflict detection and lenient
  settings. The ignored hotkey test registered Ctrl+Alt+F23, confirmed that
  Windows rejected a second registration, then unregistered and re-registered
  the chord.
- **Ran and failed, corrected:** incomplete Phase 3 wiring first failed
  compilation; clippy then reported small style issues, which were fixed.
- **Not run:** live delivery for the four audio-changing hotkey actions,
  conflict with a separate application, a UI Automation pass over pinned
  placeholders, live preset application and automatic rule on a real process.
  Those remain on the manual checklist. The show/hide shortcut was later
  tested while hidden in Phase 4.

Manual additions: bind/clear all shortcuts, verify a conflict explains that
the previous binding remains active, hide the window and invoke each action;
pin an app, close it, inspect its disabled slider and screen reader state,
then restart it; filter pinned rows; save/apply/delete a preset and restore
levels; enable an automatic rule, change an app after it starts, and verify
the rule does not reapply. Repeat after restart and on mixed DPI.

## Phase 2 verification (2026-10-15)

- **Ran and passed:** formatting, clippy with warnings denied, all-target check,
  36 hardware-independent tests; read-only sound-properties probe on three
  outputs and three inputs; `cargo test --test devices_integration -- --ignored
  --test-threads=1 --nocapture`. The new hardware test restores master, mute
  and every channel even on panic. All six endpoints accepted independent
  volume changes; all three stereo outputs retained balance through subsequent
  master changes. Spec and code-quality reviews completed; findings corrected.
- **Ran and failed:** initial clippy module ordering (corrected); unavailable
  enhancements and mono properties in the probe (reported, no fake defaults).
- **Not run:** new-build GUI lifecycle/visual matrix, Focus Assist suppression,
  Windows 10, hot-plug, device default switching, audio-service restart and
  sleep/resume. Lifecycle verification follows with the development scripts.

Manual additions: exercise both density modes at 100/150/200%; expand both
device property panels and scroll to every button; adjust non-default devices;
unplug/reconnect them; change format/effects/spatial settings in Windows while
open and verify current readouts; check hidden notices, deduplication and Focus
Assist; verify unavailable mono points to Accessibility audio settings.
No format-changing integration test exists because no format setter was added.

## Automated

### Unit tests (no hardware)

```powershell
cargo test
```

| Area | Covered |
| --- | --- |
| Settings | Defaults (glass on), first run, repeated round trip, per-field fallback for invalid values, preservation of newer fields, corrupt file backed up to `.json.bad`, geometry sanitizing |
| Audio model | Session grouping (multi-session apps, loudest wins, muted only if all are), stable ordering, System sounds idle-filter exception, balance math round trip, percent rounding, filter matching, exe-name fallback, tray tooltip incl. 127-unit truncation |
| Engine | Coalescing of rapid slider commands per control, meter change threshold |
| Icons | BGRA→RGBA conversion, opaque bitmaps, straight-alpha detection, retry after a failed lookup, cache invalidation when apps return or DPI changes |
| Artwork | Icon rendering, ICO encoding |
| Backdrop | Support level per Windows build |
| UI and window | Sticky backdrop rejection, texture pruning, monitor-specific DPI position validation, source encoding regression scan |

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

### Phase 1 maintenance branch (2026-10-15)

- **Ran and passed:** `cargo fmt --check`, `cargo clippy --all-targets --
  -D warnings`, `cargo build`, `cargo test` (28 library tests, 1 binary
  geometry test and 1 source-encoding test), and the seven ignored audio
  integration tests with `--test-threads=1`. Those integration tests
  restored the volumes and mutes they changed.
- **Ran and failed, then corrected:** the new regression tests against the
  previous behavior, plus an initial formatting and clippy pass.
- **Not run:** launch/tray/shutdown of the branch build while the installed
  app owns the single-instance mutex; visual scale checks at 100%, 150% and
  200% in both densities; mixed-monitor DPI, Windows 10, microphone and
  multi-output checks.

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
- [ ] First launch with no saved geometry opens around 480×720; a deliberately saved size stays in place. Settings > Appearance > Use recommended window size resizes it.
- [ ] At 100 %, 150 % and 200 % scaling in both density modes, rows, sliders, footer and Settings controls remain visible and usable without clipping.
- [ ] Move between monitors with different DPI, exit and restart: the saved position stays on the correct monitor. With an ambiguous version-1 position, the app centers rather than guessing.
- [ ] Refuse a supported DWM backdrop (or simulate failure): glass status stays Unsupported without repeated DWM calls or flicker until the appearance setting, theme or Windows build changes.
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

### 0.2.0 beta additions (not yet run on real hardware unless noted)
- [ ] Install the beta over an installed 0.1.0: the app upgrades in place, settings keep their theme, accent and window, and `settings.v1.backup.json` appears next to `settings.json`. Uninstall leaves settings in place.
- [ ] Go back to 0.1.0 after renaming `settings.v1.backup.json` to `settings.json`: 0.1.0 starts with its old settings.
- [ ] Devices view with two outputs and a microphone: each device's volume, mute and balance change only that device; unplug and replug one while the view is open.
- [ ] Global shortcuts: bind all five, hide the window and use each; bind a chord another app owns and check "Not active" plus Retry after that app releases it; restart and confirm the same state.
- [ ] Presets: save, apply, delete; enable an automatic rule, launch the app fresh (including one whose level Windows remembered) and note whether the rule applied.
- [ ] Keyboard only: Tab through Mixer, Devices (expanded), Presets and Settings; note any focused control that stays off-screen.
- [ ] On a machine or VM where OpenGL fails (or over Remote Desktop): the app opens an opaque window, the log has a `renderer: wgpu fallback, Dx12 …` or `Vulkan …` line, Settings explains the solid background, and exit completes.
- [ ] 125%, 150% and 200% scaling, light theme and high contrast: no clipped controls in any view.
