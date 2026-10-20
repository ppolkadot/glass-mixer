# Glass Mixer: follow-up review of R1–R9 updates

Review date: 2026-10-17.

Reviewed worktree: C:/Users/PC/.gemini/antigravity/worktrees/glass-mixer/glass_mixer_audit
Branch: glass_mixer_audit
Committed baseline: 107a305f2da9524d478bfabd64452329a02deab9
Reviewed changes: the 19 modified tracked files in that worktree, not a new commit.

The experiments checkout still points to 107a305 and contains the previous review document. The implementation updates are currently uncommitted in the audit worktree.

## Verdict

Substantial progress. The original normal-migration data-loss path, delayed first audio, stale accessibility-tree results, unsupported-balance wrapping and injector-unwind cleanup have been corrected. Meaningful regression tests were added.

Recommend another small correction pass before merge: fix live-marker eviction and the two verification false-success paths below. Also finish the malformed-settings recovery/presentation edges and preserve historical test evidence. No Critical/P0 or High/P1 defect was established.

This review changed no tracked application files.

## Remaining findings

### F1 — Important/P2: tracker capacity can make a considered live process qualify again

Location: src/audio/preferences.rs:221–224.

The tracker now uses the correct (AppKey, pid, created) identity, but it removes the oldest marker when it reaches 1,024 entries without establishing that the process exited.

Safe, hardware-independent reproduction against the updated compiled library:
1. Arm a tracker at 100.
2. Observe PID 1, created at 101, with untouched=false. It is correctly skipped and recorded.
3. Observe 1,024 other distinct process instances without explicit exit reclamation.
4. Revisit PID 1 with its original creation time and an untouched current session.
5. The tracker returns true: the existing process is eligible again.

Observed output:
- first_seen_touched=false
- same_running_process_qualifies_after_capacity=true

This can reapply after a session is recreated, or retry a rule that was intended to be considered only once. It breaks the lifecycle requirement even though the original delayed-audio ordering case now passes.

Correction: preserve live considered/intervened markers. Reclaim confirmed exited instances; if a hard bound is necessary, handle saturation conservatively rather than evicting markers and treating known processes as new.

Acceptance: exceed capacity, revisit the oldest live process, and verify it remains ineligible. Include a previously intervened or failed process and a recreated audio session. Retain the delayed-audio, PID reuse, equal-timestamp and sibling-intervention tests.

### F2 — Important/P2 for verification: UI Automation can report traversal success after skipping it

Location: scripts/ui-automation.ps1:39–58, :73–97.

The script catches a missing Settings InvokePattern, leaves settingsInvoke null, skips the entire Settings/HelpText/Tab block, then prints unconditional success.

Its Tab loop also checks only whether the folder button's rectangle lies within the outer window. It does not establish that the target started off-screen or require that the target actually received keyboard focus. An already-visible button or a preserved bottom scroll position can pass without testing focus-driven scrolling. The outer window rectangle is also weaker than the client/scroll viewport.

Correction:
- Require Settings invocation for the advertised checks, or explicitly report a skipped check without success.
- Reset/establish the starting scroll position and confirm a target that requires scrolling.
- Require the target to acquire keyboard focus and become visible within the relevant viewport.
- Check failed posted-key operations where applicable.
- Emit success only for checks actually completed.

The headless focus-scroll tests are useful and pass. This finding concerns the stronger native verification claim, not an established production focus-scroll failure.

### F3 — Important/P2 for verification: failed input injection is silently replaced with a synthetic hotkey message

Location: src/platform/hotkeys.rs:834–865, especially :863–864.

When SendInput returns zero, the test helper posts WM_HOTKEY directly to the worker. That can satisfy the delivery assertion when no key was injected and no Windows hotkey registration was exercised. The test/documentation still describe real input-queue pressure.

Correction: separate deterministic posted-message pressure from the ignored desktop-input test. The real-input case must require full injection success or explicitly report the environment unavailable. Do not count the posted-message path as proof of keyboard delivery.

Acceptance: force zero injection through a seam and verify that real-delivery success is not reported. Retain the new RAII cleanup in every path. Handle partial injection accurately as well.

The original R7 panic/thread leak is fixed; this is a new evidence-quality issue introduced while hardening that test. No desktop injection tests were run in this review.

### F4 — Minor/P3: invalid-first duplicate settings can hide the active shortcut

Location: src/platform/hotkeys.rs:511–539; validation at :392.

The editor still chooses the first raw binding for the current label and draft. Validation can reject that first entry as invalid and accept a later entry for the same action.

Example: VolumeUp→garbage, followed by VolumeUp→Ctrl+F1. The second chord can be active, but the editor displays the invalid first chord as Not active and never identifies the active chord in that row.

The originally reported valid-first/duplicate-second case is fixed.

Correction: consistently select the retained validated/active binding, or display accepted and rejected entries explicitly. Add an invalid-first/valid-second presentation fixture alongside the existing duplicate fixture.

### F5 — Minor/P3: schema-zero migration cannot retry saving after the obstruction is removed

Location: src/settings.rs:422–430, particularly the 1..SETTINGS_VERSION range at :426.

