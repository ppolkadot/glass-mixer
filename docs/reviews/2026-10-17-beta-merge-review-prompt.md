# Glass Mixer 0.2.0 beta: review and follow-up prompt

Review date: 2026-10-17. Repository: C:/Users/PC/glass-mixer.
Reviewed branch: experiments. Reviewed HEAD: 107a305f2da9524d478bfabd64452329a02deab9.
Primary comparison: 1016466..107a305, including commits 6bb5f79 and 107a305.
Current package version: 0.2.0-beta.1.

This document is a prompt for a future implementation/review session. The review that produced it changed no application source. Do not treat the suggestions at the end as authorization to implement new features.

## Assessment

The beta has a sound core and a substantially stronger test suite than the last reviewed baseline. No Critical/P0 or High/P1 defect was established in this review. It should remain a beta until the Important/P2 items below are corrected and their acceptance checks pass. Passing builds and unit tests alone do not establish merge readiness: the current suite misses several reproduced edge cases.

Recommended merge decision: **with fixes**, rather than merge the current tree unchanged. Resolve R1–R4 and R7, address the small R5/R6 correctness issues, and reconcile R8's user-facing claims. R9 defines the evidence needed for a subsequent merge review. The broader hardware matrix can remain explicitly deferred for beta work; it must not be represented as having passed.

Some findings are inherited or incompletely fixed, rather than new regressions. Each item identifies that distinction. The suspected PID reuse issue in the investigation backlog is not a proven runtime defect and is not included among confirmed merge blockers.

## Prompt to give the next implementer

You are continuing Glass Mixer, a native Windows Rust audio mixer. Study the current source before editing. Start by checking the working tree, branch and HEAD: line references below are for 107a305 and may drift. Read CHANGELOG.md, README.md, docs/ARCHITECTURE.md and the current verification sections of docs/TESTING.md and docs/PROJECT_CONTEXT.md. Compare them with the code.

Implement the approved correctness and accessibility corrections R1–R8 in small, reviewable commits on a feature branch based on the user's intended current branch. Preserve unrelated changes. Do not commit or merge into main, push, tag, publish, install over the user's app, or make a release without explicit authorization. Do not bump the version merely because the commit message says “Beta 2”: the manifest, installer and current changelog agree on 0.2.0-beta.1. A later review should explicitly choose the next beta or stable version.

Keep Windows authoritative for live audio state. Keep Core Audio objects on the engine's COM MTA thread; callbacks send messages. Preserve OWN_EVENT_CONTEXT, command coalescing, one reread after apply, fatal-error rebuild/backoff, and RAII cleanup. Do not add hidden polling, network access, telemetry, elevation, app routing, or per-app hiding. Pins remain metadata; presets persist levels only on explicit save. Do not relax automatic-rule safeguards to make tests pass.

First reproduce the findings against your actual starting HEAD. Choose the smallest correction that meets each acceptance criterion. Preserve the useful behavior listed below. Run targeted checks as changes are made, then the final checks in R9. Keep observed passes, observed failures, static findings, historical results and untested behavior distinct.

### R1 — Important/P2: make migration preservation a prerequisite for overwriting old settings

Evidence: src/settings.rs:284–292 and :309–315.

The new migration helper treats any existing backup path as success. If it must create a backup, write errors are only logged. load() still promotes the schema and returns usable settings; a later save() replaces the original file.

This contradicts the unconditional promise that an untouched old settings file is kept for downgrade. It also exposes a recovery problem: a failed direct write can leave an incomplete backup that later loads accept solely because the path exists.

Reproduced safely using SettingsStore in an isolated temporary directory under target:
1. Write a valid schema-1 settings.json.
2. Create a directory named settings.v1.backup.json.
3. Load, then save the returned settings.
4. The schema becomes 4 and the original is replaced, although the “backup” is a directory.

Probe output: migration_version=4, backup_is_directory=true, original_replaced=true.

Required correction:
- Make backup creation report success or failure to the caller.
- Create a complete backup through a temporary file and commit it safely. A failed or partial write must not be accepted as a successful preserved copy.
- Check that an existing backup is a usable regular file. Preserve the intentional policy of retaining the first successful backup; do not require its bytes to match every subsequent old-format source.
- Prevent replacement of the old source while required preservation is unresolved. The user must receive an actionable warning; a log-only warning is insufficient.
- Keep ordinary audio use available where possible. Avoid inventing a broad persistence framework for this narrow migration safeguard.

