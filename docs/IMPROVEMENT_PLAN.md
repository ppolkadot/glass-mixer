# Glass Mixer improvement plan

Written 2026-11-13 against commit `374fbba` on branch `experiments` (version 0.5.0).
This document is a work order for a coding agent. Each task is self-contained:
it says what is wrong, where, what to change, how to test it and when it is done.

## 0. Read this first

### 0.1 How to work from this plan

1. Do one task at a time, in the order given in section 6. Finish and verify a
   task before starting the next.
2. Line numbers in this plan were correct at `374fbba` and **will drift**. Treat
   them as hints. Before editing, find the code by the symbol name given in the
   task (use a content search for the function, struct or string) and read the
   whole function. If the code you find does not match the task's description of
   it, stop and report the mismatch; do not guess at what was meant.
3. Each finding has a status:
   - **Verified**: the described code was read and confirmed at `374fbba`.
   - **Reported**: found by a code read, not independently re-checked. Confirm
     the described behaviour by reading the code before changing anything.
   - **Suspected**: plausible from the code but never reproduced. Reproduce or
     prove it first (the task says how). If it does not reproduce, write that
     down in your report and skip the task.
4. Never invent an API. This project uses `egui`/`eframe` **0.36.2** and
   `windows` **0.62**, which differ from older versions you may remember (for
   example, `eframe::App` here has `logic()` and `ui(&mut self, ui: &mut egui::Ui, ...)`,
   not `update()`; rounding is `CornerRadius`; panels are `egui::Panel::top(..)`).
   Before calling any egui function you have not already seen used in this
   repository, open its definition in the local registry and read the signature:
   `%USERPROFILE%\.cargo\registry\src\index.crates.io-*\egui-0.36.2\src\`
   (siblings: `eframe-0.36.2`, `epaint-0.36.2`, `emath-0.36.2`, `egui-winit-0.36.2`).
   Section 7 lists the APIs this plan relies on and where each is defined.
   For Win32, check the function exists in the `windows` crate and that its
   feature is enabled in `Cargo.toml` before using it.
5. Do not add a dependency unless the task says so.
6. If a task turns out to need a decision this plan does not make, stop and ask.

### 0.2 Project rules (from the maintainer; not negotiable)

- **Polaris stays the main font.** `Typeface::Polaris` is the default
  (`src/settings.rs`, `Settings::default`) and must remain so. Do not add, swap
  or bundle any other font. Do not use Apple's SF Pro or SF Symbols: their
  licence restricts them to Apple platforms. Icons stay Segoe Fluent Icons /
  Segoe MDL2 (`src/ui/widgets.rs`, `mod glyph`). New text must use
  `FontFamily::Proportional` or the `SEMIBOLD` family via `widgets::semibold(size)`
  so it follows the Typeface setting; never name a font file or family directly.
- Do not modify `assets/fonts/` at all (the licence file must stay byte-exact).
- **Fluidity over resource savings.** Never lower a frame rate, meter rate or
  animation smoothness to save CPU on capable hardware. Level meters stay at
  30 Hz. Performance tasks below remove wasted work; they must not make
  anything visibly less smooth.
- Comments: minimal and useful. No comments that restate the code.
- Commit only when the maintainer asks. Never push. Never add a
  `Co-Authored-By` or any Claude/AI attribution line to commits or PRs.
- Work on the `experiments` branch unless told otherwise.

### 0.3 File and tooling pitfalls

- `.rs` files use LF line endings; `.ps1` and `.iss` files use CRLF. Keep them.
- `rustfmt.toml` sets `max_width = 120`. Run `cargo fmt` after every edit.
- `tests/source_encoding.rs` fails the build if any file under `src/` contains
  mojibake sequences, including a raw capital A-tilde. In test strings that need
  that character write `\u{c3}`.
- When generating Rust through a shell heredoc, backslashes get halved and
  `\a`, `\b`, `\u` can turn into control characters. Write Rust with the file
  write/edit tools, not through the shell.
- Never pipe `cargo` through `tail`/`grep` inside an `&&` chain: it hides failures.
- Do not edit sources while a background build or test run is in progress.
- The installed copy of the app holds the single-instance mutex. To run the app
  for testing use a **debug** build with an absolute `GLASS_MIXER_DEV_PROFILE`
  directory (see section 5). Write test `settings.json` files without a BOM.
- The settings schema is versioned (`SETTINGS_VERSION` = 4 in `src/settings.rs`).
  Only bump it if a task says so. New fields must have `#[serde(default)]` so
  version-4 files still load.

### 0.4 Out of scope

Do not work on these; they are tracked elsewhere or need the maintainer:
code-signing certificate, running CI on GitHub, Windows 10 testing, README
screenshots, the compact overlay, and the 125–200 % DPI pass. The tray flyout is
described in task U12 as a design note only; do not build it under this plan.

---

## 1. Verified facts about the repository

| Fact | Where |
| --- | --- |
| Windows-only Rust app, edition 2024, MSRV 1.95 | `Cargo.toml` |
| UI: eframe 0.36 with `glow` (primary) and `wgpu` dx12+vulkan (fallback, opaque) | `Cargo.toml`, `src/main.rs` `run_with_renderer` |
| App struct `MixerApp`; `impl eframe::App` with `logic()` and `ui()` | `src/app.rs` (~772, ~891) |
| Views: `enum View { Mixer, Devices, Presets, Settings }`; nav is four `ui.selectable_value` calls | `src/app.rs` (~44, ~910) |
| Look/tokens: `Look { p: Palette, m: Metrics, icons: bool }` | `src/ui/mod.rs`, `src/ui/theme.rs` |
| Style applied once per look change in `MixerApp::refresh_look` via `theme::apply` | `src/app.rs` (~388) |
| Custom widgets: `Slider`, `icon_button`, `toggle_row`, `segmented`, `card`, `fallback_tile`, `percent_label`, `focus_ring`, `set_name`, `scroll_on_focus` | `src/ui/widgets.rs` |
| Mixer view: `header`, `input_section`, `apps_section`, `app_row`, `details`, `show_devices` | `src/ui/mixer.rs` |
| Presets view (heading text is "Profiles") | `src/ui/presets.rs` `contents` |
| Settings view: `section`, `labeled`, `appearance`, `behavior`, `audio`, `backup`, `about` | `src/ui/settings_view.rs` |
| UI test harness: `Harness::run(width, height, ui_fn) -> Frame`, `unnamed()`, `past_right_edge()` | `src/ui/test_support.rs` |
| Window: default 480×720, min 360×440, native title bar, transparent, DWM backdrop | `src/settings.rs` consts, `src/main.rs`, `src/platform/backdrop.rs` |
| Audio engine thread: `struct Runner` owns `Option<Engine>` and survives engine rebuilds | `src/audio/engine.rs` (~171) |
| Reductions (Solo, focus, mic ducking) go through `attenuation::Ledger`, keyed by session **instance** id | `src/audio/attenuation.rs`, `engine.rs` `apply_reductions` |
| Settings store: temp file + rename, migration backup `settings.v{n}.backup.json`, damaged file kept as `settings.json.bad` | `src/settings.rs` `SettingsStore` |
| Settings dir: `%APPDATA%\GlassMixer`; logs and diagnostic reports: `%LOCALAPPDATA%\GlassMixer` | `SettingsStore::default_dir`, `src/main.rs` `log_dir` |
| Git tags: `v0.1.0`, `v0.3.0`, `v0.4.0`, `v0.5.0`. `--exit` handling first exists in `v0.4.0` | `git tag`, `git show v0.3.0:src/main.rs` |
| Release profile: `opt-level 3`, thin LTO, `codegen-units 1`, `strip`, `panic = "unwind"` (unwind is required for Drop cleanup; keep it) | `Cargo.toml` |

