# Changelog

## 0.6.0 (2026-11-13)

Package details and checksums are in [the release record](docs/releases/0.6.0.md).

### Visual refresh and typography

- **Tahoe / Control Center design language:** updated dark glass, light glass,
  opaque, and high-contrast palettes (`ui/theme.rs`), card/control corner radii
  (`12/10 pt` and `8/7 pt`), hairline grouped list dividers, and centralized
  motion tokens with reduced-motion support.
- **Polaris typography:** embedded Salted Flutters Polaris Bold alongside Regular
  and Semibold (`platform/fonts.rs`), added a 9-role typography system with
  interpolated `trak` letter-spacing via `LayoutJob` (`ui/typography.rs`), and
  computed dynamic numeric readout widths (`ui/widgets.rs`).
- **Widgets and views:** added semantic buttons, an animated-pill segmented tab
  bar, view fade transitions, restyled sliders/toggles/disclosures/search
  fields/combo icons/check rows, `2.5 pt` focus rings (`2.0 pt` in high
  contrast), grouped Mixer cards with refreshed details/empty states, refreshed
  Devices, Presets, and Settings views, standalone `HotkeyEditor`
  (`ui/hotkey_editor.rs`), and fade-in visibility-frozen toasts (`ui/toasts.rs`).

### Native Windows integration and performance

- **Immediate OS appearance and DPI wakeups:** subclassed the window for
  `WM_SETTINGCHANGE`, `WM_DWMCOLORIZATIONCOLORCHANGED`, and `WM_DPICHANGED`
  (`platform/window.rs`).
- **Themed, per-monitor-DPI tray menu:** replaced the stock tray menu with an
  owner-drawn Win32 popup (`platform/tray_menu.rs`) matching system dark/light/high-contrast
  themes, scaled per monitor, rendered with a process-private Polaris Semibold
  font and `&`-derived mnemonics.
- **DWM border color:** set `DWMWA_BORDER_COLOR` from the blended glass border
  palette (`DWMWA_COLOR_NONE` when solid or high contrast) in
  `platform/backdrop.rs`.
- **Lower UI overhead:** replaced per-frame settings cloning with persistent
  `settings_seen` diffing in `app.rs` and skipped redundant Win32 tray menu
  rebuilds when tray state is unchanged.

### Reliability, packaging, and internal cleanup

- **Audio & engine robustness:** preserved Solo across engine rebuilds, skipped
  non-fatal session errors during attenuation reconciliation, returned early on
  `LoadDevices` when the engine is unavailable, avoided caching transient
  `Route::Unknown`, caught engine thread panics (`Unavailable`), bounded
  `file_description` UTF-16 slices, and coalesced `SetStreamVolume` commands.
- **Settings, transfer, and lifecycle:** normalized and capped
  `mic_ducking.targets`, compared JSON floats within `1e-6` in `import_notes`,
  retained `dirty_since` and rate-limited failed-save toasts, preserved
  maximized window geometry separately from normal geometry, prevented
  `--minimized` launches from activating an existing window, raised unfocused
  visible windows on show/hide shortcuts, and skipped startup path repair in
  debug builds and dev profiles.
- **Installer & workflow hardening:** checked `--exit` return codes during
  upgrades and removed `%LOCALAPPDATA%\GlassMixer` on confirmed uninstall; pinned
  workflow actions to commit SHAs, added a weekly `cargo audit` schedule, split
  release build/publish permissions, and required a release record in
  `scripts/check-version.ps1`.
- **Readiness fixes:** the draft-release job now names its repository
  explicitly (`GH_REPO`); the automatic backup just written always survives
  retention, even if the clock moved back; export and import share one 32 MiB
  limit, so every saved export imports again and an oversized one is refused
  before any file is created; foreground-app reports survive an audio-service
  outage and are resolved after recovery; failed background-lowering restores
  retry on their own with a capped backoff while healthy state keeps
  publishing; the tray menu follows the app's selected theme, accent, and
  high-contrast setting, including while the window is hidden; and changing
  only the Windows animation preference now reapplies the style immediately.
- **Code organization:** extracted `platform/hotkey_model.rs`,
  `ui/hotkey_editor.rs`, `ui/toasts.rs`, and `src/audio/engine/*` submodules,
  and removed dead endpoint commands and registry mono helpers.

## 0.5.0 (2026-11-06)

Package details and checksums are in [the release record](docs/releases/0.5.0.md).

### Added