Acceptance:
- Existing valid migration tests still pass, including byte-for-byte preservation and retention of the first successful backup.
- A directory at the backup path cannot result in an overwritten original.
- Simulate a backup write/commit failure, including partial output; the source remains unchanged.
- After the obstruction is removed, retry can create a complete backup and migration can proceed.
- Test an invalid existing backup and current/newer schemas. Do not start making unnecessary backups for every ordinary save.
- Explain the final failure behavior in README/testing documentation.

### R2 — Important/P2: track individual process instances, not an application timestamp watermark

Evidence: src/audio/preferences.rs:185–217; src/audio/engine.rs:767–788.
Provenance: the underlying limitation existed in the baseline. The beta's sorting fix solves only same-refresh discovery order.

StartTracker stores one latest creation time per AppKey. Sorting current groups cannot handle audio sessions that arrive in different refreshes:
1. Arm at time 100.
2. Process A of an app starts at 150 but creates audio later.
3. Process B of the same app starts at 160 and creates audio immediately.
4. Observe B first: it qualifies and advances the watermark.
5. A first produces an untouched session later: it is rejected even though it has never been considered.

The isolated probe reproduced newer_process_first=true and older_process_first_audio_later=false. This is independent of the documented requirement for an initial 100% and unmuted state.

Required correction:
- Identify considered/intervened process instances using stable app identity plus PID and creation time, or an equivalent identity that handles PID reuse and distinct processes sharing a timestamp.
- Carry that identity through grouping, observation and intervention recording.
- A newer process must not suppress an older, previously unseen eligible process.
- Continue to reject processes predating arming, already-considered processes, nondefault initial states, and observed user changes.
- Keep markers long enough to prevent a new audio session in the same running process from causing reapplication. Bound retained state using lifecycle information/rearming without introducing polling.

Acceptance:
- Newer-first and older-first observations across separate passes both evaluate each distinct eligible process once.
- Same-refresh ordering, repeat observation, PID reuse, equal timestamps with different PIDs, rearming and interventions are covered.
- Intervening in one process cannot suppress an untouched sibling process.
- Explicit preset application and error handling remain unchanged.
- Update the launch-order claim in CHANGELOG only to the extent demonstrated by the final tests.

### R3 — Important/P2: keep keyboard focus visible in scrolling views

Evidence: src/ui/mixer.rs:391 and :481; src/ui/presets.rs:67; src/ui/settings_view.rs:49.
Provenance: already disclosed in CHANGELOG.md:92–93 and docs/TESTING.md, rather than a new regression.

Focus can move to an off-screen control while the scrolling viewport stays still. That makes keyboard operation unreliable and can allow an invisible slider to be adjusted. README.md:57 currently promises full keyboard navigation.

Required correction:
- Scroll a newly focused interactive control into view in populated Mixer, Devices, Presets and Settings.
- Use focus transitions/navigation events so persistent focus does not continually pull the viewport back when the user scrolls with a pointer.
- Preserve focus rings, disabled-control behavior, slider key operation, and sensible ordering through expanded sections.
- Do not add a timer or hidden repaint loop for this.

Acceptance:
- At 360×440 and 480×720, in both densities, Tab and Shift+Tab through enough content to require scrolling.
- The focused control remains visible; focus does not disappear behind the viewport edge.
- Exercise Space/Enter, slider arrows, Home/End and page steps.
- Test expanded device properties, many app rows, several presets and the shortcut editor.
- Include a meaningful headless keyboard-event regression where practical, plus an actual Windows traversal check. Programmatic request_focus alone does not establish native Tab behavior.
- Record Narrator speech as unverified unless it is actually exercised.

### R4 — Important/P2: make the headless accessibility harness describe the settled tree

Evidence: src/ui/test_support.rs:49–73.
Provenance: introduced by the beta's new harness.

The harness accumulates nodes across four passes under the assumption that updates are incremental. The installed egui 0.36.2 creates a fresh accessibility map each pass and exports a complete rooted tree. The harness never removes nodes that disappeared.

