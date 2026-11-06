# Architecture

## Opt-in shortcuts, pins and profiles

`platform/hotkeys.rs` owns a blocking Win32 message thread. It registers only
user-assigned chords and sends audio commands (mute, relative volume, the next
pinned output, an app's mute, volume or Solo) directly to the audio handle. A
binding can carry a target: a pinned app (by `AppKey`) or a profile (by name).
Validation allows one chord per action and target. It never touches the
window: the show/hide and "activate profile" shortcuts are queued as
`HotkeyEvent`s that the UI thread carries out in `App::logic`, so exit cannot
deadlock on a cross-thread window call. Each binding is validated and
registered on its own. An invalid, duplicate or Windows-rejected chord is
reported as a failed binding (saved but not active, with the reason) while
every other binding stays active; "Retry failed shortcuts" reconfigures and
retries only the ones that are not active. Drop unregisters all shortcuts and
waits up to 2 seconds for the thread before detaching it, before the audio
engine stops. The thread has no timer. Tests exercise the worker's message
handling and shutdown through a synthetic registration seam, so posted-message
pressure is checked without owning a global chord; real `RegisterHotKey`
conflict handling and real `SendInput` delivery stay in ignored desktop tests.

`audio/preferences.rs` normalizes stable app identities. Pins persist only
identity/name/icon hints; a missing app becomes a disabled UI placeholder with
no numeric value or meter. The `settings.json` schema is v4 and parses list
items individually so one bad pin, preset or hotkey does not reset preferences.
Fields added since (pinned outputs, wheel step, Solo, focus rules, microphone
ducking, a profile's optional parts, a binding's target) are optional with
defaults, so v4 files from 0.4 load unchanged and are written back without
the new keys unless they are set.

Profiles (stored as `presets`) are the explicit exception to never persisting
volumes. Saving captures only the parts chosen: app volume/mute, each app's
output device, the master level, and the default output and input. Applying a
profile switches the default devices first, re-reads the sessions (Windows
keeps app levels per device), then sets the master level, the app levels and
the app devices, and reports what couldn't be applied (a disconnected device,
an app not running). The optional
automatic rule is conservative: it arms at configuration, tracks each running
process instance by `(AppKey, pid, created)`, considers only a newly launched
process instance with a new audio session, rejects observed external changes or
a nondefault initial state (anything other than `100%` and unmuted), and never
repeats for the same process instance. Tracking by `(AppKey, pid, created)`
avoids PID reuse and handles multi-instance launches or older processes that
open audio late independently; no polling or startup-wide restore occurs.
Live markers are never evicted. A marker is released when its process's exit
watch fires, and also by a reclaim pass that checks each marker with
`OpenProcess` plus the creation time (catching exits that were never observed:
no `SYNCHRONIZE` access, or sessions that ended before the process did, and PID
reuse). Anything Windows won't confirm as exited counts as running. The pass
runs during session refreshes, at most every 5 minutes, or every 10 seconds
once 512 markers are held; there is no timer. If 1,024 live instances are
tracked anyway, the tracker latches saturated and skips unseen instances. A
reclaim pass that frees capacity clears the latch and moves the arming time to
that moment, so no instance skipped while saturated (all started earlier) can
qualify later; only launches after recovery do.

## Solo, background lowering and microphone ducking

`audio/attenuation.rs` holds a ledger of temporary reductions per session: the
level before the first reduction and the level last written. Solo (every app
but one, except System sounds), focus rules (an app while another app is in
the foreground) and microphone ducking (chosen apps while the microphone is
above a threshold) each say which reduction an app should have; the strongest
applies. The engine reconciles in every snapshot pass, after the automatic
preset rules. If a session's level differs from what was written, the user or
another app changed it: the session is released and nothing is written to it
again until all its reductions end. Reductions are undone before session sets
are released (a removed device, an engine rebuild, exit). If Glass Mixer is
killed while something is lowered, Windows keeps that level for the app.

`platform/foreground.rs` runs only while a focus rule exists: an out-of-context
`EVENT_SYSTEM_FOREGROUND` hook on its own message thread reads the window's
process ID and sends it to the engine, which resolves the app. A change takes
effect after focus has stayed put for 250 ms (`automation::FocusSettle`), so
alt-tabbing doesn't bounce levels; Glass Mixer's own windows are ignored.
Microphone ducking reads the default input's peak at 20 Hz only while it is on;
a gate needs speech for the attack time to start and silence for the release
time to end. The capture meter reports a level only while some app is using
the microphone.