- **Per-app output devices.** An app's details offer an Output menu: System
  default or any connected output. Windows remembers the choice for the app,
  so its later launches start there too. Apps that follow the default device
  move at once; apps that opened a specific device switch when they restart
  playback, and the row says so. Settings > Audio can reset every app's
  device, after a confirmation, because Windows only offers that for all apps
  at once. Apps playing on any output now appear in the Mixer.
- **Solo.** A pinned app's Solo button lowers every other app (to a percentage
  of its own level, or muted; set in Settings). The header shows which app is
  soloed with an End solo button. When Solo ends, levels come back, except
  any you changed meanwhile, which are kept.
- **Lower in background.** Opt-in per pinned app: lower or mute it while
  another app's window is in front, and bring it back when you return to it.
  Focus has to stay put for a quarter second first, so alt-tabbing doesn't
  bounce levels.
- **Microphone ducking.** Opt-in in Settings > Audio: lower chosen pinned apps
  while you talk, with a speech level, start and restore delays, and a level
  to keep. Only the microphone level is read; nothing is recorded. The header
  shows when it's active, with Turn off.
- **Profiles** (formerly presets) can also store each app's output device, the
  master level, and the default output and input; they can be renamed,
  duplicated, trimmed part by part and deleted. Existing presets load as they
  were.
- **More shortcuts:** next pinned output device, and per pinned app mute,
  volume up/down and Solo, plus one per profile. Pin outputs for cycling in
  Devices; a shortcut switch is confirmed in the tray while hidden.
- **Per-stream controls.** An app with several streams lists each one, with
  its own mute and volume, in its details.
- **Mouse wheel step** for volume sliders: 1, 2 or 5 % or a custom value.
- **Salted Flutters Polaris typeface.** The interface now uses Polaris (Regular
  and Semibold), embedded in the executable (about 7 MB larger). Segoe UI backs
  it up for any glyph it lacks, and Settings > Appearance > Typeface switches
  back to Segoe UI. The font's license ships in `fonts\LICENSE.txt` in both
  packages.
- **Rename apps and reorder pins.** An app's details have a Name field (empty
  or Reset restores the name Windows reports), and pinned apps have Move up and
  Move down. Pinned outputs can be moved Earlier or Later, which sets the order
  the cycle shortcut follows.
- **Export and import settings** (Settings > Backup and restore). Export saves
  your preferences, without the window position, to a versioned file. Import
  checks a file first, shows what it contains and what couldn't be read, and asks
  before replacing anything; your current settings are saved to a backup file
  first, and a file that is malformed, too large or from a newer version is
  refused with the reason.
- **Diagnostic report** (Settings > About): saves or copies a plain-text report
  of versions, states, counts, recent warnings and the log's tail for bug
  reports. App, device and profile names and your user folder are hidden, audio
  and files are never included, and nothing is sent anywhere.

### Fixed

- **An app's balance showed only one of its streams.** When an app has several
  audio sessions, the row displayed the first stereo session's balance even if
  the others differed. The balance is now aggregated: one value when every
  stereo stream agrees, "Differs between streams" with a "Center all" button
  when they don't, and "Not available" when no stream is stereo.
- **Automatic preset rules could stop for the rest of the session.** Markers
  for processes whose exit was never observed (no `SYNCHRONIZE` access, or
  audio that ended before the process did) were never released, so a long
  session could reach the 1,024-instance limit, after which no launch
  qualified until the rules were re-armed. Markers of exited or reused PIDs are
  now reclaimed during session refreshes, and reaching the limit recovers
  safely once capacity frees up.
- **One unreadable audio session blocked the whole app list from refreshing.**
  It is now skipped and logged.

### Changed

- **Level meters no longer allocate while polling.** Each 30 Hz poll reuses its
  buffers, and the UI shares the published levels instead of copying them
  every frame. The rate is unchanged.
- **Relaunched apps reuse their icon.** Extracted icons are cached by the
  executable's path, size and modification time (plus the icon resource,
  AUMID and pixel size) in a bounded LRU, so a short-lived process or a
  second app running the same binary needs no new Shell extraction, and an
  updated binary is extracted again. Recently used textures are kept and an
  identical result is not uploaded to the GPU again.
- **The icon request queue is bounded (16).** A full queue never blocks the
  UI; the dropped request is made again on a later frame.
- **Smaller executable.** wgpu (the fallback renderer) is built with only its
  Direct3D 12 and Vulkan backends, the only ones the fallback ever uses: the
  release exe shrinks from 15.86 MB to 15.33 MB and 2 crates leave the build.