An isolated probe rendered a named button only in the first pass. The returned settled Frame still reported it: removed_button_still_reported=true. This can make expected-control assertions pass for a control that no longer exists, or make overflow/name checks fail on vanished controls.

Required correction:
- Base assertions on the final tree's reachable nodes. Replace the per-pass snapshot appropriately or use a correct tree consumer; avoid retaining orphaned or removed nodes.
- Resolve labelled_by relationships within that current tree.
- Keep text input values distinct from accessible names.
- Include interactive Link roles in name validation; the new Mixer navigation link is currently omitted by is_interactive().
- Preserve painter-only allocation checks while making clear that allocation extent and visible clipping are different assertions.

Acceptance:
- A first-pass-only button is absent from the final result.
- Disappearing unnamed/overflowing controls no longer affect final assertions.
- A control that remains present still exposes the current name, role, value/disabled state where checked, and bounds.
- An unnamed link is detected.
- Rerun all UI tests after correcting the harness. Investigate newly exposed failures; do not weaken bounds or name checks merely to regain a green suite.
- Update any test-coverage claims that the corrected harness no longer supports.

### R5 — Minor/P3: fix and cover the expanded unavailable-balance layout

Evidence: src/ui/mixer.rs:353–379. Coverage gaps: :753, :892–944.
Provenance: the underlying horizontal row predates the beta; relocating details and adding tests did not resolve this state.

The unsupported state places “Balance” and “Not available — this device is not stereo” in a nonwrapping horizontal layout. The card margin and expanded section indentation reduce its usable width.

An isolated headless reconstruction using the production card/theme and identical row measured:
- Comfortable density: 332 points available; content allocated to x=345.5.
- Compact density: 340 points available; content allocated to x=339.5.

This confirms an allocation overrun in the comfortable layout. It is not a newly captured screenshot of the full native Devices view. The actual scrolling container can clip the overflowing text.

Required correction:
- Reflow or wrap the unavailable explanation so it fits inside the card and expanded section.
- Keep the status textual and accessible, and do not show a fake numeric balance.
- Test the actual expanded device card. Existing Devices tests keep it collapsed; the standalone properties test omits card/section indentation and uses only stereo endpoints.

Acceptance:
- Mono/non-stereo output and input examples at minimum size, both densities.
- Several devices, expanded properties, realistic unavailable/error text and a visible scrollbar.
- No lost explanation or clipped actionable control.
- Stereo balance still operates and double-click still recenters it.

### R6 — Minor/P3: report shortcut state for the actual binding being displayed

Evidence: src/platform/hotkeys.rs:498–526, especially :518; src/settings.rs:213.
Provenance: new status presentation, triggered by leniently accepted duplicate-action entries.

A settings list may contain VolumeUp→Ctrl+F1 followed by VolumeUp→Ctrl+F2. Validation keeps the first and rejects the duplicate. The editor displays the first chord but searches failures only by action, so it can label that working first chord “Not active” using the second chord's error. Its initial draft also takes the last duplicate while the current label takes the first.

Required correction:
- Match displayed status to the binding's identity, including its chord, or normalize duplicate actions with a clear separate warning for the discarded entry.
- Align current label, editable draft, active registration and failed-entry explanation.
- Preserve per-binding registration: one invalid/conflicting shortcut must not disable unrelated valid ones.
- Keep Retry for recoverable Windows conflicts; do not imply retry repairs a structurally duplicate setting.

Acceptance:
- Duplicate same-action settings show the retained working chord accurately.
- Rejected duplicates remain understandable.
- Invalid, conflicting, pending, cleared and successful states remain accurate.
- Cover presentation without requiring a real global key registration if possible.

### R7 — Important/P2 for test safety: guarantee keyboard-injector cleanup on assertion failure

Evidence: src/platform/hotkeys.rs:786–809, especially :803.
Provenance: new ignored shutdown stress test; ordinary application use is unaffected.

The test starts an injector thread, then asserts that a hotkey event arrived. If that assertion panics, it never sets the stop flag or joins the injector. Dropping the JoinHandle detaches it, so it can keep injecting Ctrl+Alt+F21 into the desktop during the rest of the test process, after the service unregisters the chord.