Areas reviewed and found sound; **do not "fix" these**: audio callback
registration/unregistration, COM apartment and drop order in `run`, the settings
temp-write-and-rename, import validation in `transfer.rs`, hotkey registration
and cleanup, tray re-creation after an Explorer restart (handled inside
`tray-icon` 0.26), the foreground hook, window position restore, and
hidden/idle repaint scheduling.

---

## 2. Workstream A: correctness and robustness

### A1. Sessions that end while lowered come back lowered (Verified)

**Problem.** `Engine::apply_reductions` (`src/audio/engine.rs`, ~1334–1393) ends
with `self.reductions.ledger.retain(...)`, which forgets every session that no
longer exists without restoring its level. Windows persists a session's volume
per app, so an app closed during Solo (or while ducked) restarts at the lowered
level. If a reduction is then applied again, the lowered level becomes the new
"base" and the loss compounds. The same loss happens when an output device is
removed while its sessions are lowered (`open_session_sets` calls `release_in`
on sets whose endpoint is already gone, ~1406), and when shutdown detaches after
2 s (`AudioHandle::shutdown`, `src/audio/mod.rs` ~338).

**Change.**

1. In `src/audio/attenuation.rs` add a second, pure structure for restores that
   could not be written, for example:

   ```rust
   /// A base level owed to a session that ended (or became unreachable) while lowered.
   pub struct Pending { pub base: Level, pub written: Level }
   pub struct PendingRestores { /* map from a stable key to Pending, bounded */ }
   ```

   - Key: the pair (device id, **session identifier**). The session identifier
     is `IAudioSessionControl2::GetSessionIdentifier` (stable across launches),
     not `GetSessionInstanceIdentifier` (unique per instance, which the ledger
     uses today; see `engine.rs` ~1811). Read `Session::open` (~1888) to see how
     the instance id is obtained and add the session identifier next to it as a
     new `Session` field.
   - `Ledger::retain` must return (or hand to a callback) the entries it drops
     that are **not** `released`, so the caller can move them into
     `PendingRestores`. An entry whose `released` flag is set is dropped as today
     (the user changed the level; the user wins).
   - `PendingRestores::claim(key, current: Level) -> Option<Level>`: removes the
     entry; returns `Some(base)` only if `current` matches `written` (reuse
     `Level::matches`). If it does not match, the user or Windows changed the
     level and nothing is restored.
   - Bound the map (64 entries, drop the oldest) so it cannot grow without limit.
2. Keep the `PendingRestores` in `Runner` (it already holds state that survives
   rebuilds, such as `focus_rules` and `mic`), and pass it into the engine the
   same way that state is passed. Read how `Runner` hands `solo_level` /
   `focus_rules` to `Engine` and follow that pattern.
3. When a session is opened (the sync path in `SessionSet`, ~1800–1850), before
   reductions are applied, call `claim` with the session's current level; if it
   returns a base, write it with the existing `write_level`. After that the
   normal reduction logic runs (so if Solo is still on, the session is lowered
   again from the correct base).
4. In `release_in` (~1628): when the read of the current level fails, move the
   entry to `PendingRestores` instead of only logging.
5. Persist `PendingRestores` to `%APPDATA%\GlassMixer\pending-restores.json`
   (same directory as settings; find how the engine learns paths, and if it does
   not, pass the directory in at `AudioHandle` start). Write on change, with the
   same temp-file-then-rename approach as `SettingsStore::save`. Load at engine
   start. This covers crash, kill and the 2 s shutdown detach. A missing or
   unreadable file means "no pending restores"; never fail startup over it.
   Under `GLASS_MIXER_DEV_PROFILE` the file must live in the dev profile
   directory (follow `SettingsStore::default_dir`).

**Tests.**

- `attenuation.rs` unit tests: dropped unreleased entries are returned; released
  ones are not; `claim` restores only when the current level matches `written`;
  a claim removes the entry; the bound evicts the oldest.
- The existing test `sessions_are_independent_and_ended_sessions_are_forgotten`
  asserts the old behaviour. Update it to the new contract; do not delete it.
- Add an `#[ignore]` real-device test in `tests/audio_integration.rs` following
  the existing Solo tests: start Solo, end the lowered helper process, start it
  again, assert its level equals the pre-Solo level.

**Done when** the unit tests pass, the ignored integration test passes on a
machine with audio, and `docs/ARCHITECTURE.md` (the section describing the
ledger and the "kill" case) is updated to describe pending restores.

### A2. Installer hangs when upgrading from 0.1.0–0.3.0 (Verified)

**Problem.** `PrepareToInstall` in `installer/glass-mixer.iss` (~70–80) runs the
installed exe with `--exit` and `ewWaitUntilTerminated`. Versions before 0.4.0
do not understand `--exit`, so the old exe starts as the full app (close-to-tray
by default) and setup waits forever.

**Change.** Only run `--exit` when the installed exe is 0.4.0 or newer. Use Inno
Setup's `GetVersionNumbers(Exe, MS, LS)` (verify the exact name and signature in
the Inno Setup 6 help before use): major = `MS shr 16`, minor = `MS and $FFFF`.
Run `Exec` only if `(major > 0) or (minor >= 4)`. Otherwise do nothing and let
the existing `CloseApplications` handling deal with it. Keep CRLF line endings.