- Bursts of device notifications (for example a Bluetooth reconnect) are merged
  into one re-enumeration 50 ms after the first; default-device changes remain
  immediate.
- New read-only `examples/grouping_probe.rs` lists sessions with their Windows
  grouping parameter, to check multi-process grouping before relying on it.
- **The installer asks a running copy to exit before an upgrade.** The previous
  installer relied on Windows to close the app, which terminated it without
  saving its settings or removing its tray icon.
- **Release tooling.** One version number is checked across `Cargo.toml`,
  `Cargo.lock`, the manifest, the installer and this changelog. Packaging
  verifies both packages (exact contents, version, 64-bit build, the Polaris
  notice, byte-exact licenses, signature state) and writes SHA-256 checksums.
  `scripts/installer-smoke.ps1` tests install, upgrade over a running copy and
  uninstall of a running copy in isolation from a real installation, and a
  tagged release builds a draft release in CI. Packages are signed when a
  certificate is configured; none is, so 0.5.0 downloads are unsigned. See
  `docs/RELEASING.md` and `docs/VALIDATION_MATRIX.md`.

## 0.4.0 (2026-10-20)

Fixes and hardening from the 2026-10-20 audit of 0.3.0 and the final review.
Package details and checksums are in [the release record](docs/releases/0.4.0.md).

### Fixed

- **Level meters saturated the CPU without graphics acceleration.** When the
  fallback renderer runs on the software rasterizer (WARP, for example in
  virtual machines or on GPUs without Direct3D 12 or Vulkan), each meter
  repaint is drawn by the CPU; with music playing this used four full cores
  and made the controls themselves lag. Meters are now paused in that mode
  (Settings explains why), which brought it to 4–8 % of one core. With
  graphics acceleration, meters keep their full rate (about 30 Hz).
- **Exit could hang behind a stuck icon lookup.** The icon thread was joined
  without a time limit after the tray icon had already been removed, so a Shell
  call blocked on, for example, an offline network share left a process with no
  window or tray icon holding the single-instance lock. Shutdown now waits at
  most 2 seconds and drops icon lookups still queued.
- **A hand-edited settings file could reset every preference.** A UTF-8 file
  with a byte-order mark (Notepad, Windows PowerShell `-Encoding utf8`) was
  treated as damaged, and a UTF-16 file (PowerShell `>`) was overwritten by
  defaults on the next save without being kept. Both encodings now load;
  undecodable files are kept as `settings.json.bad`; migration backups keep the
  original bytes; and a settings file that exists but can't be read (locked or
  access denied) is never replaced. A failed startup read also blocks later
  saves after a temporary lock clears, until a successful reload. Damaged-file
  backups are synced and checked byte for byte; an obstructed backup leaves
  the source untouched and can be retried safely.
- **Endpoint property notifications refreshed every device.** Any property
  change, including ones some drivers report constantly, re-read all devices
  and their properties. Only names, the selected format and the system-effects
  flag now trigger a refresh.

### Changed

- The uninstaller no longer runs `taskkill /IM glass-mixer.exe /F`, which also
  killed portable copies. It runs `glass-mixer.exe --exit`, which asks the copy
  running from the install folder to exit cleanly (saving settings and removing
  its tray icon) and terminates it only if it hasn't exited after 10 seconds.
  Discovery does not require a surviving window or exit listener. The process
  path is verified again on its opened handle, and success requires confirmed
  exit rather than merely a successful termination request.
- Both packages include `LICENSE` and `THIRD-PARTY-NOTICES.txt`, generated by
  `scripts/third-party-notices.ps1` from `cargo metadata`.
- "Open settings folder" starts `explorer.exe` by its full path in the Windows
  folder; a bare name is looked up in the application's own folder first.
- CI also checks the build with Rust 1.95 (`rust-version`), runs `cargo audit`,
  and checks that the notices script succeeds.

## 0.3.0 (2026-10-18, branch `experiments`)

First stable release of the 0.3 series, carrying forward the 0.2.0 beta
features and review corrections. This release includes the fixes for live
preset-process tracking at capacity, hidden warning expiry, deterministic
shortcut pressure coverage, accessibility verification, and the Rust `glib`
iterator unsoundness advisory by removing the unused Linux GTK tray dependency
from the Windows-only build graph.

### Release packages