Required correction:
- Own injector shutdown/join in a guard that runs during unwinding, or capture outcomes and perform unconditional cleanup before asserting.
- Preserve the test's purpose: pressure across service shutdown and a bounded completion assertion.
- Keep this desktop-input test ignored by default.

Acceptance:
- Exercise a deliberate failure/no-delivery path through a controllable seam.
- Confirm the injector terminates after failure and no detached injection loop remains.
- Confirm the normal stress test still checks both delivery and shutdown.
- Do not run real input stress tests until cleanup has been corrected and reviewed.

### R8 — Documentation and summary consistency

Evidence: README.md:23–26, :57, :146–147; CHANGELOG.md:45–48, :66–68, :87–99; src/settings.rs:146–147.

Correct these specific mismatches:
- README promises full keyboard navigation while off-screen focus remains a known limitation. Align it with R3's actual result and distinguish names/roles from verified Narrator speech.
- The automatic-rule restriction to initial 100% and unmuted is disclosed in CHANGELOG but not adequately in README or the rule-enabling UI. Explain it where users enable the rule, unless later approved behavior changes supersede it.
- CHANGELOG says unknown fields from newer versions are kept. The flatten map preserves unknown top-level settings fields; nested pin/preset/shortcut structures do not generally retain unknown fields. Narrow the statement rather than silently promising nested forward compatibility.
- README says glass turns off when Windows transparency effects “are on.” The implementation uses a solid surface when transparency effects are off, or high contrast is on. Correct the inversion.
- Keep historical 0.1.0 release results distinct from beta results. Current prerelease strings are consistent with each other, and numeric Windows assembly/resource version 0.2.0.0 is appropriate for that representation.
- Distinguish local checks from hosted CI, real hardware from fixtures, direct forced-wgpu startup from the real glow-error-to-wgpu transition, and a successful package compile from an installed upgrade.
- Preserve the specific sound-property limits. Readouts/Windows Settings shortcuts are functional; undocumented setters for format, spatial audio, enhancements and mono were intentionally not shipped.

Acceptance:
- Every fixed/verified claim traces to a current test, current inspection, or clearly dated historical evidence.
- Known limitations appear where they affect user expectations.
- No wording claims that the remaining OS/hardware/assistive-technology matrix passed.
- Update docs/PROJECT_CONTEXT.md without rewriting its historical record.

### R9 — Final verification and merge-review evidence

After the corrections:
- Run cargo fmt --check.
- Run cargo clippy --locked --all-targets -- -D warnings.
- Run cargo test --locked.
- Run cargo build --release --locked.
- Run relevant isolated lifecycle/UI checks on the corrected binary. Verify process termination by PID and exit code, not only a “shutdown complete” log.
- After R7, run ignored shortcut tests serially on a suitable desktop and restore their state.
- Recheck both renderers' lifecycle and window-toggle exit behavior if those paths change. Do not repeat the historical 70-run exercise unless a new failure warrants it.
- Validate beta packaging/version metadata locally; do not install over the user's 0.1.0 merely to obtain evidence.
- Obtain a successful Windows-hosted PR CI result before recommending merge. The workflow's presence and a local pass do not establish that result.
- Check the declared Rust 1.95 MSRV with the locked dependency set, or clearly qualify/update the support claim if it cannot compile. Current CI checks stable only; no current MSRV failure was demonstrated by this review.
- Report unsupported/unavailable hardware tests as not run. Do not introduce fake fixtures as evidence for microphone, hot-plug, multi-output, sleep/resume or audio-service behavior.

Separate merge acceptance from broader release validation:
- Require R1–R4/R7 correctness, the targeted regressions, coherent docs and green required CI for the next merge recommendation.
- Fix R5/R6 in the same work if possible; otherwise record explicit owner-approved deferrals.
- Keep Windows 10, microphone, multi-output, hot-plug, sleep/resume, service restart, 125–200% scaling, high contrast/light theme, Narrator, actual no-OpenGL/RDP transition, hardware Vulkan/DX12 and installer-upgrade evidence as an explicit matrix. A beta merge may defer some of these; supported-release claims need an intentional decision.

## Functionality and design worth preserving

These areas are good foundations for the next merge; avoid replacing them while fixing unrelated issues:

