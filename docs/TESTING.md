# Testing

> **Commit hashes.** On 2026-11-06 the repository history was rewritten to remove
> co-author lines from commit messages. File contents did not change, but every commit
> has a new hash, and the tags `v0.1.0`, `v0.3.0` and `v0.4.0` point to the new commits.
> The documents cite the new hashes. GitHub Actions runs from before then still show the
> old ones: `107a305` is now `ea9c816`, `e47b328` is `209e563`, `518cd33` is `2561aee`
> and `41f656d` is `86fdb9b`.

## 0.6.0 release verification (2026-11-13, `main`)

- **Gate**, with every step's exit status checked:
  - `cargo fmt --check`: passed.
  - `cargo clippy --locked --all-targets -- -D warnings`: passed (0 warnings
    across library, binary, integration tests, and examples).
  - `cargo test --locked`: **222** library tests passed (3 ignored), **5**
    binary tests passed, **1** source-encoding test passed (`tests/source_encoding.rs`),
    and the **13** real-hardware integration tests (`12` in `tests/audio_integration.rs`,
    `1` in `tests/devices_integration.rs`) remained ignored as expected without
    --ignored.
  - New and expanded 0.6.0 tests cover:
    - Font & typography: Polaris Bold weight chain (`platform::fonts`), 9 semantic
      `Role` mappings, `polaris_tracking` interpolation/clamping, and zeroed
      tracking when disabled (`ui::typography`), plus dynamic `percent_width`
      measurement (`ui::widgets`).
    - Theme & widgets: WCAG contrast thresholds for all palettes, reduced-motion
      zero duration helper (`ui::theme`), semantic button kinds/names/focus,
      segmented pill tabs, toggle row, disclosure chevron, combo icon, search
      field, check row, icon glyph coverage, and 2.5 pt focus rings (`ui::widgets`).
    - Views & toasts: toast deduplication, 3-toast cap, and visibility-frozen
      expiry (`ui::toasts`), Mixer grouped cards/search/details/empty state,
      Devices disclosure/controls, Presets cards/focus scrolling, Settings
      minimum-width layout/shortcuts/backup/diagnostics, and standalone
      `HotkeyEditor` (`ui::hotkey_editor`).
    - Platform & performance: immediate `WM_SETTINGCHANGE`/`WM_DPICHANGED`/
      `WM_DWMCOLORIZATIONCOLORCHANGED` wake signaling (`platform::window`),
      theme/DPI-aware tray menu and derived `&` mnemonics (`platform::tray_menu`),
      `DWMWA_BORDER_COLOR` calculation and retry keys (`platform::backdrop`),
      unchanged tray menu state skip (`app`), and `settings_seen` diffing.
    - Reliability & hardening: non-fatal reduction errors and Solo preservation
      across rebuilds (`audio::engine`), early return on `LoadDevices` when
      engine unavailable, non-caching of transient `Route::Unknown`, bounded UTF-16
      slice in `file_description`, `SetStreamVolume` coalescing, engine thread
      panic safety, `mic_ducking.targets` normalization and cap (`settings`),
      `1e-6` float tolerance in `import_notes` (`transfer`), failed-save
      `dirty_since` preservation and rate-limited toast, maximized geometry
      preservation, `--minimized` second-instance non-activation, unfocused visible
      window raising, and skipping startup path repair in debug builds / dev
      profiles (`main`).
- **Environment limitations (not run in this non-interactive sandbox):**
  - Live desktop automation scripts (`scripts/verify-ui.ps1`,
    `scripts/lifecycle.ps1`, `scripts/hotkey-smoke.ps1`) require an interactive
    desktop foreground window (`GetForegroundWindow() != 0`), which is unavailable
    in this sandbox session.
  - The 13 `#[ignore]` hardware audio integration tests (`tests/audio_integration.rs`
    and `tests/devices_integration.rs`) require physical Windows audio endpoints.
  - Inno Setup 6 (`ISCC.exe`) is not installed in this environment, so local
    packaging produces and verifies the release executable, portable ZIP, and
    `SHA256SUMS.txt` without `glass-mixer-0.6.0-setup.exe` (`scripts/installer-smoke.ps1`
    was not run locally; `scripts/check-version.ps1` and `scripts/installer-smoke.ps1`
    were checked for PowerShell syntax and clean behavior in WP-K3/WP-K6).

## Stage 6: release readiness (2026-11-06, `experiments`)

Machine: Windows 11 build 26200, HDMI output active, one monitor at 100 %. See
[VALIDATION_MATRIX.md](VALIDATION_MATRIX.md) for what this machine does and does
not cover.