Lenient version parsing converts an invalid value to zero. If initial backup creation fails, the loaded in-memory settings retain version zero. Removing the obstruction and calling save still does not retry: disk version zero is excluded from the backup branch, so save refuses because the backup does not exist.

Reproduced with an isolated file containing {"version":"bad","theme":"dark"} and a directory at settings.v0.backup.json. After removing the directory, save still returned the backup-required error. Restarting load can recover; ordinary schema-1 migration is protected and retries successfully.

Correction: make retry behavior consistent for leniently recovered zero versions, or carry the pending original migration version explicitly. Add the failure/removal/save-retry case. Preserve the source until backup succeeds.

### F6 — Minor/P3: current test results replace a historical dated record

Location: docs/TESTING.md:3 and :60–69.

The section remains dated 2026-10-16, but its previous 71-library/2-binary results were replaced with 83/3 and checks for these later updates. Keep the original historical record and add a dated current verification section with the reviewed commit/worktree state.

The cited hosted CI run is for committed 107a305. It cannot validate the current uncommitted changes.

## Status against the earlier prompt

| Item | Current assessment |
| --- | --- |
| R1 migration preservation | Corrected for valid old-schema files: warning propagation, original schema retention, synced temporary backup, save gating, obstruction/partial failure and retry tests. F5 remains for malformed version zero. |
| R2 process-instance tracking | Original delayed-audio scenario corrected. Equal timestamps, PID identity and sibling interventions improved. F1 prevents full acceptance. |
| R3 visible keyboard focus | Application implementation and headless keyboard tests look sound. F2 means native verification is not yet reliable enough to close the stronger claim. |
| R4 final accessibility tree | Corrected: final snapshot, reachable traversal, names separate from values, Link coverage, disappearing-control test. |
| R5 expanded non-stereo layout | Corrected: unavailable text wraps; expanded mono/stereo/error fixtures cover actual card indentation and operation. |
| R6 binding status | Original valid-first duplicate case corrected; F4 remains for invalid-first input. |
| R7 injector cleanup | Original failure-path leak corrected with a Drop guard and an unwind test. F3 undermines the desktop-delivery claim. |
| R8 documentation | Initial-level rule restrictions, transparency wording and top-level unknown-field wording improved. F6 remains. |
| R9 final evidence | Fresh local checks passed. Current hosted CI, MSRV and wider runtime coverage remain unverified. |

## Evidence from this review

Fresh checks against the audit worktree:
- cargo fmt --check: passed.
- cargo clippy --locked --all-targets -- -D warnings: passed.
- cargo test --locked: 83 library tests passed, 3 ignored; 3 binary tests passed; 1 source-encoding test passed; 8 hardware audio tests ignored.
- cargo build --release --locked: passed.
- PowerShell parser checks for scripts/*.ps1: passed.
- git diff --check: passed.
- Isolated pure-library probe: original delayed-audio case now returns true for both newer-first and older-later processes.
- Isolated pure-library probes reproduced F1 and F5. Settings data stayed under target; no real user settings were changed.

No new native GUI traversal, Narrator speech, desktop input injection, live audio mutation, installer upgrade/package run, or hosted CI execution was performed here. Only stable Rust is installed; a Rust 1.95 MSRV run was not performed.

## Useful improvements to preserve

- Engine execution helpers now let the suite exercise partial nonfatal failures and fatal-error propagation without audio hardware.
- Migration failures now surface to startup and block unsafe replacement.
- The per-process tracker fixes the original observation-order bug, apart from its saturation policy.
- AccessKit tests now operate on the final reachable tree and examine input values correctly.
- Focus scrolling occurs on gained_focus rather than every focused frame; pointer scrolling is not continually fought.
- Expanded Devices tests cover mono inputs/outputs, stereo behavior, errors and narrow cards.
- Shortcut status matches action plus chord for the reported duplicate case.
- Injector cleanup is owned by RAII and tested during unwinding.
- Ambient WGPU_BACKEND=gl is stripped outside the development profile; the release renderer flag makes deliberate fallback testing possible.

## Additional non-blocking notes

- The new migration warning is a ten-second startup toast. On minimized launch it can expire unseen. Consider retaining it until visible or routing it through normal hidden-window notifications; data remains protected.
- The forward Mixer traversal fixture describes an offline pin as the bottom target, although pins precede unpinned live apps. Use the last live app as the forward endpoint and test the pin separately. Reverse traversal may already cover those rows, so this is a precision/coverage improvement.
- Retract the earlier speculative fixed-master scroll-target-leak concern: a probe left the app-list offset unchanged while the fixed slider gained focus, and egui saves/clears outside scroll targets at ScrollArea entry. It is not a remaining defect.
- Do not elevate the earlier watched-PID-reuse speculation into an established runtime finding. ProcessWatch owns a process handle; the added stale-entry check is defensive hardening. No runtime reproduction was established.

Next merge review should verify F1–F3 corrected, F4–F6 addressed or explicitly deferred, targeted regressions passing, and the final committed changes covered by hosted CI. Optional new product features from the earlier prompt remain subject to later approval.