- The current tree compiles cleanly, lints with warnings denied, and passes its default hardware-independent suite. The new pure helpers for preset planning, rule decisions, name/conflict validation and geometry changes improve testability.
- Shortcut validation/registration is genuinely per binding. Good chords remain active when another is invalid or owned by another application, and a retry path exists.
- The hotkey worker emits window events instead of calling a UI-owned HWND synchronously. The UI ignores toggles after exit begins. This addresses the earlier cross-thread shutdown deadlock structurally.
- Hidden startup defers registration until the cloak-to-hidden handoff completes. RAII owns registrations, and hotkey shutdown precedes audio-engine shutdown.
- The fallback explicitly chooses DX12/Vulkan by default rather than retrying wgpu's GL backend. Renderer/backend/adapter logging makes diagnosis much clearer. An explicit diagnostic environment override remains.
- Sound properties are honest readouts and Windows Settings links. The app does not invent device support or silently write unreliable endpoint settings.
- Live audio still comes from Windows, commands retain their event context, and pins do not persist volume/mute. Manual preset behavior remains explicit.
- Settings migration's successful path preserves the original text, and tests verify first-copy retention. Per-entry leniency preserves valid siblings. R1 hardens its failure path.
- Device details now occupy the Devices scroll view rather than a cramped nested mixer viewport. Names/descriptions for selectors, filtering, presets and shortcut controls are improved; disabled Save explains why.
- CI has limited permissions, locked lint/test/release checks, concurrency cancellation and no publishing steps.
- The changelog documents important beta limits and downgrade behavior. Keep that candor while correcting the specific overstatements in R8.

## Investigation backlog: do not present these as established blockers

1. **PID reuse/session-cache timing** — src/audio/engine.rs:1023–1044 and :1092. Session opening can consult a PID-keyed cached ProcessWatch before exited entries are pruned. Static inspection suggests a new process reusing the PID could inherit the old exited watcher and lose its session until another structural refresh. This predates the reviewed commits. Reproduce with a synthetic process/session lifecycle seam before selecting a fix; if confirmed, evict/validate stale process identity before session insertion.
2. **Failed icon bookkeeping** — src/platform/icons.rs:135–177. Worker cache and requested entries are pruned, but the UI-side failed set is not retained against live identities at that point. Many distinct failed app identities can accumulate until pixel-size change/retry/reappearance. This is an inherited, low-impact growth concern, not evidence that GPU texture pruning is broken. Confirm with a bounded lifecycle test before changing it.
3. **Release coverage** — the current review did not inspect live GitHub check state. “No hosted result verified here” must not be rewritten as “CI has never run.” Inspect the actual PR when one exists.

## Evidence from this review

Fresh checks on 107a305, on 2026-10-17:
- Passed: formatting; clippy --locked --all-targets with warnings denied; cargo test --locked.
- Tests: 71 library passed, 3 desktop/global-shortcut tests ignored; 2 binary passed; 1 source-encoding passed; 8 hardware audio tests ignored.
- Passed: cargo build --release --locked and scripts/package.ps1. Portable zip and Inno installer compiled locally. The release executable reports 0.2.0-beta.1 and is 15,789,056 bytes. The installer reports product version 0.2.0-beta.1.
- Passed: git diff --check for the reviewed range.
- Observed failures: the safe isolated probes reproduced R1, R2 and R4; the identical-row headless layout probe measured the R5 comfortable-density overrun. These are product/helper edge cases, not failures of the default suite.
- Probe setup initially failed because direct rustc linking lacked the Windows import-library search path. After supplying the existing dependency path, the probes ran. This setup error was not a product build failure.
- Static review: independent hotkey, UI and audio/settings reviewers; local egui/eframe 0.36.2 dependency-source inspection.
- Not run here: GUI traversal/screenshots, Narrator, desktop key injection, live audio mutations, installer upgrade, no-OpenGL/RDP transition, hosted CI and the wider hardware matrix. Historical results in docs/TESTING.md remain historical.

The reproduction source and executable are scratch artifacts under target/review_probes.*. They use isolated settings data and headless egui; no real user settings, live audio levels, desktop input or installed app were changed.

Expected output from the future implementation session: a concise result per R1–R9 with files/commits, exact observed checks, remaining limitations, and a fresh merge recommendation. Optional future features must still be awaiting approval.