**Tests.** Extend `scripts/installer-smoke.ps1` if it has a place for upgrade
cases; otherwise document a manual check in `docs/TESTING.md`: install the
`v0.3.0` installer, exit the app, run the new installer, confirm it completes.

**Done when** the new installer upgrades over 0.3.0 with the old app both
running and not running, without hanging.

### A3. A failed settings save is never retried (Verified)

**Problem.** In `MixerApp` (`src/app.rs`, the save function around 420–435)
`self.dirty_since = None` runs before `self.store.save(...)`. If the save fails
(antivirus or a sync client holding the file), a toast is shown and the change
is lost on restart, because shutdown saves only when dirty.

**Change.** On error keep the settings dirty and retry with backoff: set
`dirty_since` so the next attempt is due after 2 s, doubling up to 30 s; reset
the backoff on success. Show the error toast once per failure streak, not on
every retry. At shutdown always attempt a final save when a previous save
failed.

**Tests.** `SettingsStore` already has a test-only failure switch
(`fail_backup_commit`). Add an equivalent way to make `save` fail once, and a
unit test proving a failed save leaves the app's dirty state set and a later
attempt succeeds. If the dirty logic is not reachable from a unit test, extract
it into a small pure struct (`SaveSchedule { dirty_since, backoff }`) and test that.

**Done when** the test passes and a manual check (make `settings.json` read-only,
change a setting, make it writable again) shows the change saved without restart.

### A4. Opening Devices while audio is unavailable shows a false error (Verified)

**Problem.** In `Runner::apply` (`src/audio/engine.rs` ~334–350)
`Command::LoadDevices` sets `devices_loaded = true` and then falls through to
the "audio service isn't available, so the change wasn't applied" notice when
`self.engine` is `None`.

**Change.** After setting the flag, `return` when `self.engine.is_none()`.

**Tests.** Add a unit test if `Runner` can be constructed in tests; otherwise
verify by reading the control flow and note that in the report.

### A5. An older build silently strips newer settings (Reported)

**Problem.** `Settings` deserialization is lenient (`lenient*` helpers,
`src/settings.rs` ~284–354): unknown enum values and bad list items are dropped
or defaulted. `SettingsStore::load` (~548) accepts a file whose `version` is
**greater** than `SETTINGS_VERSION`, and the next save (even a window move)
writes the stripped result. A test near line 840 asserts no backup is made in
this case. `transfer::import` already refuses newer versions and detects lossy
reads (`src/transfer.rs` ~57–110, helper `differs`).

**Change.**

1. In `SettingsStore::load`, when the file's version is greater than
   `SETTINGS_VERSION`, or when re-serializing the parsed settings differs from
   the original JSON in a way `transfer::differs` reports as lossy (`differs`
   is a private function in `src/transfer.rs`; make it `pub(crate)` or move it
   to a shared place rather than copying it), back up the
   original bytes before the first save. Reuse `back_up_before_migration`'s
   mechanism with a distinct name: `settings.v{file_version}.newer.backup.json`
   for a newer file, `settings.lossy.backup.json` for a same-version lossy read.
   Never overwrite an existing valid backup.
2. Return a `LoadOutcome` variant (or reuse the existing warning path, see
   `LoadOutcome::warning`) so the app shows one toast: "These settings were
   written by a newer Glass Mixer. A copy was kept as <path>."
3. As with migration, refuse to overwrite `settings.json` until that backup exists.
4. Update the test that asserts no backup is made to assert the new behaviour.

**Done when** loading a hand-written version-5 file with an unknown key and an
unknown enum value produces the backup, the warning, and an intact backup file
after the app saves.

### A6. Diagnostic report can leak names it promises to hide (Reported)

**Problem.** In `src/diagnostics.rs`: (a) `replace_ignoring_case` (~113) folds
case with `to_ascii_lowercase`, so a non-ASCII name logged in a different case
(for example a lower-cased exe path under `C:\Users\Élodie`) is not matched;
(b) `private_names` (~58) is built from the current snapshot and settings only,
so log lines that print an `AppKey` with `{key:?}` (search `src/audio/engine.rs`
for `{key:?}`) keep full exe paths outside the user profile and names of apps
that have since exited.

**Change.**

1. Make matching Unicode-aware. Offsets differ after Unicode lowercasing, so do
   not index the original with offsets from a lowercased copy. Compare
   char-by-char with `char::to_lowercase` on both sides, or build an index map
   from lowercased offsets to original offsets.
