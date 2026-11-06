# Windows validation matrix

What has been validated, how, and on what. It is kept honest: a scenario is
marked as tested only if it was actually run, and the way it was run is named.
Update it with every release, next to the results in [TESTING.md](TESTING.md).

| Status | Meaning |
| --- | --- |
| **Automated** | Runs in `cargo test` or CI on every push, without hardware. |
| **Simulated** | Exercised through unit tests or synthetic seams (a fake event, a fake device list), not through Windows. |
| **Physical** | Run by hand or by a script against real Windows audio and a real desktop. The date and machine are given. |
| **Not tested** | Never run. The note says what is needed. |

Physical results come from one machine: Windows 11 Pro build 26200, a 2010-era NVIDIA
GPU without Direct3D 12 or Vulkan (so OpenGL, and the wgpu fallback on the WARP software
adapter), onboard analog audio (generic High Definition Audio driver) and NVIDIA HDMI
audio, one monitor at 100 % scaling.

## Windows versions

| Version | Status | Notes |
| --- | --- | --- |
| Windows 11 build 26200 | **Physical** | Every gate: unit tests, real-device tests, lifecycle on both renderers, installer smoke test. |
| Other Windows 11 builds | **Not tested** | Needs a machine on, for example, 22H2 (Acrylic and Mica differ by build: README, "Requirements and compatibility"). |
| Windows 10 (the installer's minimum is build 17763) | **Not tested** | Needs a Windows 10 machine. The per-app device interface has a separate ID before Windows 10 21H2; that variant has never run. |
| Windows Server 2022 and the latest Server image (hosted CI) | **Automated** | Build, format, lint, unit tests. `ci.yml` runs both; Server images have no audio devices or interactive desktop, so nothing else runs there. |

## Audio hardware

| Hardware | Status | Notes |
| --- | --- | --- |
| Wired analog headphones and microphone (onboard) | **Physical** | Real-device tests and per-app routing on 2026-11-05. |
| HDMI or DisplayPort audio | **Physical** | The default output on 2026-11-06; every gate ran on it. |
| USB headphones | **Not tested** | Needs the device. |
| USB DAC | **Not tested** | Needs the device. |
| Bluetooth headphones | **Not tested** | Needs the device; also covers profile changes and reconnection. |
| Dedicated audio interface | **Not tested** | Needs the device. |
| Several microphones | **Not tested** | Only one microphone was available. |
| Virtual devices and virtual audio cables | **Not tested** | Needs the software installed. |
| Spatial audio drivers | **Not tested** | Spatial support is shown read-only; nothing was exercised with a spatial driver. |

## Audio scenarios

| Scenario | Status | Notes |
| --- | --- | --- |
| Several simultaneous sessions | **Physical**, **Simulated** | Real apps and silent helper processes; grouping and aggregation in unit tests. |
| Several sessions from one app | **Simulated** | Balance, volume, mute and per-stream rows in unit tests. The real apps seen had one session each. |
| App process recreation | **Physical**, **Simulated** | A helper's session appears and disappears (real-device test); 20,000 launches with PID reuse in a unit test. |
| Apps with helper processes | **Not tested** | No real multi-process app (a browser, a chat client) was playing. Per-app routing was checked with two processes of the test helper. |
| Device hot-plug | **Simulated** | Notification coalescing in unit tests. Physically seen only by accident: the headphones and microphone were unplugged overnight and Glass Mixer started on HDMI with no input, as it should. Never plugged or unplugged while it ran. |
| Default-device change | **Simulated** | Output cycling in unit tests. The real switching call was not exercised: the tests avoid changing the user's defaults. |
| Windows audio service restart | **Not tested** | The rebuild it triggers is covered by the Refresh test (**Physical**). |
| Bluetooth profile changes | **Not tested** | Needs Bluetooth headphones. |
| Microphone disconnection | **Physical** (accidental) | See hot-plug. |
| Sleep and resume | **Not tested** | A rebuild is scheduled 1.5 s after resume (unit-tested logic); no real sleep was done. |
| Rapid device reconnection | **Simulated** | A 50 ms coalescing window, unit-tested. |
| Long-running operation | **Not tested** beyond minutes | The longest run was a 40-second soak. |
| Lowered levels restored on exit and on rebuild | **Physical** | Real-device test; fails if the restore is removed. |

## Interface scenarios

| Scenario | Status | Notes |
| --- | --- | --- |
| Window close to tray, second instance, clean exit, start minimized | **Physical** | `scripts/lifecycle.ps1`, both renderers. |
| Several monitors, mixed DPI, taskbar edges, resolution changes | **Not tested** | One monitor at 100 %. Position restore across monitors is unit-tested with synthetic monitor tables (**Simulated**). |
| Typeface and new controls at 125, 150 and 200 % scaling | **Not tested** | Layout tests run at 100 %. |
| Explorer restart | **Not tested** | Relies on the tray library re-adding the icon on `TaskbarCreated` (its source was read, not run). Needs Explorer ended by hand. |
| Rapid show and hide, global shortcut delivery | **Physical** (earlier versions) | `scripts/hotkey-smoke.ps1`; not rerun for 0.5.0. |
| Global shortcut conflicts | **Simulated** | Synthetic registrations; the desktop tests that register real chords are `#[ignore]`d. |
| Keyboard traversal, control names, fit at minimum width | **Automated** | Accessibility-tree tests. Narrator and other screen readers: **Not tested**. |
| Foreground-window rules with real windows | **Not tested** | The engine path ran with reported process IDs; the hook itself starts and stops (unit test) but was not driven by switching real windows. |
| Microphone ducking with a live voice | **Not tested** | The gate is unit-tested; no app was capturing from the microphone. |

## Packaging

| Scenario | Status | Notes |
| --- | --- | --- |
| Portable ZIP contents, version, 64-bit, embedded font, byte-exact licenses | **Automated** (`scripts/verify-package.ps1`) | Also run by `package.ps1` and `release.yml`. |
| Launching the portable ZIP's executable | **Not tested** | A second copy would defer to the already-running real one. |
| Installer: fresh install, upgrade over a running copy, uninstall of a running copy | **Physical** (`scripts/installer-smoke.ps1`) | A test variant with its own identity and the debug build; see TESTING.md for results. The real release executable was not installed. |
| Upgrade from earlier published versions | **Not tested** | Only variant-to-variant. Settings from 0.4 are loaded by unit tests. |
| Code signing | **Not tested** | No certificate is available; unsigned builds are reported as unsigned. |

## Performance

Measured on the machine above; see the Stage 6 section of [TESTING.md](TESTING.md)
for numbers and method. Windows 10, other GPUs and long-running behavior have not been
measured.
