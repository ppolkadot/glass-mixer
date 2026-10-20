# Glass Mixer: review of the latest follow-up updates

Date: 2026-10-18.
Reviewed source: C:/Users/PC/.gemini/antigravity/worktrees/glass-mixer/glass_mixer_audit.
Branch: glass_mixer_audit. Committed base: 107a305.
The reviewed fixes are still uncommitted modifications in that worktree.

Input summary: C:/Users/PC/.codex/attachments/88830027-525c-4c64-a545-ec7caf7d62b0/Pasted text.txt.

## Decision

The earlier F1–F6 corrections are substantially implemented. No new High/P1 application defect was established. One Important/P2 concurrency defect remains in the new test-helper error cleanup and should be corrected before merge. There are also two Minor/P3 limitations in notification lifetime and test-coverage reporting.

The supplied summary largely matches the source. Its claim that hidden toast expiry is paused is stronger than the implementation, and its deterministic posted-message test can still return success without exercising pressure when the test chord is occupied.

This review made no application-source changes.

## Remaining findings

### N1 — Important/P2: joining the injector while holding its error mutex can deadlock

Location: src/platform/hotkeys.rs:978–980; worker error publication at :971.

The new helper contains:

    if let Some(err) = first_err.lock().clone() {
        guard.stop_and_join();
        service.shutdown();
        return Err(err);
    }

The temporary MutexGuard survives through the if-let body. Meanwhile the injector's error branch locks the same shared error slot.

Concrete race:
1. A zero/partial injection error has been recorded.
2. The caller enters this if-let and retains the error mutex.
3. The injector has passed its stop check and is about to publish another error.
4. stop_and_join sets the stop flag and waits for the injector.
5. The injector cannot finish publishing because the caller retains its mutex; the caller cannot leave the if body because join is waiting.

This affects the ignored real-input error path and the default mocked zero/partial-injection test. It does not affect the production hotkey service. The default suite passed in this review; it does not reliably force this scheduling window.

A safe isolated probe confirmed the relevant Rust lifetime: the mutex remains locked in the if-let body, the worker's error publication blocks during that body, and the worker completes once the temporary guard is released. The probe intentionally joined outside that body to avoid creating a stuck process. This is lock-lifetime evidence, not a claim that the complete application hung.

Required correction:
- Clone the error into a separate local, release the lock, then enter the conditional and stop/join the worker.
- Keep all joining outside locks needed by the joined thread.
- Add a synchronization-based regression that coordinates overlapping error publication and cleanup. Fixed sleeps alone do not establish this ordering.
- Retain the current RAII guard and strict rejection of failed/partial injection.

### N2 — Minor/P3: hidden toast expiry is extended on callbacks, rather than paused

Location: src/app.rs:620–626; expiration filtering at :439.

The new hidden branch moves each deadline to at least seven seconds after that hidden logic callback. It does not preserve remaining display time across a hidden interval without callbacks.

If the app stays hidden and idle longer than seven seconds, and its next event reveals the window, logic observes shown=true and bypasses the extension. draw_toasts then removes the already-expired warning. This still matters when tray notifications are disabled or suppressed.

Routing the startup warning through MixerApp::toast is a useful improvement. The remaining issue is its in-window lifetime, not migration data safety.

Correction:
- Track visibility transitions/remaining visible duration, or begin the warning's expiry when it is first shown.
- Avoid introducing a hidden timer or repaint loop.
- Test a hidden interval longer than the deadline with no intermediate callbacks, followed directly by reveal.
- Align the summary and docs/TESTING.md claim with the actual behavior.

### N3 — Minor/P3: posted-message pressure coverage is conditional on a global shortcut

Location: src/platform/hotkeys.rs:1083–1095.

The new non-ignored pressure test still registers Ctrl+Alt+F23 through Windows. If another process owns that chord, active is empty and the test returns successfully before exercising pressure or checking shutdown duration.

This is not the old false keyboard-delivery fallback: that fallback has been removed correctly. It is a separate coverage gap in the test described as deterministic.

Correction:
- Make deterministic posted-message pressure independent of OS chord ownership through a suitable dispatch seam; or
- Treat it as an environment-dependent integration check and make skipped coverage explicit.

A conflict should not look identical to a completed pressure assertion. Keep real SendInput delivery as a separate ignored check.

## Status of the supplied summary

| Summary item | Review result |
| --- | --- |
| F1 marker preservation | Resolved. Live markers are not evicted. Saturation is latched and unknown instances are skipped conservatively. Rearming resets the latch. Capacity/intervention/failure regressions are present and passed. |
| F2 strict native UI verification | Resolved for the previously reported paths. Navigation must support Invoke/Toggle or fail; descriptions use the actual FullDescription mapping; the target starts off-screen and unfocused, then must gain focus and move inside client bounds. Fresh checks passed under both renderers. |
| F3 input-delivery honesty and cleanup | The false SendInput-to-WM_HOTKEY fallback is removed. Zero/partial input is rejected and partial input attempts release cleanup. Real and posted paths are separated. N1 remains in error cleanup; N3 affects the deterministic-coverage claim. |
| F4 primary binding selection | Resolved. Active or retained validated binding is selected consistently for display and initial draft. The invalid-first/valid-second fixture passes. |
| F5 schema-zero save retry | Resolved. Save uses the same lenient settings parsing as load, and the malformed-version obstruction/removal/retry case passes. |
| F6 history and auxiliary notes | Historical results are restored and a separate current section exists. Forward traversal reaches the bottom live row. Startup warnings reach toast/notification handling; N2 remains for hidden expiry. |

Summary/documentation refinements:
- Exit reclamation uses confirmed cached ProcessWatch exit observations before/after enumeration. A failed process open alone does not prove the process exited.
- Saturation intentionally remains latched until rearming/restart, even if markers are later reclaimed. Do not automatically clear it without accounting for skipped unrecorded instances. Document this rare limitation.
- scripts/README.md:8 still describes HelpText alone, while the implementation correctly reads FullDescription as well.
- docs/TESTING.md:6 names branch experiments, while these reviewed uncommitted changes are in glass_mixer_audit. Identify the actual tested worktree/snapshot accurately.

## Fresh verification in this review

Passed on the current audit worktree:
- cargo fmt --check.
- cargo clippy --locked --all-targets -- -D warnings.
- cargo test --locked: 86 library passed, 3 ignored; 3 binary passed; 1 source-encoding passed. The 8 hardware audio tests remained ignored.
- cargo build --release --locked.
- PowerShell syntax parsing across scripts/*.ps1.
- git diff --check.
- scripts/verify-ui.ps1: OpenGL run passed its assertions and exited with code 0.
- scripts/verify-ui.ps1 -Renderer wgpu: forced fallback run passed its assertions and exited with code 0.
- Safe isolated error-lock probe: confirmed retained lock/blocked publication in the if-let body, with cleanup after leaving it.

The native UI check uses posted Windows Tab messages. It is useful integration evidence but does not establish physical keyboard layouts, Narrator speech, high contrast or the multi-DPI matrix.

Not run here: real desktop-input injection, live audio mutations, installer upgrade, actual glow-initialization failure/RDP transition, MSRV 1.95 or hosted CI for these uncommitted changes.

## What is ready to preserve

Migration protection, process-instance identities and conservative saturation, final-tree accessibility assertions, expanded device layout, per-binding shortcut status, strict native UI assertions, and the separation of synthetic pressure from real input are all good improvements.

After N1 is corrected and its coordinated regression passes, the remaining notes can be handled as small follow-up fixes or explicit deferrals. Obtain CI coverage of the final committed changes before completing a merge review. Optional future product features remain subject to later approval.

