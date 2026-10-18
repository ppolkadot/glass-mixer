# Architecture

## Opt-in shortcuts, pins and presets

`platform/hotkeys.rs` owns a blocking Win32 message thread. It registers only
user-assigned chords and sends mute/relative-volume commands directly to the
audio handle. It never touches the window: the show/hide shortcut is queued as
a `HotkeyEvent` that the UI thread carries out in `App::logic`, so exit cannot
deadlock on a cross-thread window call. Each binding is validated and
registered on its own. An invalid, duplicate or Windows-rejected chord is
reported as a failed binding (saved but not active, with the reason) while
every other binding stays active; "Retry failed shortcuts" reconfigures and
retries only the ones that are not active. Drop unregisters all shortcuts and
waits up to 2 seconds for the thread before detaching it, before the audio
engine stops. The thread has no timer.

`audio/preferences.rs` normalizes stable app identities. Pins persist only
identity/name/icon hints; a missing app becomes a disabled UI placeholder with
no numeric value or meter. The `settings.json` schema is v4 and parses list
items individually so one bad pin, preset or hotkey does not reset preferences.

Presets are the explicit exception to never persisting volumes. Saving captures
current app volume/mute only. Applying changes currently running sessions with
the regular event context and one read after the command batch. The optional
automatic rule is conservative: it arms at configuration, tracks each running
process instance by `(AppKey, pid, created)`, considers only a newly launched
process instance with a new audio session, rejects observed external changes or
a nondefault initial state (anything other than `100%` and unmuted), and never
repeats for the same process instance. Tracking by `(AppKey, pid, created)`
avoids PID reuse and handles multi-instance launches or older processes that
open audio late independently; no polling or startup-wide restore occurs.

## Additional endpoints and sound properties

Opening Devices sends `LoadDevices`. The engine lazily opens non-default
endpoints and owns each volume callback until the device disappears or the
engine rebuilds. Device commands use endpoint IDs; fatal read errors rebuild
the engine. The app session list remains scoped to the default output.
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

 icon-resolver thread (COM STA): shell image extraction, cached per (app, px)
```

| Module | Responsibility |
| --- | --- |
| `audio/` | The engine thread, Core Audio callbacks, `IPolicyConfig`, process identity and exit watching, sleep/resume. `model.rs` holds the pure, unit-tested logic: snapshot types, session grouping, balance math, tooltip text. |
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
| Peak meters | `IAudioMeterInformation` (polled at about 30 Hz, only while the window is visible) |
| Default-device switching | Undocumented `IPolicyConfig` (`CPolicyConfigClient`) |
| App identity | `QueryFullProcessImageNameW`, `GetApplicationUserModelId`, version-resource `FileDescription` |
| Process exit | `RegisterWaitForSingleObject` on the process handle (no polling) |
| Icons | `IAudioSessionControl::GetIconPath` → AppsFolder shell item (packaged apps) → `IShellItemImageFactory` on the exe → generic tile |
| Sleep/resume | `PowerRegisterSuspendResumeNotification` |
| Glass and fallback | `DwmExtendFrameIntoClientArea` plus `DWMWA_SYSTEMBACKDROP_TYPE` (Acrylic = `TRANSIENTWINDOW`, Mica = `MAINWINDOW`), with a transparent GL surface; if GL startup fails, retry eframe/wgpu with an opaque surface on Direct3D 12 or Vulkan only (wgpu's OpenGL backend is excluded; see "Renderer fallback" below) |
| Tray | `tray-icon` (Shell_NotifyIcon) with a `muda` menu |
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

**Idle cost.** While hidden, the engine thread blocks on its channel and
the UI thread gets no frames, so CPU use is 0 ms over 10 s. While visible,
it re-checks Windows theme and transparency every 2 s, and polls meters
only while audio is playing.

**Starting hidden.** eframe always shows its window after the first frame.
To start in the tray without a flash, the window is DWM-cloaked at creation,
hidden once eframe shows it, then uncloaked.

**Window position.** New positions are saved in physical screen pixels and
restored with Win32 after window creation. This avoids converting a position
with the system DPI when monitors use different scales. Older settings saved
egui logical points; restoration tries each monitor's DPI and centers the
window if more than one monitor could match. The saved size is retained only
when the position can be validated on a monitor.

**Icon caches.** Both UI textures and worker results are pruned for apps that
disappear and pixel sizes replaced by a DPI change. Failed icon lookups can
be retried on Refresh or when an app returns.

**Glass verification.** `examples/glass_spike.rs` is the minimal test case
that confirmed DWM backdrops render behind a transparent eframe/glow window.
Run it with `GM_BACKDROP=acrylic|mica|none` and `GM_RENDERER=glow|wgpu`.
The fallback uses a solid surface to avoid presenting a broken backdrop on
machines where OpenGL cannot initialize.

**Renderer fallback.** `main.rs` runs eframe with glow first. If that fails,
it runs eframe again with wgpu, restricted to Direct3D 12 and Vulkan
(`WGPU_BACKEND` can still override this for diagnostics). wgpu's OpenGL
backend is deliberately excluded. It cannot help once OpenGL has failed, and
it caused an intermittent exit deadlock: its instance owns a "wgpu-hal WGL
Instance Thread" that loads the vendor OpenGL driver. When that thread exited
during teardown, the NVIDIA 341.92 driver's `DllMain` (under the loader lock)
waited for a driver thread that was blocked on a `dwmapi` lock, while the main
thread held that lock inside `DestroyWindow` and waited for the loader lock.
Thread stacks of a hung process showed all three. `MixerApp::new` logs a
`renderer:` line naming the renderer and, for wgpu, the backend and adapter.