2. Add generic redaction that does not depend on a name list: replace any
   Windows path token (drive letter + `:\` or `:\\`, up to the next quote,
   whitespace or closing parenthesis) with `<path>\<file name>` reduced to
   `<path>`, and replace the payload of `Exe("...")`, `Package("...")` debug
   forms with a placeholder. Keep the existing placeholders list (`PLACEHOLDERS`)
   in sync.
3. Prefer fixing the source too: where the engine logs `{key:?}`, log a short
   stable hash of the key instead. Do this only for `log::` calls, not for
   user-facing notices.

**Tests.** Unit tests in `diagnostics.rs`: a non-ASCII profile name in mixed
case is redacted; an `Exe("d:\\games\\title\\x.exe")` log line is redacted; a
line with no private data is unchanged. Remember `\u{c3}` for A-tilde.

### A7. `--exit` gives up when another user runs the app (Reported)

**Problem.** `instance_pids` in `src/platform/window.rs` (~115–150) returns
`Err` for the whole search when `process_image_path` fails with anything other
than `ERROR_INVALID_PARAMETER`. A copy running in another user's session fails
with access denied, so `request_exit_and_wait` (`src/platform/exit_request.rs`
~119) returns `StillRunning` without signalling the current user's copy.

**Change.** In the `Err(e)` arm, treat `ERROR_ACCESS_DENIED` as "not ours, skip"
(compare against `ERROR_ACCESS_DENIED.to_hresult()`, imported from
`windows::Win32::Foundation`). Keep returning `Err` for other failures.

**Tests.** This needs a second user session to reproduce; add a unit test only
if the candidate-filtering decision can be extracted into a pure function
(`fn keep_candidate(result: Result<bool, HRESULT>) -> Result<bool, String>`).
Do that extraction; it is small.

### A8. Release workflow exposes signing secrets to every step (Verified)

**Problem.** `.github/workflows/release.yml` sets `SIGN_PFX_BASE64` and
`SIGN_PFX_PASSWORD` in the **job** `env` (~27–32), so they are visible to
third-party actions and to every dependency build script run by `cargo`.
Actions are referenced by mutable tags (`dtolnay/rust-toolchain@stable`,
`@master`, `Swatinem/rust-cache@v2`, `taiki-e/install-action@v2`,
`actions/upload-artifact@v4`, `actions/checkout@v7`), and
`choco install innosetup` is unversioned.

**Change.**

1. Move the two secrets from job `env` to the `env` of the single step
   "Import the code-signing certificate". That step's `if:` currently tests
   `env.SIGN_PFX_BASE64`; a step-level env is not visible to its own `if:`, so
   add a tiny earlier step that only sets an output/`GITHUB_ENV` flag
   (`HAS_SIGNING=true`) from the secret's presence, with the secret scoped to
   that step, and test the flag instead.
2. Pin every third-party action in both workflows to a full commit SHA with the
   tag in a trailing comment. **Look the SHAs up** (for example
   `gh api repos/<owner>/<repo>/git/ref/tags/<tag>`); never write a SHA from
   memory. While doing this, confirm `actions/checkout@v7` exists; if it does
   not, use the latest major that does.
3. Pin the Rust toolchain used for release builds to an explicit version (see
   C4) and pin the Inno Setup version in the `choco install` line.
4. Remove `Swatinem/rust-cache` from `release.yml` only (keep it in `ci.yml`).

**Done when** `release.yml` has no secret in job-level `env`, and every `uses:`
line in both workflows is a 40-character SHA. These workflows have never run on
GitHub; say so in your report rather than claiming they pass.

### A9. Launch-at-login is repointed by any copy that runs (Reported)

**Problem.** `startup::repair_path_if_enabled` (`src/platform/startup.rs` ~93)
rewrites the Run value whenever the registered command differs from the current
exe, so running a portable or `target\release` copy once redirects login launch
to it. It is called from `main` for every non-dev-profile start.

**Change.** Repair only when the registered executable no longer exists on
disk. Parse the exe path out of the registered command line (quoted path
followed by arguments; see `command_line()` in the same file for the exact
format written) and check `Path::exists`. Extract the parsing into a pure
function and unit-test it (quoted path, path with spaces, trailing `--minimized`).

### A10. A second launch can do nothing (Reported)

**Problem.** The single-instance mutex is per session
(`src/platform/single_instance.rs`), but `activate_existing_instance`
(`src/platform/window.rs` ~178) only accepts a window whose process has the
same exe **file name**. With a renamed portable copy running, launching the
installed app exits silently.

**Change.** Drop the file-name comparison in `visit`; the window title match
plus "not our own pid" is sufficient because the mutex already proved an
instance exists in this session. Keep `process_image_name` if other code uses it.

**Suspected, same file:** if the first instance is elevated, `CreateMutexW`
from a non-elevated second instance may fail with access denied, which the code
treats as "go ahead" (two instances writing `settings.json`). Reproduce by
running one copy as administrator. If confirmed, treat `ERROR_ACCESS_DENIED`
from `CreateMutexW` as "already running".

### A11. Uninstall leaves logs and reports behind (Verified)

**Problem.** The uninstaller's "remove settings" choice deletes only
`{userappdata}\GlassMixer` (`installer/glass-mixer.iss` ~93). Logs and
diagnostic reports are under `%LOCALAPPDATA%\GlassMixer` (`src/main.rs`
`log_dir`).

**Change.** In the same `if` block add
`DelTree(ExpandConstant('{localappdata}\GlassMixer'), True, True, True);` and
change the prompt to "Also remove your Glass Mixer settings and logs?".

### A12. Engine resilience (Suspected; prove before changing)

Each item needs evidence first. Use `examples/audio_probe.rs` and the app's log
(`%LOCALAPPDATA%\GlassMixer\glass-mixer.log`, or `<dev profile>\logs`).

1. **One dead secondary output rebuilds the whole engine.** A fatal HRESULT from
   any `SessionSet` propagates out of `refresh`/`snapshot`/`apply_reductions`
   to `Runner::fail` (~442). Reproduce: two outputs, Solo on, unplug the
   non-default one; look for "Audio devices are reinitializing". Fix if
   reproduced: on a fatal error from a non-default set, drop that set and mark
   `dirty.devices`; rebuild only for enumerator or default-endpoint failures.
2. **Self-wake loop on an unreadable session.** In `Engine::snapshot` (~862–874)
   a non-fatal `facts()` error sets `needs_sync` and sends `Msg::SessionsChanged`
   to itself with no rate limit. Fix regardless of reproduction (the logic is
   verified): route the self-send through the existing `Coalesce` type or skip
   re-sending for a session that failed on the previous pass. Also change
   `GetDisplayName()?` / `GetIconPath()?` in `Session::facts` (~1897) to
   `unwrap_or_default()` equivalents so one bad string does not hide a session.
3. **A broken default input leaves the app on "Connecting".** `Endpoint::open(..)?`
   for the default endpoints (~752, ~773) aborts the whole refresh. Fix: degrade
   per endpoint (leave that endpoint `None`, record the error for the UI) so the
   other flow keeps working.
4. **Solo ends silently on every rebuild.** Either keep the solo key in `Runner`
   and re-apply after rebuild, or push a notice "Solo ended because audio
   devices changed". Pick the notice (smaller, no behaviour change) unless the
   maintainer says otherwise; an integration test documents the current
   behaviour as intended.
5. **Version-string read ignores its length** (`src/audio/process.rs` ~144).
   Use the returned character count to build the slice
   (`std::slice::from_raw_parts(value, chars)`) instead of scanning for NUL.

### A13. Small leaks (Reported)

- `src/platform/icons.rs` (~437): the device context is not released when
  `CreateDIBSection` fails. Release it on that path (or wrap it in a Drop guard).
- `src/main.rs` `init_logging`: the log is rotated only at startup, so a tray
  process that runs for weeks grows it without bound. Rotate when the file
  passes 1 MB at write time (wrap the writer) or check size on a slow timer.

---

## 3. Workstream B: performance

Rule: measure before and after with a debug build under a dev profile (section
5). CPU samples on the maintainer's machine have multi-second outliers; compare
medians over several interleaved runs. None of these may reduce smoothness.

### B1. Settings cloned and compared every frame (Reported)

`MixerApp::ui` does `let before = self.settings.clone();` (`src/app.rs` ~896)
and compares at the end (~988). `Settings::extra` (`src/settings.rs` ~246) holds
unknown keys with no size bound, so a large import is cloned per frame.

Change: (a) cap `extra` on import and load (drop it entirely when its serialized
size exceeds 64 KB, and log that); (b) keep the clone-and-compare (it is the
simplest correct change detector) but skip it on frames with no input events
and no pending import. Confirm with the egui source how to tell a frame had no
input (`ctx.input(|i| i.events.is_empty() && !i.pointer.any_down())`; verify the
field names).

### B2. Disk read on every foreground change (Reported)

`Engine::key_for_pid` (`engine.rs` ~918) calls `process::query`, which reads the
exe's version resource from disk, only to compute an `AppKey`. Add an
identity-only query in `src/audio/process.rs` that skips the version resource,
and use it here.

### B3. One slider tick re-reads everything (Reported)

Each value change runs a full `Engine::snapshot()` (5–6 COM calls per session in
`Session::facts`, ~1950), then `apply_start_rules` and `apply_reductions` read
volume and mute again, then the snapshot is deep-compared while holding the
mutex the UI locks (`src/audio/mod.rs` ~204). Do this in three separate,
separately verified steps:

1. Compare outside the lock: build the new snapshot, compare, and only then lock
   to swap.
2. Put `outputs`, `inputs` and `properties` behind `Arc` so an unchanged list is
   a pointer copy.
3. Read each session's volume/mute once per pass and hand the values to the
   start-rule and reduction code instead of re-reading.

### B4. Meter publish allocates (Reported)

The meter map is cloned up to 30 times a second (~497). Change the key type to
something cheap to clone (`Arc<str>`-backed) or publish an index-ordered `Vec`.
**Do not change the 30 Hz rate.**

### B5. Device properties read eagerly (Reported)

`Engine::refresh` reads sound properties for the default endpoints on every
device event (~813) even if the Devices view was never opened. Gate it on
`devices_loaded`.

### B6. Mic ducking polls at 20 Hz forever (Reported)

Poll only while a capture session is active on the default input. Leave the
poll rate unchanged while active.

### B7. Binary size (Suspected)

The portable ZIP grew from 6.45 MB (0.4.0) to 11.74 MB (0.5.0). Measure with
`cargo bloat --release --crates` (install it if missing; it is a dev tool, not a
dependency). Candidates: wgpu's `vulkan` feature in `Cargo.toml` (DX12 with
WARP is available on every supported Windows build) and `simplelog`'s default
features (`termcolor` is unused). Remove a feature only if (a) the size drop is
real and (b) `scripts/lifecycle.ps1 -Renderer wgpu` still passes. Report the
numbers either way.

---

## 4. Workstream C: structure and hygiene

Pure moves: no behaviour change, no renames beyond what the move requires. Run
the full gate after each. Do these **after** workstream A's engine tasks so the
diffs do not collide.

### C1. Split `src/audio/engine.rs` (2,315 lines) into `src/audio/engine/`

| New file | Contents |
| --- | --- |
| `runner.rs` | `run`, `Runner`, `Dirty`, `Coalesce`, `push_coalesced`, `meters_differ`, `merge_peaks` |
| `error.rs` | `ApplyError`, `Context` |
| `endpoint.rs` | `Endpoint`, `list_devices`, `default_device_id`, `device_name` |
| `session.rs` | `SessionSet`, `ProcessEntry`, `Session`, `app_key`, `resolve_indirect` |
| `reductions.rs` | `Reductions`, solo toggles, `apply_reductions`, `release_in`, `write_level` |
| `profiles.rs` | profile, preset and start-rule code with their outcome types |
| `routing.rs` | `cycle_output`, `route_app`, `processes_of`, `read_routes` |
| `mod.rs` | `Engine`, `new`, `refresh`, `snapshot`, `apply`, `Drop` |

Confirm each listed item exists by searching for it before moving; if a name is
not found, report it rather than guessing its new home.

### C2. Move the hotkey editor into the UI layer

`src/platform/hotkeys.rs`: `HotkeyEditor` (struct ~663, impl ~687 to ~880) is
egui code. Move it to `src/ui/hotkey_editor.rs`, register it in `src/ui/mod.rs`,
and fix the imports in `src/app.rs` and `src/ui/settings_view.rs`. Move its
tests with it.

### C3. Remove duplication (one commit each)

- FNV path hash: `src/platform/tray.rs` (~26) and `src/platform/exit_request.rs` (~27).
- Temp-write-and-rename, three copies in `src/settings.rs` (~500, ~530, ~652):
  one `fn write_atomically(path, bytes) -> io::Result<()>`.
- Timestamped-file-and-prune: `src/diagnostics.rs` (~361) and `src/transfer.rs` (~183).
- Unix-now, three copies in `src/app.rs` (~494, ~558, ~579).
- `sessions.iter().flat_map(|set| set.sessions.iter())` (about 15 uses in the
  engine): one `fn all_sessions(&self) -> impl Iterator<Item = &Session>`.
- The "fatal → rebuild + notice" block, three copies in `Runner` (~275, ~313, ~356).

### C4. Repository hygiene

- `rust-toolchain.toml` pinning the channel to the stable version the
  maintainer builds with (run `rustc --version` and use that; do not invent one),
  with `components = ["rustfmt", "clippy"]`.
- `[lints]` in `Cargo.toml`: `rust.unsafe_op_in_unsafe_fn = "deny"`. Then try
  `clippy.undocumented_unsafe_blocks = "warn"`; if it produces more than a
  handful of warnings, leave it out and report the count instead of adding
  boilerplate `// SAFETY:` comments (the comment policy forbids filler).
- `.github/dependabot.yml` for `cargo` and `github-actions`, weekly.
- `SECURITY.md` (how to report a vulnerability; ask the maintainer for the
  contact address, do not invent one).
- New typed wrappers (optional, do last, one at a time): `DeviceId` with
  case-insensitive equality (device ids are compared with `==` in most places
  and `eq_ignore_ascii_case` in others), and `Volume` clamping to 0..=1 and
  rejecting NaN.

### C5. Test gaps

Add unit tests for: `app_key` precedence, `is_fatal`, `Reductions::active`,
`PresetOutcome::notice` branches, `startup::command_line`,
`window::same_executable`, `tray::write_wide` truncation. Give
`tests/audio_integration.rs` the RAII restore guard that
`tests/devices_integration.rs` already uses, so a failed assertion does not
leave Solo or mute changes behind.

---

## 5. Workstream U: interface, SF-Symbol iconography, and `/unslop-ui` cleanup

Goal: the calm, layered, high-craft feel of Apple's macOS Control Center and
System Settings, built natively from this project's own pieces (Polaris,
procedural SF-Symbol-proportioned vector icons with Segoe Fluent fallbacks, and
the Windows DWM backdrop) and audited against `/unslop-ui` (`U1`–`U20` and plain-speech
rules). No feature is removed, and every control keeps its keyboard behaviour,
focus ring and AccessKit accessible name.