- Portable: `glass-mixer-0.3.0-portable.zip` (executable and README).
- Installer: `glass-mixer-0.3.0-setup.exe` (per-user Inno Setup installer).
- Both packages are unsigned unless a signing certificate is configured.

### Verification status

The packages are built and verified locally on Windows. The correction commit
still needs hosted Windows CI and a final merge review before publication on
the GitHub Releases page.

## 0.2.0-beta.2 (2026-10-18, branch `experiments`; review corrections on `fix/beta2-review-corrections`)

The second beta of the 0.2.0 feature release. It includes the follow-up review
fixes documented below, plus a correction to the shortcut pressure-test cleanup
that releases its error mutex before joining the injector thread, and the final
review corrections: the Linux-only glib advisory is removed from the lockfile,
hidden in-window warnings keep their remaining display time, the posted-message
shortcut pressure test no longer depends on a global chord, and the process
tracker's saturation latch is documented. The build artifacts are packaged
locally; this branch has not been merged into `main`.

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
  presets later. An optional automatic rule tracks each running process
  instance (`(AppKey, pid, created)`) and applies a preset once to a newly
  launched process whose initial session starts at `100%` and unmuted (see
  "Known limitations").
- **Tray notifications** for warnings and errors while the window is hidden,
  with throttling and duplicate suppression; Windows notification settings
  apply. Can be turned off in Settings.
- **Renderer fallback:** if OpenGL cannot start, Glass Mixer opens an opaque
  window on Direct3D 12 or Vulkan instead of failing, and `--renderer=wgpu`
  forces the opaque fallback on the command line in both release and debug
  builds. The log names the renderer and graphics adapter in use.
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
  invalid entry no longer resets the others; unknown top-level fields from
  newer versions are kept. Saves flush temporary files to disk with `sync_all`
  before atomic rename. Migration writes and verifies `settings.v1.backup.json`
  atomically before promoting the schema or overwriting `settings.json`; if
  backup creation fails, Glass Mixer reports a warning, keeps the migrated
  settings in memory, and leaves the original `settings.json` untouched until
  backup succeeds on a later save.
- System sounds stays visible when idle apps are hidden.
- Executable size grew from 8,442,880 to about 15.8 MB because the Direct3D 12 /
  Vulkan fallback renderer is now included.

### Fixed

- Garbled characters (for example "â€¦") in some interface text.
- Failed app-icon lookups can be retried with Refresh or when the app returns,
  and icon caches no longer grow without bound.
- Found and fixed while auditing this beta:
  - One invalid or conflicting global shortcut disabled every shortcut, and
    duplicate same-action entries in hand-edited settings could cause the
    editor to display an active primary shortcut as "Not active".
  - Pressing the show/hide shortcut while Glass Mixer was exiting could hang
    the exit, leaving the process running with no tray icon, and a dropped
    request channel could leave a queued `WM_HOTKEY` unprocessed on shutdown.
  - With the fallback renderer, exit intermittently deadlocked inside the
    OpenGL driver (the process stayed running after "shutdown complete").
    Ambient `WGPU_BACKEND=gl` environment variables outside a dev profile are
    now stripped so they cannot re-enable the OpenGL backend in the fallback.
  - At the minimum window width, the output percentage and Settings button
    were cut off; long names in offline pins, the shortcut editor, expanded
    non-stereo device balance labels and the settings path ran past the window
    edge; the sound-properties panel was squeezed into a small nested scroll
    area.
  - Several controls (device lists, filter box, preset name, volume step) had
    no name for screen readers, toggle descriptions such as the reason glass
    is off were not exposed, and a disabled **Save** gave no reason.
  - Controls inside scrollable views (`Mixer`, `Devices`, `Presets`,
    `Settings`) now scroll into view on focus transition (`gained_focus`)
    without fighting manual pointer scrolling.
  - Automatic preset start rules now track individual process instances
    (`(AppKey, pid, created)`) rather than a per-executable timestamp
    watermark, so multi-instance launches and older processes that open audio
    late are handled independently; applying a preset continues across
    remaining apps if one app returns a non-fatal COM error and reports the
    failed app names.
  - The lockfile no longer contains `glib 0.18.5` (RUSTSEC-2024-0429 /
    GHSA-wrw7-89jp-8q8g): `tray-icon` is built without its default
    `libappindicator` feature, which existed only for Linux GTK and was never
    compiled on Windows. Glass Mixer is Windows-only, so the Win32 tray and
    menu behaviour is unchanged.
  - An in-window warning queued while the window is hidden no longer expires
    during a long idle. Toast time is frozen while hidden and starts (or
    resumes) when the window is shown, still without a hidden timer or repaint.
  - The posted-message shortcut pressure test no longer needs Windows to grant
    it a global chord: a synthetic registration seam resolves its posted
    `WM_HOTKEY` messages, so the pressure assertion always runs. Real
    `SendInput` delivery remains a separate ignored desktop-input test.