- **Gate**, with every step's exit status checked: `cargo fmt --check`, `cargo
  clippy --locked --all-targets -- -D warnings`, `cargo test --locked` (194 library
  tests passed, 3 ignored, plus the binary and source-encoding tests), a release
  build (23,137,280 bytes), the 12 real-device integration tests, and
  `scripts/lifecycle.ps1` with OpenGL (hidden CPU 16 ms over 1 s, 47.7 MB private)
  and `-Renderer wgpu` (109 ms on WARP, 285.8 MB private).
- **Packaging:** `scripts/package.ps1` ran end to end: version check, release build,
  notices, portable ZIP, installer, checksums, and `verify-package.ps1`, which
  passed. Both packages are reported as **unsigned**: no certificate exists, so the
  signing path has not been exercised. Negative tests showed the verifiers fail on a
  wrong version, a missing file, a tampered checksum and `-RequireSigned` without a
  signature.
- **Installer lifecycle** (`scripts/installer-smoke.ps1`): a variant installer with
  its own identity and the debug build, so the real installation could not be touched.
  The final run passed all 32 checks: fresh install, upgrade over a running copy
  (closed cleanly, one uninstall entry, new files), uninstall of a running copy
  (asked to exit and did, everything removed, nothing left running), and the real
  installation (launch-at-login, uninstall entry, executable hash, files, shortcuts,
  processes) unchanged. An earlier run failed one check: the upgrade closed the copy
  by terminating it. The installer now runs `--exit` from the installed folder before
  replacing files, and the check passes. Not tested: upgrading from a 0.1.0 to 0.3.0
  installation (those versions do not know `--exit`), and the installer built from the
  release executable (the smoke test uses the debug build).
- **Performance, 0.4.0 (`86fdb9b`) against this branch**, release builds in scratch
  copies with their own settings folders and single-instance name, three runs each,
  10 s of CPU after 4 s of settling:

  | | 0.4.0 | 0.5.0 candidate |
  | --- | --- | --- |
  | Executable | 15.12 MB | 22.07 MB (the Polaris typeface is about 7 MB of it) |
  | CPU, window visible, meters on (median) | 47 ms | 62 ms |
  | CPU, hidden (median) | 0 ms | 0 ms |
  | Working set, visible (median) | 68 MB | 69.7 MB |
  | Working set, hidden (median) | 67.3 MB | 69.2 MB |
  | Private memory, visible (median) | 50.5 MB | 50.6 MB |

  The CPU difference is inside the spread of the runs (0 to 78 ms over 10 s). Time to
  the first window handle was 552 ms and 389 ms on the cold first runs and about
  30 ms afterwards; that is the first top-level window, not the first painted frame,
  so it is not a startup benchmark. No clean-build times were measured.
- **Not run:** the GitHub Actions workflows (`ci.yml`, `release.yml`) have not run on
  GitHub; Windows 10; other hardware; the matrix in [VALIDATION_MATRIX.md](VALIDATION_MATRIX.md).

## Stage 5, part 2: app names, ordering, and settings export and import (2026-11-06, `experiments`)

Machine: Windows 11 build 26200, with only the HDMI output active.

- Gate, with every step's exit status checked: `cargo fmt --check`, `cargo clippy
  --locked --all-targets -- -D warnings`, `cargo test --locked` (194 library tests
  passed, 3 ignored; 3 binary tests and the source encoding test passed; 12
  hardware tests ignored), a release build (23,137,280 bytes), the 12 real-device
  integration tests, and `scripts/lifecycle.ps1` with OpenGL (hidden CPU 31 ms
  over 1 s) and `-Renderer wgpu` (328 ms on WARP, within its usual spread). The
  first full run failed once, on a clippy style rule in one of the new tests; it
  was fixed and the whole gate rerun.
- New tests (17): custom names being trimmed, unique per app, limited to stable
  apps and bounded; setting, changing and clearing a name and whether anything
  changed; moving an item one place and refusing to move past the ends; the
  rows showing the custom name, the rename and order controls being named and
  disabled at the ends of the list, and fitting at minimum width; typing a name
  and pressing Enter renaming the app, and Reset restoring the reported name;
  the cycle-order buttons for connected and missing outputs; and in
  `transfer.rs`: an export being versioned and free of the window position, an
  export importing back unchanged, plain settings files and UTF-8 BOM and UTF-16
  files being accepted, ten kinds of file being refused with a reason (empty,
  not JSON, an array, no marker, no settings, no version, bad format number, not
  text, over 1 MB), newer export formats and settings versions being refused,
  unreadable values and list entries being reported rather than replaced
  silently, the window position of this PC being kept, only the automatic
  backups being pruned, and a path that is missing, a folder or too large being
  refused before it is read; and the Backup section's buttons being named,
  fitting, and each producing its outcome (Cancel discarding a checked import).
- End to end with the real app in an isolated profile, driven by UI Automation and
  guarded real keystrokes (the script refuses to type unless the Glass Mixer
  window is in front and the path field has keyboard focus): Export wrote a
  versioned file without a window position; a junk file was refused with
  "isn't valid JSON" and nothing was offered for import; a good file was checked
  and described ("1 renamed app", and a note that `hotkey_step` wasn't valid); on
  confirmation a backup of the previous settings was written first, the saved
  `settings.json` then held the imported theme, wheel step and renamed app, the
  invalid `hotkey_step` 99 had become the default 5, and the import panel closed.
- Not run: typing a custom name in the real window (the UI test types and presses
  Enter in the real widget; the end-to-end run covered export and import only),
  importing a file made on another PC, the new controls at 125, 150 and 200 %
  scaling, and Windows 10. Per-app routing was not exercised because only one
  output was active.

## Stage 5, part 1: Polaris typeface and diagnostic report (2026-11-06, `experiments`)

The interface typeface (Salted Flutters Polaris Regular and Semibold, embedded) and
the diagnostic report (Settings > About). Machine: Windows 11 build 26200; only the
HDMI output was active, because the headphones and microphone were unplugged that
day.

- Gate, with every step's exit status checked: `cargo fmt
  --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test
  --locked` (177 library tests passed, 3 ignored; 3 binary tests and the source
  encoding test passed; 12 hardware tests ignored), a release build (22,979,072
  bytes, 7.24 MB more than the 15,737,856 before the fonts), the 12 real-device
  integration tests, and `scripts/lifecycle.ps1` with OpenGL (hidden CPU 0 ms
  over 1 s) and `-Renderer wgpu` (188 ms on WARP, within its usual spread).
- The first full run of this gate failed twice, and both were fixed: a `rustfmt`
  difference, and `focused_offscreen_controls_scroll_into_the_settings_viewport`.
  The latter judged the first Shift+Tab while the previous step's mouse-wheel
  smooth-scroll was still animating (the harness advances only 10 ms per pass),
  so the control near the page end looked off-screen. The test now lets that
  animation finish first. Its Tab and Shift+Tab budgets rose from 45 to 90 key
  presses because the page has grown to dozens more controls.
- Font checks with fontTools on the files as bundled: Regular and Semibold are CFF
  OpenType, `fsType` 0 (no embedding restriction), and carry the metadata line
  "Salted Flutters Polaris - Created by ppoolar". They cover Portuguese accents,
  `R$ € £ ¥ ¢`, typographic punctuation, arrows and `✓`; the only glyph missing
  from the probe set was `✕` (U+2715), which the interface doesn't draw. Average
  advance 0.544 em for Regular against 0.540 for Segoe UI, so layouts barely move;
  line height is 1.19 em against 1.33. Finding for the font's author: in
  `SaltedFluttersPolaris-Bold.otf`, `usWeightClass` is 600, the same as Semibold
  (it should be 700). It has no effect on the embedded use here.
- New tests: the family chains (Polaris, then Segoe UI, then egui's built-in
  fonts; the icon font separate; the Windows typeface leaving Polaris out;
  missing system fonts), the embedded files being OpenType and keeping the notice
  and the unmodified license, every glyph of a Portuguese and symbols sample being
  covered in both the regular and semibold chains, and the two typefaces really
  laying out differently (so Polaris is actually used); the setting's default and
  its recovery from a bad value; in `diagnostics.rs`: UTC dates (leap days
  included), redaction (longest name first, case, plain and escaped paths, short
  and placeholder-like names left alone), a report holding the facts and none of
  the private names, unavailable services being reported, the log's tail, and
  keeping the newest 10 reports without touching other files; and a click on each
  new button producing its outcome. The existing layout, naming and keyboard tests
  all pass under Polaris.
- Visual: the real mixer window captured with Polaris (tabs, semibold app names,
  regular status text, percentages) shows no clipping; a neutral preview window
  (`examples/font_preview.rs`, no user data) shows the glyph sample in both
  typefaces.
- End to end with the real app in an isolated profile, driven by UI Automation:
  Settings > Save diagnostic report wrote a 2,122-character report with every
  section, and Copy report put the same sections on the clipboard. Neither
  contained the names of the two apps and the output device on this PC, nor the
  user folder.
- Not run: the typeface at 125, 150 and 200 % Windows scaling (the harness and the
  captures are at 100 %), Windows 10, and the installer and portable ZIP with the
  new `fonts\LICENSE.txt` (the script and installer entries were edited but
  `scripts/package.ps1` was not run).

## Stability check before Stage 5 (2026-11-06, merge `107f2cb`)

`experiments` held the 0.5.0 work (`a488137`) and an in-progress merge of the
published 0.4.0 release; this checks the merged tree.

- 163 library tests passed three times in a row; the 12 real-device tests passed
  twice in a row; release build and lifecycle (both renderers) passed.
- Today the per-app routing test passed without exercising anything: it needs two
  active outputs and only HDMI was present. It ran for real on 2026-11-05, when
  the headphones were connected.
- Soak of the debug app for 10 s per sample with a silent helper session, in an
  isolated profile: with no automation, a background-lowering rule, microphone
  ducking, or both. Medians were about the same (roughly 60 to 160 ms of CPU
  per 10 s); single samples of several seconds appeared in every mode, including
  ones that poll nothing, and also on the unmodified build earlier, so they are
  machine noise. No warnings, errors or panics in the log, and the settings were
  intact afterwards.
- New real-device test `lowered_levels_come_back_when_the_engine_shuts_down_or_rebuilds`:
  with another app lowered, a rebuild ends Solo and restores its level, and a
  shutdown leaves it restored for a fresh engine. Removing the restore in
  `Engine::drop` makes it fail (checked, then reverted).

## Stages 3 and 4: controls, profiles, routing and ducking (2026-11-05, branch `stage3-4/features`)

Pinned outputs and cycling, the wheel step, app and profile shortcuts, per-stream
controls, Solo, profiles, per-app output devices (with sessions on every
output), background lowering and microphone ducking. Run on the machine in
`PROJECT_CONTEXT.md` section 4 (Windows 11 build 26200; outputs: headphones and
HDMI; one microphone).

- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`:
  passed.