## Per-app devices

`audio/app_routing.rs` binds the undocumented WinRT factory
`Windows.Media.Internal.AudioPolicyConfig` that Windows Settings uses for "App
volume and device preferences". Its interface ID changed in Windows 10 21H2;
both are tried at runtime, with the same explicit method layout (verified
against EarTrumpet's binding, and by a test that checks the vtable slots). A
preference is set for every process of an app; Windows stores it for the app,
so later launches start on that device (verified on this machine). A null
device resets one app; `ClearAllPersistedApplicationDefaultEndpoints` resets
every app, so the UI offers it only as a separate, confirmed action. Running
streams move only if the app follows the default device; apps that opened a
specific device switch when they restart playback, and the row says so.

The engine opens a session set for every active output, the default's first,
so an app playing elsewhere stays visible and controllable. Sessions on other
outputs are listed only while playing or when their app has none on the
default output; System sounds is listed for the default output only.

## Diagnostic report

`diagnostics.rs` builds a plain-text report from the app's own state: versions,
the settings schema version, the renderer, the Windows build and appearance
flags, the audio service status, device and session counts, device read errors,
failed shortcuts, a settings summary (counts and choices), the last 30 warnings
and errors the user was shown, and the last 80 log lines. It never contains
audio, files or a name: app, device and profile names (from the snapshot, pins,
rules, profiles and shortcut targets) and the user's profile folder, in plain
text and in the escaped form the log uses, are replaced with `<app>`,
`<device>`, `<profile>` and `%USERPROFILE%` before the text is written or
copied. Names shorter than three characters, or inside a placeholder, are left
alone so the replacement can't garble text. "Save diagnostic report" writes it
to `diagnostics\` beside the log (the newest 10 are kept) and opens the folder; "Copy
report" puts it on the clipboard. Nothing is sent anywhere, and no dependency or
file dialog is involved.

## Additional endpoints and sound properties

Device added, removed, state and property notifications are coalesced: the
first one schedules a re-enumeration 50 ms later and the rest of the burst joins
it. Default-device changes refresh immediately, and also cover anything pending.
Opening Devices sends `LoadDevices`. The engine lazily opens non-default
endpoints and owns each volume callback until the device disappears or the
engine rebuilds. Device commands use endpoint IDs; fatal read errors rebuild
the engine.
Properties are read on structural/property notifications, never by a new timer.
`audio/properties.rs` exposes read-only formats and qualified availability;
unverified setters are replaced by Windows Settings shortcuts.

Tray notifications use the existing icon's GUID with `NIF_INFO`, `NIF_REALTIME`,
quiet-time and no-sound flags, plus `SHQueryUserNotificationState`. The GUID
depends on the executable path, respecting Windows' unsigned-icon binding.
Only hidden warnings/errors are eligible: at most one per 10 seconds, and
identical text at most once per 60 seconds. No WinRT AUMID or installer change.
See [Windows notification behavior](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area).

## Overview

```
┌────────────────────── UI thread (winit/eframe event loop) ──────────────────────┐
│ app::MixerApp ── logic(): tray actions, close→tray, prefs, settings save        │
│               └─ ui():    ui::mixer / ui::settings_view (egui, immediate mode)  │
│ platform::tray (tray-icon, !Send)   platform::backdrop (DWM)   fonts            │
└───────▲──────────────────────────── commands ──────────────┬────────────────────┘
        │ request_repaint() on new snapshot                   │ crossbeam channel
┌───────┴──────────── audio engine thread (COM MTA) ──────────▼────────────────────┐
│ audio::engine::Runner → Engine { IMMDeviceEnumerator, Endpoint×2, SessionSet }   │
│   re-reads Windows after every change → publishes Arc<AudioSnapshot> + Meters    │
└───────▲──────────────────────────────────────────────────────────────────────────┘
        │ Msg::* (send only)
 Core Audio callbacks (service threads) · process-exit waits (thread pool) · power notifications

 icon-resolver thread (COM STA): shell image extraction, LRU per (file identity, px)
```

| Module | Responsibility |
| --- | --- |
| `audio/` | The engine thread, Core Audio callbacks, `IPolicyConfig`, process identity and exit watching, sleep/resume. `model.rs` holds the pure, unit-tested logic: snapshot types, session grouping (an app's balance is `Uniform`, `Mixed` when its stereo sessions disagree, or `Unavailable`), balance math, tooltip text. |
| `platform/` | Win32 integration: tray, window show/hide/cloak, single instance, launch at login (registry), DWM backdrop, system preferences, fonts, and app icon resolution. |
| `ui/` | The design system (`theme.rs`), custom widgets (`widgets.rs`), the mixer view and the settings view. |
| `settings.rs` | Versioned JSON preferences with per-field lenient parsing, preservation of unknown top-level fields, atomic migration backups, and atomic writes. |
| `app.rs` | Lifecycle. It wires everything together and owns the single shutdown path. |
| `main.rs` | Logging, panic hook, single-instance check, window options. |

## Windows API strategy

| Concern | API |
| --- | --- |
| Devices, defaults, hot-plug | `IMMDeviceEnumerator`, `IMMNotificationClient` |
| Device volume, mute, balance | `IAudioEndpointVolume`, `IAudioEndpointVolumeCallback` |
| Sessions | `IAudioSessionManager2`, `IAudioSessionNotification`, `IAudioSessionControl2`, `IAudioSessionEvents` |
| Per-app volume, mute, balance | `ISimpleAudioVolume`, `IChannelAudioVolume` |
| Peak meters | `IAudioMeterInformation` (polled at about 30 Hz, only while the window is visible; never with a software renderer, where each repaint is rasterized by the CPU) |
| Default-device switching | Undocumented `IPolicyConfig` (`CPolicyConfigClient`) |
| Per-app devices | Undocumented `Windows.Media.Internal.AudioPolicyConfig` factory (`RoGetActivationFactory`) |
| Foreground app | `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)` |
| App identity | `QueryFullProcessImageNameW`, `GetApplicationUserModelId`, version-resource `FileDescription` |
| Process exit | `RegisterWaitForSingleObject` on the process handle (no polling) |
| Icons | `IAudioSessionControl::GetIconPath` → AppsFolder shell item (packaged apps) → `IShellItemImageFactory` on the exe → generic tile |
| Sleep/resume | `PowerRegisterSuspendResumeNotification` |
| Glass and fallback | `DwmExtendFrameIntoClientArea` plus `DWMWA_SYSTEMBACKDROP_TYPE` (Acrylic = `TRANSIENTWINDOW`, Mica = `MAINWINDOW`), with a transparent GL surface; if GL startup fails, retry eframe/wgpu with an opaque surface on Direct3D 12 or Vulkan only (wgpu's OpenGL backend is excluded; see "Renderer fallback" below) |
| Tray | `tray-icon` (Shell_NotifyIcon) with a `muda` menu; its default `libappindicator` (Linux GTK) feature is disabled, keeping the GTK/glib 0.18 family out of `Cargo.lock` |
| Launch at login | `HKCU\...\CurrentVersion\Run` (plus `StartupApproved` for the Task Manager state) |

## Key design decisions

**Windows is the source of truth.** Callbacks never carry values into the
UI. They only send a message such as `SessionValuesChanged`. The engine then
re-reads every value it shows and publishes a new immutable snapshot, but
only when something actually differs. As a result, the UI can't drift from
Windows, and a failed change can never look successful.

**Feedback-loop prevention.** Every change the mixer makes carries
`OWN_EVENT_CONTEXT`. Callbacks ignore notifications with that context; the
engine re-reads once after applying its own batch of commands instead. On the
UI side, a dragged slider shows its local value for 450 ms after the last
movement (`MixerState::hold`) and then returns to the Windows value. The
engine also collapses a burst of 60 slider events per second into the latest
value per control.

**Threading and COM.**
- All Core Audio objects live on one MTA thread and are never shared.
- Callbacks run on audio-service threads. They only hold a `Sender`, and
  they never call back into Core Audio, which can deadlock inside a
  notification.
- Shell icon extraction runs on its own STA thread.
- The tray icon isn't `Send`, so it stays on the UI thread.
  - eframe 0.36's `App::logic` runs even while the window is hidden, so
    tray actions and tooltips keep working with the window closed.
  - Showing the window from a tray click goes through Win32 directly, so
    it is instant and allowed to take focus.

**Lifetimes and shutdown.** Every registration has an owner whose `Drop`
unregisters it: `Engine`, `Endpoint`, `SessionSet`, `Session`,
`ProcessWatch` and `PowerWatch`. Exit runs one path:
1. Save settings.
2. Drop the tray icon.
3. Stop the engine, dropping every COM object before `CoUninitialize`.
4. Join the icon thread.

**Recovery.**
- `AUDCLNT_E_DEVICE_INVALIDATED`, `AUDCLNT_E_SERVICE_NOT_RUNNING` and
  RPC disconnects tear down and rebuild the whole engine.
- Rebuilds back off exponentially (250 ms → 30 s) and report "audio
  service unavailable" in the UI.
- Resume from sleep schedules a rebuild after 1.5 s.
- Transient errors retry only the structural refresh.
- A session that can't be read while enumerating is skipped and logged;
  the other sessions still refresh.

**Idle cost.** While hidden, the engine thread blocks on its channel and
the UI thread gets no frames, so CPU use is 0 ms over 10 s. While visible,
it re-checks Windows theme and transparency every 2 s and polls meters at
about 30 Hz. A poll reuses its buffers, so silence or a steady level costs a
few `GetPeakValue` calls and no allocation; only a change of more than 1 %
publishes new levels (shared as an `Arc`) and repaints.

**Toast lifetime follows visibility.** An in-window notice's countdown runs
only while the window is shown. Hiding the window freezes the remaining time,
and a warning queued while hidden starts (or resumes) counting down when the
window is next shown. This keeps a long hidden idle from expiring a warning
before it can be displayed, without a hidden timer, poll or repaint loop.

**Starting hidden.** eframe always shows its window after the first frame.
To start in the tray without a flash, the window is DWM-cloaked at creation,
hidden once eframe shows it, then uncloaked.

**Window position.** New positions are saved in physical screen pixels and
restored with Win32 after window creation. This avoids converting a position
with the system DPI when monitors use different scales. Older settings saved
egui logical points; restoration tries each monitor's DPI and centers the
window if more than one monitor could match. The saved size is retained only
when the position can be validated on a monitor.

**Typefaces.** `platform/fonts.rs` builds the egui font families from the
Typeface setting. Salted Flutters Polaris (Regular for text, Semibold for names,
labels and headings) is embedded with `include_bytes!` and `FontData::from_static`,
so it needs no installation, no file at run time and no extra memory copy. Each
chain is Polaris, then Segoe UI of the same weight when Windows has it, then
egui's built-in fonts, so a glyph Polaris lacks is drawn by the next font and
nothing disappears. The icon font (Segoe Fluent Icons or Segoe MDL2 Assets) is a
separate family. The Windows typeface leaves Polaris out entirely. Changing the
setting reinstalls the fonts once; there is no restart. The font's license
requires that the files stay unmodified, that its metadata line stays in every
style, and that `LICENSE.txt` accompanies redistribution: a test checks the first
two, and the portable ZIP and the installer both carry `fonts\LICENSE.txt`.

**Icon caches.** The worker keeps up to 96 extracted icons in an LRU keyed by
what they came from: the executable's lower-cased path, size and modification
time, the session's icon resource, the AUMID, and the pixel size. A relaunch
of the same binary, or another app running it, reuses the extraction; an
updated binary has a new size or time, so it is extracted again. Failures are
never cached. Requests go through a queue of 16; when it is full the request
is dropped without blocking the UI and made again on a later frame. The UI
keeps textures for live apps plus the 32 most recently seen apps that left,
and uploads a result only when its pixels differ from the texture it already
has. Textures for an old pixel size (after a DPI change) and failed lookups of
apps that left are dropped. Failed lookups can be retried on Refresh or when
an app returns.

**Glass verification.** `examples/glass_spike.rs` is the minimal test case
that confirmed DWM backdrops render behind a transparent eframe/glow window.
Run it with `GM_BACKDROP=acrylic|mica|none` and `GM_RENDERER=glow|wgpu`.
The fallback uses a solid surface to avoid presenting a broken backdrop on
machines where OpenGL cannot initialize.

**Renderer fallback.** `main.rs` runs eframe with glow first. If that fails,
it runs eframe again with wgpu, restricted to Direct3D 12 and Vulkan.
`WGPU_BACKEND` can narrow that choice for diagnostics. wgpu is compiled with
only those two backends; its OpenGL ES, WebGPU and Metal backends are not
built. wgpu's OpenGL backend is deliberately excluded. It cannot help once OpenGL has failed, and
it caused an intermittent exit deadlock: its instance owns a "wgpu-hal WGL
Instance Thread" that loads the vendor OpenGL driver. When that thread exited
during teardown, the NVIDIA 341.92 driver's `DllMain` (under the loader lock)
waited for a driver thread that was blocked on a `dwmapi` lock, while the main
thread held that lock inside `DestroyWindow` and waited for the loader lock.
Thread stacks of a hung process showed all three. `MixerApp::new` logs a
`renderer:` line naming the renderer and, for wgpu, the backend and adapter.