### U0. Ground rules for every UI task

- **Fonts:** see 0.2. `Typeface::Polaris` stays the default. Do not bundle Apple's
  `SF Pro` or `SF Symbols` font files. Sizes stay on the existing scale (17
  heading, 14 body, 12–13 small) unless a task says otherwise. Always use
  `FontFamily::Proportional` or `widgets::semibold(size)` rather than `.strong()`.
- **Cut first, write little (`/unslop-ui`):**
  - No eyebrow labels (`U1`), decorative numbering (`U2`), redundant subtitles
    under headings (`U3`), botsplaining captions/tooltips (`U4`), leaked design
    or implementation notes (`U5`), chained `·` status strings beyond two facts
    (`U6`), labels that repeat placeholders/buttons/headings (`U7`), or em
    dashes (`unslop 13`).
  - Use sentence case everywhere (`unslop 17`), active voice (`unslop 29`), one
    idea per sentence (`unslop 28`), and proper singular/plural helper formatting
    instead of `(s)` or `(ies)`.
  - Use **"Profiles"** consistently in all visible UI text (`unslop 11`); do not
    mix "Presets" and "Profiles" on screen (keep `View::Presets` and the
    `presets` JSON key unchanged in code/schema).
- **Accessibility must not regress:** every interactive widget keeps
  `response.widget_info(..)` (or `widgets::set_name`), a `focus_ring`,
  `scroll_on_focus`, and keyboard operation. Everything must fit at
  `MIN_WIDTH` (360) in both densities.