### Upgrading and downgrading

- **From 0.1.0:** settings are migrated automatically. The first time 0.2.0
  reads the old file it writes and verifies an untouched copy as
  `settings.v1.backup.json` next to `settings.json` (`%APPDATA%\GlassMixer`)
  before overwriting `settings.json` with the new schema.
- **Back to 0.1.0:** 0.1.0 cannot keep 0.2.0's pins, presets and shortcuts. On
  its next save it drops them and rewrites the file in the old format (this
  was reproduced). To go back cleanly, exit Glass Mixer and rename
  `settings.v1.backup.json` to `settings.json` before starting 0.1.0.
- The installer upgrades in place (same per-user install, no admin rights).
  Installing this beta over 0.1.0 has **not** been tested yet.

### Known limitations

- **Automatic preset rules only apply to default `100%` unmuted initial sessions.**
  A rule applies once to a newly launched process after the rule is enabled,
  and only when its initial session starts at `100%` and unmuted with no user
  change seen first. Windows remembers per-app levels between launches, so an
  app whose level was previously changed will keep its Windows-restored level
  unless you press **Apply now**. Real-hardware interaction with
  Windows-restored sessions remains unverified.
- **Accessibility:** programmatic focus-transition auto-scrolling inside scroll
  views, live Windows `Tab` traversal in Settings, and UI Automation
  `FullDescription` exposure on Settings toggles are verified by tests, but real
  Narrator speech, high-contrast themes and multi-DPI scaling remain unverified.
- The fallback renderer can be forced with `--renderer=wgpu` in release and
  debug builds, but has not been tested on a machine where OpenGL naturally
  fails at startup. On the test machine (GeForce GT 340, no Direct3D 12 or
  Vulkan driver) it uses Windows' software (WARP) adapter.
- Sound properties are read-only by design. Default-device switching still
  uses the undocumented `IPolicyConfig` interface.
- **Automatic preset rules stop arming after 1,024 un-reclaimed process
  instances.** The tracker never evicts live markers, so it latches frozen and
  skips unseen instances; the latch stays set until the rules are re-armed or
  Glass Mixer restarts, even if confirmed exits later free slots. Instances
  skipped while saturated were never recorded, so clearing the latch
  automatically could requalify an untracked running process.
- Tray notifications depend on Windows notification settings.
- Builds are unsigned.

### Verification

Checked locally on Windows 11 Pro 25H2 (build 26200), NVIDIA GeForce GT 340,
one HDMI output and no microphone. Hosted GitHub Actions CI run `#38919273079`
covered the earlier committed base `ea9c816`; the pushed beta.2 commit
`209e563` passed run `#39094235224` on `windows-latest`. The final review
corrections documented above were verified locally and need their own hosted
run after push (details in `docs/TESTING.md`).

- **Automated:** formatting, PowerShell script syntax, clippy with warnings
  denied, the full test suite (unit tests, headless settled-tree layout /
  accessibility / keyboard focus-scroll tests, settings migration/backup
  gating tests, process-instance start-rule and batch execution tests) and the
  release build, all with `--locked`.
- **Manual on this machine:** repeated isolated start/exit runs with both
  renderers, global-shortcut smoke and exit-stress tests, UI Automation
  (including Settings toggle `FullDescription` descriptions and live `Tab`
  scroll into view) and screenshots of every view at the default and minimum
  width.
- **Not verified:** Windows 10, microphones, several outputs, hot-plug,
  sleep/resume, audio-service restart, 125–200% scaling, light and
  high-contrast themes, Narrator speech, automatic preset rules with live
  Windows-restored app volumes, the installer upgrade from 0.1.0, and the
  fallback on a machine without OpenGL or over Remote Desktop.

## 0.1.0 (2026-10-15)

First release: master and per-app volume, mute and balance, default-device
switching, tray icon and menu, glass backdrop, themes, accent colours and
density, launch at login, single instance, and an installer and portable zip.
See `docs/PROJECT_CONTEXT.md` for the full record.