- `cargo test --locked`: 157 library tests passed (31 new), 3 ignored; 3 binary
  tests and 1 source-encoding test passed; 12 hardware tests ignored. New tests
  cover cycling with no, one or several pinned outputs (pin order, unpinned and
  disconnected devices, devices returning), the settings fields with bad values,
  the attenuation ledger (written once, undone, a user change kept, mute,
  combined reductions, ended sessions), app and profile shortcuts resolving to
  their commands, one shortcut per app, the unchanged saved format of a binding
  without a target, the editor's slots, the per-app routing device-ID format and
  vtable slots (25, 26, 27), profile capture of chosen parts and the preset
  format, unique copy names and shortcuts following renamed or deleted
  profiles, profile notices, the microphone gate (attack, release, pauses), focus
  settling (alt-tab doesn't change anything), how Solo, focus rules and
  microphone ducking combine, the foreground watcher starting and stopping
  within 500 ms, the hook path not using audio, COM or blocking calls, the
  output-device status wording, the confirmation before resetting all app
  devices, and that every new control is named and fits at minimum width.
- `cargo test --test audio_integration -- --ignored --test-threads=1`: all 11
  passed on real devices. New: Solo halves another app (a silent helper),
  restores it, and keeps a level set while it was lowered; app mute and
  relative-volume commands; a focus rule lowers the helper while another
  process is reported in front and restores it when the helper is reported in
  front; and per-app routing. For routing, Windows reported the chosen device
  (HDMI) at once; the helper's running stream did not move within 8 s, because
  it opened a specific device; a second process of the same app, launched
  afterwards, started on HDMI; resetting returned the preference to the system
  default.
- `cargo run --example routing_probe`: the per-app routing interface is
  available on this build (the 21H2 interface ID).
- Release build: 15,726,592 bytes. `scripts/lifecycle.ps1` passed with OpenGL
  (hidden CPU 16 ms over 1 s) and `-Renderer wgpu` (281 ms on WARP, within its
  usual spread).
- Not run: the real foreground hook switching between real windows (the engine
  path was tested with reported process IDs), microphone ducking with a live
  voice (no capturing app was running; the gate is unit-tested), routing on
  Windows 10 (the pre-21H2 interface ID), routing an elevated app, a shortcut
  cycle with real tray balloons while hidden, and Windows 10 in general.

## Stage 2 performance (2026-11-05, branch `stage2/performance`)

Allocation-free meter polling, the icon extraction LRU with a bounded request
queue, texture reuse, and wgpu built with only Direct3D 12 and Vulkan.

- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`:
  passed.
- `cargo test --locked`: 126 library tests passed (8 new), 3 ignored; 3 binary
  tests and 1 source-encoding test passed; 8 hardware tests ignored. New tests
  cover each app's loudest peak with every session read once and the map
  reused, apps that left or went silent, the meter change threshold in both
  directions, a relaunched app and a second app on the same binary reusing one
  extraction, a separate extraction per pixel size, a modified executable being
  extracted again, failures not being cached, a full request queue returning at
  once and every dropped request resolving later, LRU eviction, identical icons
  not being uploaded again while changed ones are, and keeping only the 32 most
  recent icons of apps that left.
- Build measurements, each a clean release build in its own target directory
  on this machine:

  | Configuration | Clean build | Incremental (one file) | Release exe | Crates |
  | --- | --- | --- | --- | --- |
  | Before (eframe `wgpu` feature) | 146 s | 50 s | 15,859,200 B | 199 |
  | wgpu `dx12` + `vulkan` only (adopted) | 140 s | 51 s | 15,325,696 B | 197 |
  | Same, plus `lto = "fat"` (not adopted) | 216 s | 113 s | 14,751,232 B | 197 |

  The lockfile lost `khronos-egl` and three empty `wgpu-core-deps-*` crates; no
  version changed. Fat LTO saves another 0.57 MB but more than doubles
  incremental release builds with no measured runtime gain. `panic = "unwind"`
  is kept: the panic hook and the `Drop`-based COM and registration cleanup
  rely on unwinding.
- `--renderer=wgpu` with a dev profile logged `renderer: wgpu fallback, Dx12 on
  "Microsoft Basic Render Driver"` and exited with code 0.
  `scripts/lifecycle.ps1` passed with OpenGL and with `-Renderer wgpu`. CPU in
  the first second after hiding: OpenGL 0–31 ms; wgpu on WARP 78–484 ms, the
  same spread as before the change (78–344 ms).
- Silent-path meter CPU, debug builds, window visible with meters on, one
  silent audio session, 10 s per sample, 8 alternating samples each: before
  47–5,625 ms (median about 211 ms), after 16–1,312 ms (median about 86 ms).
  Both have outliers of seconds from other activity on the machine, so this
  suggests a reduction but doesn't prove one.
- Not run: Explorer restart (tray-icon 0.26.1 re-adds the icon on
  `TaskbarCreated`; it needs Explorer to be ended by hand), the wgpu fallback
  on real Direct3D 12 or Vulkan hardware (this GPU has neither), and Windows 10.

## Stage 1 correctness fixes (2026-11-05, branch `stage1/correctness`)

Balance aggregation, process-marker reclaim and saturation recovery, skipping
unreadable sessions, device-notification coalescing, and the hotkey test guard.

- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`:
  passed.
- `cargo test --locked`: 118 library tests passed (12 new), 3 ignored; 3 binary
  tests and 1 source-encoding test passed; 8 hardware tests ignored. New tests
  cover uniform, mixed, mono-only and mono-plus-stereo balances, sessions
  appearing and ending, aligning after an adjustment, the mixer showing the
  right balance control for each state, 20,000 launches with unobserved exits
  and constant PID reuse (never saturates, each launch qualifies once),
  recovery from saturation without requalifying skipped or live processes,
  unknown processes keeping their markers, the liveness check against the test
  process and an exited child, and device-event bursts producing one refresh
  without losing a late event. The hotkey "never touches `window::`" guard now
  scans the whole module up to `mod tests`.
- `cargo test --test audio_integration -- --ignored --test-threads=1`: all 7
  passed on real devices (external change seen after 20 ms, new session after
  61 ms, removal 20 ms after process exit).
- `cargo run --example grouping_probe`: 4 sessions on the default output
  (system sounds and three single-process apps), each with its own grouping
  GUID. No multi-process app was running, so how Windows groups helper
  processes is still unverified; grouping is unchanged.
- Not run: a Bluetooth reconnect burst (the coalescing is unit-tested only), an
  app with several stereo sessions at different balances on real hardware,
  saturation over a real long-running session, and Windows 10.

## 0.4.0 final review corrections (2026-10-20, `experiments`, based on `86fdb9b`)

The three review findings have focused regression coverage. New tests first
failed against the unchanged implementation, then passed with the corrections:

- A missing exit event or a listener removed during shutdown no longer bypasses
  waiting on a running process. Process discovery also finds copies without a
  main window, and an opened handle is checked against the executable path
  before termination. A regression supplies a PID for another executable and
  verifies that copy survives.
- A failed settings load blocks saves across store clones even after a
  temporary read lock is released, including a geometry-only save. Successfully
  reloading the original preferences allows intentional changes to be saved.
- Directory and sharing-lock obstructions at `settings.json.bad` produce an
  accurate warning and prevent overwriting the original. Removing the
  obstruction allows a retry that verifies the original bytes in the backup.

Fresh local checks passed: formatting, Clippy with warnings denied, and
`cargo test --locked` (112 library tests, 3 binary tests and 1 encoding test
passed; 3 desktop-shortcut and 8 hardware tests ignored). The first full-suite
run exposed interference between parallel test children sharing a filename;
the fixtures now use separate names per test while retaining explicit
same-name/different-folder cases. The full-suite rerun passed.

A live isolated `--exit` smoke check passed for OpenGL and wgpu, both visible
and `--minimized`: helper and app returned exit code 0 with normal shutdown
markers, while a second copy in another folder kept running. Its separate
helper then shut that copy down cleanly too. The independent correction review
found no remaining merge-blocking issue.

`scripts/package.ps1` passed: the optimized executable, portable ZIP and Inno
Setup 6.7.3 installer were rebuilt. Both executables report product version
`0.4.0`; the ZIP includes the README, LICENSE and third-party notices alongside
the executable. PowerShell syntax and diff whitespace checks also passed.

Local Rust checks used 1.99.1. Release publication is gated on hosted Windows
checks, the Rust 1.95 minimum-version check, and dependency audit for the final
source. The exact hosted runs are linked from the GitHub Release; the earlier
run for `86fdb9b` covers only the audit baseline. Package hashes are in the
[0.4.0 release record](releases/0.4.0.md).

Not run in this correction pass: the installed upgrade/uninstall flow,
Windows 10, and the hardware audio matrix. The live exit matrix checks process
handling but does not replace an installed upgrade/uninstall test.

## 0.4.0 verification: audit fixes (2026-10-20, branch `fix/audit-findings`)

Fixes for meters under software rendering, the unbounded icon-thread join, settings file
encodings, unfiltered endpoint property notifications, and the
hardening/compliance items (uninstaller `--exit`, license notices, full
`explorer.exe` path, CI). Run on the machine in `PROJECT_CONTEXT.md` section 4
(it now also has a headphone output and a microphone).

- `cargo fmt --check`, PowerShell syntax parsing, and
  `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked`: 106 library tests passed (16 new), 3 ignored; 3 binary
  tests and 1 source-encoding test passed; 8 hardware tests ignored. New tests
  cover pausing meters under software rendering, a stuck icon lookup (shutdown returns within the 2 s limit
  and queued lookups are dropped), UTF-8 BOM / UTF-16 LE / UTF-16 BE settings,
  undecodable bytes, byte-exact migration backups, refusal to replace a settings
  file another program holds open without read sharing, the property filter,
  per-path exit events, stale exit signals, termination of a copy that ignores
  `--exit`, and the absolute `explorer.exe` path. Temporarily reverting each
  fix made its tests fail.
- `cargo +1.95 check --locked --all-targets`: passed. `cargo audit`: no
  vulnerabilities; three unmaintained warnings for Linux-only `zbus` crates
  that are not in the Windows build.
- `scripts/lifecycle.ps1` with OpenGL and `-Renderer wgpu`: passed.
- Live `--exit` against an isolated debug instance (visible, and hidden with
  `--minimized`): the instance logged "exit requested by another process",
  completed its normal shutdown with exit code 0 in about 1 s, and the
  separately installed Glass Mixer kept running. With nothing running from
  that path, `--exit` returned immediately with code 0.
- `scripts/third-party-notices.ps1`: 170 crates, 125 distinct license texts,
  including the egui font licenses. ISCC 6.7.3 compiled the installer with the
  new files and `[UninstallRun]` entry. The installer and uninstaller were not
  run, to avoid touching the installed copy.
- CPU, release build, an isolated copy, a browser playing audio (per process).
  With graphics acceleration, meters keep their full 30 Hz rate, so OpenGL is
  unchanged (700–1,344 ms per 5 s on this machine's 2010-era GPU, with meters
  on). With the wgpu fallback on WARP (software), meters on used
  19,700–20,000 ms per 5 s (all four cores); with meters paused, the same build
  measured 141–328 ms per 4 s. Hidden stays at 0 ms on both renderers.

- Not run: the installed upgrade and uninstall flow, hot-plug of a device whose
  name changes (covered by the property filter's name keys, not exercised
  live), and Windows 10.

## 0.3.0 release-candidate verification (2026-10-18)

The 0.3.0 version/resource update is prepared on `experiments`. Fresh local
checks on this candidate passed: `cargo fmt --check`, PowerShell syntax,
`git diff --check`, `cargo clippy --locked --all-targets -- -D warnings`,
and `cargo test --locked` (90 library tests, 3 binary tests and 1 source
encoding test passed; 3 desktop-shortcut and 8 hardware-audio tests ignored).
`scripts/verify-ui.ps1` passed with OpenGL and wgpu; `scripts/hotkey-smoke.ps1`
passed in normal and minimized startup modes. `scripts/package.ps1` with Inno
Setup 6.7.3 produced both packages. The executable and installer report
product version `0.3.0`; the portable archive contains the executable and
README. Package hashes are in [`docs/releases/0.3.0.md`](releases/0.3.0.md).

- [x] Hosted Windows CI for the exact 0.3.0 commit (`2561aee`): runs
  `#39106819443` (`experiments`) and `#39107297354` (`main`) passed.
- Not run for this release candidate: live audio mutations, installer upgrade
  from 0.1.0, physical keyboard injection, Narrator, Windows 10, and varied
  hardware. Earlier beta results remain below.

## Final review corrections (2026-10-18, branch `fix/beta2-review-corrections` on `209e563`)

This pass reviewed the pushed beta.2 commit `209e563` and applied the final
corrections: the Linux-only glib advisory is removed from `Cargo.lock`, hidden
in-window warnings keep their remaining display time, the posted-message
shortcut pressure test no longer depends on a global chord, the process
tracker's saturation latch is documented and regression-tested, and the UI
Automation check brings the window to the foreground before its live Tab
traversal. `docs/releases/0.2.0-beta.2.md` carries the SHA-256 values of the
rebuilt local packages.

- `cargo fmt --check`, PowerShell syntax parsing, and `git diff --check`: passed.
- `cargo test --locked`: 90 library tests passed, 3 ignored (up from 86: the
  new toast-lifetime and tracker-latch tests); 3 binary tests and 1
  source-encoding test passed; 8 hardware integration tests ignored (7 audio,
  1 devices).
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo build --release --locked` and `scripts/package.ps1` with Inno Setup
  6.7.3: passed; rebuilt both beta.2 artifacts in `dist/`. Windows version
  resources in the release executable and installer report `0.2.0-beta.2`; the
  portable archive contains the executable and README.
- Dependency check: `cargo tree --locked --target all -i glib` reports no
  matching package after the manifest change (`tray-icon` without its default
  `libappindicator` feature), and the lockfile diff only removes the GTK 0.18
  family. The Windows target tree never contained glib.
- Native UI Automation (`scripts/verify-ui.ps1` under OpenGL and
  `scripts/verify-ui.ps1 -Renderer wgpu`): passed, including the live `Tab`
  traversal. Before the script fix, the traversal failed whenever another
  application held the foreground: egui coalesces rapid posted Tabs while the
  window repaints at reduced rates when inactive, and UI Automation only
  reports `HasKeyboardFocus` on the active window. `ui-automation.ps1` now
  attaches to the foreground thread and activates the mixer window first,
  failing with a clear message if activation is refused. Repeated runs passed
  on both renderers.
- Isolated lifecycle (`scripts/lifecycle.ps1`) and both global-shortcut smoke
  modes (`scripts/hotkey-smoke.ps1` and `-Minimized`): passed. These cover tray
  creation and close-to-tray, second-instance rejection, settings/geometry
  save, hidden startup, clean exit, and real `Ctrl+Alt+F23` registration with
  desktop key injection after the registration seam change.
- Not run: hosted CI for these corrections (a push is required), live audio
  changes, physical keyboard injection, Narrator, installer upgrade from
  0.1.0, and Windows 10. See the manual checklist below.

## Second beta packaging (2026-10-18, commit `209e563`)

The 0.2.0-beta.2 source includes the follow-up review fixes below and releases
the shortcut injector's error mutex before joining its worker on failure.
Hosted GitHub Actions CI run `#39094235224` (`windows-latest`) passed for the
pushed commit `209e563`. The final review corrections on top of it are recorded
in the section above. Live audio changes, physical keyboard injection,
Narrator, and installer upgrade from 0.1.0 remain unverified.

Fresh checks on `experiments` for beta.2 (at `209e563`):

- `cargo fmt --check`, PowerShell syntax parsing, and `git diff --check`: passed.
- `cargo test --locked`: 86 library tests passed, 3 ignored; 3 binary and 1
  source-encoding test passed; 8 hardware integration tests ignored.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `scripts/package.ps1`: passed with `cargo build --release --locked` and Inno
  Setup 6.7.3; produced both beta.2 artifacts in `dist/`.
- Windows version resources in the release executable and installer report
  `0.2.0-beta.2`; the portable archive contains the executable and README.
- The full native UI Automation checks for both renderers passed during the
  previous review of the same feature source. They were not rerun for this
  commit alone; they were rerun on the corrected branch above, where the live
  Tab traversal was made foreground-independent.

## Follow-up merge-review verification (2026-10-17, audit worktree on `ea9c816`)

All checks in this section were run against the uncommitted worktree changes on
top of commit `ea9c816` on branch `glass_mixer_audit` (`R1`–`R9` and follow-up
findings `F1`–`F6`). Hosted GitHub Actions CI run `#38919273079` (`windows-latest`)
validated committed `ea9c816` only; the uncommitted worktree changes recorded
here require a fresh hosted CI run once committed and pushed.

- **Automated checks on the uncommitted worktree:**
  - `cargo fmt --check`: passed.
  - PowerShell parser checks across `scripts/*.ps1`: passed.
  - `cargo clippy --locked --all-targets -- -D warnings`: passed.
  - `cargo test --locked`: **86 library tests passed** with 3 ignored; **3 binary
    tests passed**; **1 source-encoding test passed**; 8 hardware audio tests
    ignored.
  - `cargo build --release --locked`: passed (version resource `0.2.0-beta.1`,
    numeric `0.2.0.0`, supports `--renderer=wgpu` in release as well as debug
    builds).
  - `git diff --check`: passed.
- **Native UI Automation checks (`scripts/verify-ui.ps1` and
  `scripts/verify-ui.ps1 -Renderer wgpu`):** passed under both OpenGL and forced
  `wgpu`. `scripts/ui-automation.ps1` now strictly requires the `Settings` and
  `Mixer` navigation buttons to expose `InvokePattern` or `TogglePattern`
  (throwing instead of skipping if absent; `egui`'s selectable tab buttons
  expose `TogglePattern` via `accesskit_windows`), verifies Settings toggle
  descriptions via `UIA_FullDescriptionPropertyId` (`30159`, where
  `accesskit_windows` maps `Node::set_description`), verifies the client
  viewport in screen coordinates via `GetClientRect` + `ClientToScreen`, asserts
  that `Open settings folder` starts off-screen below the Settings viewport and
  unfocused before `Tab` traversal, checks `PostMessageW` return values for
  `WM_KEYDOWN`/`WM_KEYUP` (`VK_TAB`), and requires `Open settings folder` both
  to gain keyboard focus and to scroll upward into the visible client bounds.
- **Summary of `2026-10-17` review and follow-up coverage (`R1`–`R9`, `F1`–`F6`):**
  - **Settings migration & backup (`R1`, `F5`, `src/settings.rs`, `src/app.rs`):**
    `settings.v<N>.backup.json` is written via a `sync_all()`ed `.tmp` file and
    validated by parsing as `Settings` with `s.version == expected_version`
    (supporting both schema `1..4` and leniently recovered version `0`). Failed
    backups retain the source `version` in memory, surface a startup warning
    through `MixerApp::toast` (reaching hidden-startup tray notifications; the
    final review corrections later replaced the hidden-deadline extension with
    a remaining-time countdown that is frozen while hidden), and refuse to
    overwrite `settings.json` until backup succeeds on a subsequent save.
  - **Per-process start-rule tracking (`R2`, `F1`, `src/audio/preferences.rs`,
    `src/audio/engine.rs`):** `StartTracker` tracks `ProcessInstance { key, pid,
    created }` across multi-instance launches and delayed audio session creation,
    reclaims confirmed exited PIDs (`forget_exited_pid`) and same-PID creation-time
    replacements, and latches `saturated = true` at `MAX_TRACKED_PROCESSES`
    (`1024`) so live markers are never evicted and running processes cannot
    re-qualify after `1024` newer processes appear. The latch stays set until
    rearming or restart even when confirmed exits later free slots (instances
    skipped while saturated were never recorded, so clearing it could requalify
    an untracked running process); a regression test covers this.
  - **Headless settled AccessKit tree & keyboard focus scrolling (`R3`, `R4`, `R5`,
    `src/ui/test_support.rs`, `src/ui/mixer.rs`, `src/ui/presets.rs`,
    `src/ui/settings_view.rs`):** `UiHarness` inspects only reachable nodes from
    the final settled frame's `TreeUpdate`, keeps `TextInput` values separate
    from accessible names, checks `Link` controls, and verifies `Tab`/`Shift+Tab`
    focus visibility across `Mixer` (through offline pins and down to the bottom
    live app row `"Application Number 13 volume"`), `Devices` (including expanded
    mono/stereo/error states), `Presets`, and `Settings` at `360×440` and
    `480×720` in both densities.
  - **Shortcut editor & shutdown pressure (`R6`, `R7`, `F3`, `F4`,
    `src/platform/hotkeys.rs`):** `primary_chord_for_action` selects the active or
    retained validated binding when duplicate same-action entries exist (both
    valid-first/duplicate-second and invalid-first/valid-second).
    `shutdown_completes_under_posted_hotkey_message_pressure` exercises
    `PostThreadMessageW(WM_HOTKEY)` pressure during shutdown in every `cargo
    test` run through a synthetic registration seam (final review corrections
    above), so it no longer depends on owning a global test chord;
    `try_inject_ctrl_alt_f21_with` rejects `0` or partial
    `SendInput` counts (`InjectionError::BlockedZero` / `Partial`), sends a
    best-effort `KEYEVENTF_KEYUP` release batch on partial injection without
    falling back to `PostThreadMessageW`, and is unit-tested alongside RAII
    `InjectorGuard` unwind cleanup. The `#[ignore]` test
    `shutdown_completes_while_the_window_shortcut_is_being_pressed` strictly
    requires real `SendInput` delivery into the desktop input queue.

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
  afterwards (F-22, see the section above).

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