- **High contrast:** `Palette.high_contrast` selects an opaque palette where
  colour is never the only signal. Every new visual (shadows, translucent fills,
  in-track meters, top highlights) must have a plain high-contrast path: solid
  fill, 1 px `p.border` stroke, no shadow.
- **Animations** must respect the system setting (`style.animation_time == 0.0`
  when off). Derive every animation duration from `ui.style().animation_time`.
- After each UI task, take screenshots in dark and light with the script in
  section 5.2 and look at them.

### U1. Design tokens and 4-point spacing scale (do first)

File: `src/ui/theme.rs`.

1. **4-point spacing grid (`/unslop-ui` `U18`):**
   - `Metrics` today: Comfortable `pad 14, gap 10`, Compact `pad 10, gap 6`.
   - Change to:
     - Comfortable: `pad: 16.0, gap: 12.0, row_height: 52.0, icon: 28.0, button: 32.0, radius: 12, radius_inner: 8`
     - Compact: `pad: 12.0, gap: 8.0, row_height: 40.0, icon: 22.0, button: 28.0, radius: 10, radius_inner: 6`
   - In `theme::apply`:
     - `style.spacing.item_spacing = Vec2::new(8.0, if m.row_height > 46.0 { 8.0 } else { 4.0 });`
     - `style.spacing.button_padding = Vec2::new(10.0, 6.0);`
     - `style.spacing.scroll = egui::style::ScrollStyle::floating();`
2. **Surface & depth tokens (`/unslop-ui` `U17`):**
   - `selection: Color32` — neutral pill fill for the active segment/tab. Dark:
     white at alpha 34 (glass) / `0x383844` (opaque). Light: `0xffffff`. High
     contrast: `accent`.
   - `selection_text: Color32` — Dark and light: `text`. High contrast: `on_accent`.
   - `highlight: Color32` — 1 px inner top-edge specular highlight on cards.
     Dark: white at alpha 24 (glass) / 16 (opaque). Light: white at alpha 160
     (glass) / 120 (opaque). High contrast: `border`.
   - `shadow: Color32` — subtle neutral shadow for floating surfaces only
     (popups, toasts, active pill/thumb). Dark: black at alpha 72; light: black
     at alpha 20; high contrast: `Color32::TRANSPARENT`.
   - `switch_off: Color32` — off-state switch track fill. Dark: white at alpha
     44. Light: black at alpha 36. High contrast: `0x808080`.
3. **Motion (`/unslop-ui` `U20`):**
   - In `theme::apply`, set `style.animation_time = if animations { 0.16 } else { 0.0 };`.
   - Add `pub fn ease(t: f32) -> f32 { egui::emath::easing::cubic_out(t) }`.

### U2. SF-Symbol-proportioned vector symbol system & app icon artwork

Files: `src/ui/widgets.rs`, `src/icon_art.rs`, `src/platform/icons.rs`.

1. **Procedural SF-Symbol-style vector icons in `src/ui/widgets.rs`:**
   - Add `pub enum Symbol` and `pub fn paint_symbol(painter, rect, symbol, color)`
     using anti-aliased `Painter` primitives (filled convex polygons, rounded
     capsules, circular arcs with rounded stroke caps):
     - `SpeakerWave(u8)` (`0..=3`) and `SpeakerSlash`: filled rounded speaker cone
       anchored at a fixed x-position so the cone never shifts horizontally as
       volume changes, plus 1–3 concentric arcs or a diagonal slash.
     - `Mic` and `MicSlash`: filled vertical capsule, U-cradle arc, stem and base,
       plus diagonal slash when muted.
     - `Headphones { filled: bool }`: 180° headband arc + earcup capsules.
     - `SlidersHorizontal`: three horizontal lines with offset circular knobs
       for the Balance & sound properties button.
     - `ArrowClockwise`: circular arc + arrowhead for Refresh buttons.
     - `MagnifyingGlass` and `XMark`: search/clear icons.
     - `ChevronLeft`, `ChevronRight`, `ChevronUp`, `ChevronDown`: 90° chevrons
       with rounded caps.
     - `Checkmark`, `ExclamationTriangleFill`, `Display`, `HifiSpeaker`, `SystemSounds`.
   - Helper `pub fn device_symbol(flow: Flow, name: &str) -> Symbol` chooses
     `Headphones`, `Display`, `HifiSpeaker`, or `Mic` from the device name.
2. **Replace legacy Windows XP `System sounds` raster icon (`src/platform/icons.rs`, `src/ui/mixer.rs`):**
   - Return early in `IconService::request` for `AppKey::System` so the legacy
     `AudioSrv.dll` monitor bitmap is never loaded.
   - Paint `AppKey::System` with a calm tonal squircle tile and `Symbol::SystemSounds`.
   - Lower saturation on `fallback_tile` and add a 1 px inner border.
3. **App & tray icon artwork (`src/icon_art.rs` `render_app_icon`):**
   - Continuous squircle (`corner = 0.225 * s`) with single-hue tonal depth (`U17`)
     and a subtle top-edge inner highlight.
   - Three vertical fader tracks with circular fader knobs (`slider.vertical.3`).
   - Knockout trench under the mute slash so 16×16 tray icons stay sharp.

### U3. Card surface, hairline dividers, and shared layout helpers

File: `src/ui/widgets.rs`.

- Update `card(look: &Look)` to use `CornerRadius::same(look.m.radius)` and
  4-point inner margins (`12` Comfortable / `8` Compact) with no dark drop
  shadow over glass (`U17`). Add `card_highlight(ui, look, rect)` / `row_divider(ui, look)`
  and `count_label(n, singular, plural)`.

### U4. Neutral animated `segmented` control & top navigation bar

Files: `src/ui/widgets.rs`, `src/app.rs`, `scripts/ui-automation.ps1`.

- Neutral sliding pill in `segmented` using `p.selection` and
  `animate_value_with_time` (`U17`, `U20`).
- Replace the four `ui.selectable_value` tabs in `src/app.rs` with a centered
  `widgets::segmented` control labelled `Mixer | Devices | Profiles | Settings`
  (`unslop 11`), preserving `LoadDevices` and `startup::state()` side effects.
- Update `scripts/ui-automation.ps1` to recognize `'Profiles'` and `RadioButton`
  / `SelectionItemPattern`.

### U5. Control Center slider & switch (`src/ui/widgets.rs`)

- Track height `6.0`, `CornerRadius::same(3)` in both `Slider::show` and
  `unavailable_slider`.
- In-track live level meter overlay (white at alpha 70 dark / 110 light; keep
  under-track bar in high contrast).
- Single clean white thumb (`radius 8..9`, subtle shadow, light-mode 1 px
  `p.border` ring, no inner dot).
- Balance sliders: 1 px center tick mark + magnetic center detent (`|v| < 0.03`
  snaps to `0.0` while dragging). Volume sliders: double-click resets to `100%`.
- `toggle_row`: 4-point vertical offsets, `40 × 22` track cross-fading between
  `p.switch_off` and `p.accent`, white knob with subtle shadow.
- `percent_label`: fixed `42.0 px` right-aligned `semibold(13.0)` slot.

### U6. Mixer view (`src/ui/mixer.rs`)

- Replace raw Segoe glyphs with `Symbol` painting and `device_symbol` for output
  endpoints.
- Replace the `"Balance and sound properties"` text link with a trailing
  `Symbol::SlidersHorizontal` `icon_button` at the end of the output volume row.
- Empty input card: single compact muted row (`Symbol::MicSlash` + `"No input device"`)
  when no input devices exist (`U7`, `unslop 29`).
- Remove raw WASAPI GUID `.on_hover_text(&d.id)` tooltips (`U5`) and botsplaining
  hover text (`U4`, `U7`).
- Quiet idle rows: omit `"Idle"` visually and center the app name while keeping
  `"Idle"` in AccessKit `a11y`. Cap row status lines to at most two `·`-separated
  facts (`U6`), truncate to `name_w - 4.0`, paint a subtle hover/focus/expanded
  disclosure chevron, and dim idle app icons.
- Offline pins: `semibold(14.0)` title, remove duplicate `"No active audio session"`
  (`U7`), move help text to `Unpin` hover (`U4`), fix em dashes (`unslop 13`).
- Expanded details: recessed sub-frame (`radius_inner`, 1 px `p.border`), hide
  redundant `"Follows the system default output."` label when `Route::SystemDefault`
  is shown in the combo (`U7`), shorten tooltips (`U4`, `U5`), fix stream `·`
  status string (`U6`).

### U7. Devices view & Sound properties (`src/ui/mixer.rs`, `src/audio/properties.rs`)

- Match Settings typography (`semibold(18.0)` title with `ArrowClockwise` refresh
  `icon_button`, `semibold(13.0)` muted section headings `"Outputs"` and `"Inputs"`).
- Delete redundant `"Apps in the Mixer use the default output..."` subtitle (`U3`, `U5`).
- Device cards: `device_symbol` + `semibold(14.0)` name without raw GUID tooltip
  (`U5`), clean `"Balance: unavailable (not stereo)"` (`unslop 13`, `unslop 17`).
- Sound properties: omit spatial-audio readout on input devices (`U5`), delete
  the three Windows-caveat filler paragraphs (`U4`, `U5`, `U7`, `unslop 29`, `unslop 30`),
  group `"Open Windows sound settings"` and `"Open mono audio settings"` into one
  wrapped button row.

### U8. Profiles view (`src/ui/presets.rs`)

- Pass `&Look` into `presets::show`, wrap creation form and each saved profile in
  `card(look)` with `look.m.gap` spacing (`U18`), use `semibold(14.0)` names and
  muted small captions.
- Delete the 2-sentence intro paragraph (`U3`, `U5`, `U7`) and the 69-word
  automatic-rules wall (`U4`, `U5`, `unslop 28`, `unslop 29`).
- Rewrite the enabled auto-rule card note without an em dash (`unslop 13`) and
  replace `(s)` with `count_label`.

### U9. Settings, Global Shortcuts & Transfer (`src/ui/settings_view.rs`, `src/platform/hotkeys.rs`, `src/transfer.rs`, `src/ui/mod.rs`)

- `settings_view`: align spacing to the 4-point grid, add `row_divider` between
  logical groups inside cards, clean up passive/botsplaining/leaked copy across
  Appearance, Behavior, Audio, Backup, and About, and keep descriptive AccessKit
  names while simplifying visible mic-ducking checkboxes to `&pin.name` (`U7`).
- `hotkeys.rs`: delete the 3-sentence intro (`U4`, `U5`, `U7`), fix inverted
  `"Pin an app..."` visibility, replace `—` with `:` (`unslop 13`), add
  press-to-record chord capture on focused shortcut inputs, remove redundant
  `Bind`/`Clear` tooltips (`U7`), and fix `(s)` / `(ies)` pluralization.
- `transfer.rs`: replace `(s)` and leaked `{:?}` debug enum formatting
  (`"typeface System"`) with user-facing labels (`U5`).
- `ui/mod.rs`: active-voice `GlassStatus::description` (`unslop 29`).

### U10. Tray & Toasts (`src/platform/tray.rs`, `src/audio/model.rs`, `src/audio/engine.rs`, `src/app.rs`)

- `tray.rs`: `"Open mixer"` sentence case (`unslop 17`).
- `model.rs`: replace em dashes in `tray_tooltip` with colons (`unslop 13`).
- `engine.rs`: replace `app(s)` and `"Preset has no valid name."` (`unslop 11`).
- `app.rs`: use `p.shadow` on toasts (`U17`) and `"Dismiss"` tooltip (`U4`).

### U11. Unified title bar spike (`examples/glass_spike.rs`)

Keep the existing 8-criteria Windows 11 spike specification. Do it last; ship
only if all 8 criteria (Mica/Acrylic backdrop, rounded corners + shadow, all-edge
resize, Aero Snap, mixed-DPI drag, close-to-tray, geometry restore, Narrator
names) pass.

### 5.2 How to look at the UI

Build once: `cargo build` (debug). Then run `scripts/verify-ui.ps1` or capture
with a throwaway `GLASS_MIXER_DEV_PROFILE`:

```powershell
$env:GLASS_MIXER_DEV_PROFILE = "<absolute scratch dir>\profile"
$env:GLASS_MIXER_DEV_EXIT_AFTER_MS = "45000"   # the app exits by itself
$p = Start-Process .\target\debug\glass-mixer.exe -PassThru
Start-Sleep 4
.\scripts\capture-window.ps1 -ProcessId $p.Id -Output "<scratch dir>\mixer.png"
```

---

## 6. Order of work

Each line is one reviewable unit. Run the gate (section 8) after each.

1. A4, A11 (one-line fixes; warm-up, low risk).
2. A3, A2, A7, A9, A10.
3. A1 (largest correctness change; touches `attenuation.rs` and `engine.rs`).
4. A12, A13.
5. A5, A6.
6. A8, C4.
7. B1–B6, each separately, with before/after numbers. B7 last.
8. C1, C2, C3, C5 (pure moves; only after A and B are done in the same files).
9. U1 → U2 → U3 → U4 → U5 → U6 → U7 → U8 → U9 → U10.
10. U11 spike; ship only if it passes its eight criteria.

Dependencies: U2–U10 need U1. U4 needs U3. U7 and U8 need the `section` /
`labeled` extraction described in U3. C1 must not start while any A or B task
in `engine.rs` is open.

Changelog: add each user-visible change to `CHANGELOG.md` under a
`## Unreleased` heading at the top (there is none at `374fbba`; create it above
`## 0.5.0 (2026-11-06)`), following the file's existing format. Do not change the
version number; `scripts/check-version.ps1 -AllowUnreleased` is what CI runs.

---

## 7. API reference checked for this plan

All paths are under the cargo registry (`...\registry\src\index.crates.io-*\`).
These exist in the pinned versions. Read the signature before calling; anything
not in this table or already used in the repository must be looked up.

| API | Defined in |
| --- | --- |
| `Context::animate_bool`, `animate_bool_with_time`, `animate_bool_with_easing`, `animate_bool_with_time_and_easing`, `animate_value_with_time` | `egui-0.36.2/src/context.rs` |
| `emath::easing::{cubic_out, quadratic_out, cubic_in_out, ...}` (all `fn(f32) -> f32`) | `emath-0.36.2/src/easing.rs` |
| `ScrollStyle::{solid, thin, floating}`; `Spacing::scroll` | `egui-0.36.2/src/style.rs` |
| `Style::animation_time` (egui default 0.2) | `egui-0.36.2/src/style.rs` |
| `ViewportBuilder::{with_decorations, with_transparent, with_resizable, with_taskbar, with_window_level}` | `egui-0.36.2/src/viewport.rs` |
| `ViewportCommand::{StartDrag, BeginResize(ResizeDirection), Minimized(bool), Maximized(bool), Decorations(bool)}` | `egui-0.36.2/src/viewport.rs` |
| `Shadow { offset: [i8; 2], blur: u8, spread: u8, color: Color32 }` | `epaint-0.36.2/src/shadow.rs` |
| `Painter::{add, set, rect, rect_filled, rect_stroke, circle_filled, line_segment, hline, text, galley, with_clip_rect, set_opacity, multiply_opacity}` | `egui-0.36.2/src/painter.rs` |
| `Mesh::{colored_vertex, add_colored_rect, add_rect_with_uv}` (for gradients) | `epaint-0.36.2/src/mesh.rs` |
| `Ui::{set_opacity, multiply_opacity, scope_builder, interact, allocate_exact_size, allocate_rect}` | `egui-0.36.2/src/ui.rs` |
| `ComboBox::{icon, height, close_behavior, popup_style}` | `egui-0.36.2/src/containers/combo_box.rs` |
| `CollapsingState::{load_with_default_open, is_open, set_open, toggle, openness, show_body_indented, show_body_unindented}` | `egui-0.36.2/src/containers/collapsing_header.rs` |
| `TextFormat::{extra_letter_spacing, line_height}` | `epaint-0.36.2/src/text/text_layout_types.rs` |
| `eframe::App::{logic, ui, on_exit, clear_color, raw_input_hook}` | `eframe-0.36.2/src/epi.rs` |

`ViewportCommand::Close` also exists in `viewport.rs`.

Not checked (look up before use): the `ResizeDirection`
variants, anything in `egui-winit`, every Win32 call not already in the code,
Inno Setup's `GetVersionNumbers`, and every Segoe glyph code point not already
in `widgets::glyph` (render it and look at it).

---

## 8. Verification gate

Run after every unit of work. Check the exit status of every step; run them as
separate commands, not chained through pipes.

```
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

When the task touched audio code, on a machine with audio devices:

```
cargo test --test audio_integration -- --ignored --test-threads=1
cargo test --test devices_integration -- --ignored --test-threads=1
```

When the task touched app lifecycle, rendering or the window:

```
powershell -File scripts\lifecycle.ps1
powershell -File scripts\lifecycle.ps1 -Renderer wgpu
```

When the task touched UI: the screenshots from section 5.2, plus
`scripts\verify-ui.ps1` (read its parameters first).

When the task touched the installer: `scripts\installer-smoke.ps1`.

Read each script's `param(...)` block before running it; do not assume flags.

### What to put in your report for each task

- What you changed, by file and symbol.
- The gate results, with the actual pass/fail of each command. If a command
  failed, paste the failing output; do not summarise it away.
- What you could not verify and why (no second audio device, no second user
  session, workflow never run on GitHub, backdrop not capturable).
- For Suspected items: whether it reproduced, and the evidence.
- Anything in this plan that turned out to be wrong.

Do not write "done" for a step you did not run.
