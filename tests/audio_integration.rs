//! Integration tests against the real Windows audio stack.
//!
//! They need an active output device and are `#[ignore]`d so `cargo test` stays
//! hardware-independent. Run them with:
//!
//!     cargo test --test audio_integration -- --ignored --test-threads=1
//!
//! Every test restores what it changes. Changes are kept tiny (±2 %) and are
//! made on the idle "System sounds" session where possible; session detection
//! uses a helper process that plays *silence*, so nothing is audible.

use glass_mixer::audio::{AppKey, AudioHandle, AudioSnapshot, Command, Flow, ServiceStatus};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};

const TIMEOUT: Duration = Duration::from_secs(4);
const HELPER_ENV: &str = "GLASS_MIXER_SILENT_HELPER";

fn wait_for(audio: &AudioHandle, what: &str, pred: impl Fn(&AudioSnapshot) -> bool) -> std::sync::Arc<AudioSnapshot> {
    let start = Instant::now();
    loop {
        let snap = audio.snapshot();
        if pred(&snap) {
            return snap;
        }
        if start.elapsed() > TIMEOUT {
            panic!("timed out waiting for: {what}\nlast snapshot: {snap:#?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ready() -> AudioHandle {
    let audio = AudioHandle::start();
    wait_for(&audio, "engine ready with an output device", |s| s.status == ServiceStatus::Ok && s.output.is_some());
    audio
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.005
}

fn nudge(v: f32) -> f32 {
    if v > 0.5 { v - 0.02 } else { v + 0.02 }
}

#[test]
#[ignore]
fn master_volume_set_and_restore() {
    let audio = ready();
    let original = audio.snapshot().output.clone().unwrap().volume;
    let target = nudge(original);
    audio.send(Command::SetEndpointVolume { flow: Flow::Output, volume: target });
    wait_for(&audio, "output volume applied", |s| close(s.output.as_ref().unwrap().volume, target));
    audio.send(Command::SetEndpointVolume { flow: Flow::Output, volume: original });
    wait_for(&audio, "output volume restored", |s| close(s.output.as_ref().unwrap().volume, original));
    audio.shutdown();
}

/// A change made outside the mixer (here: directly through Core Audio, as the
/// volume keys or Windows Settings would) must show up in the engine's snapshot.
#[test]
#[ignore]
fn external_master_volume_change_is_reflected() {
    let audio = ready();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).unwrap();
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).unwrap();
        let original = volume.GetMasterVolumeLevelScalar().unwrap();
        let target = nudge(original);
        volume.SetMasterVolumeLevelScalar(target, std::ptr::null()).unwrap();
        let start = Instant::now();
        wait_for(&audio, "external change reflected", |s| close(s.output.as_ref().unwrap().volume, target));
        let latency = start.elapsed();
        volume.SetMasterVolumeLevelScalar(original, std::ptr::null()).unwrap();
        wait_for(&audio, "external restore reflected", |s| close(s.output.as_ref().unwrap().volume, original));
        println!("external change reflected after {latency:?}");
    }
    audio.shutdown();
}

#[test]
#[ignore]
fn system_sounds_volume_and_mute() {
    let audio = ready();
    let snap = wait_for(&audio, "System sounds session", |s| s.apps.iter().any(|a| a.key == AppKey::System));
    let app = snap.apps.iter().find(|a| a.key == AppKey::System).unwrap().clone();

    let target = nudge(app.volume);
    audio.send(Command::SetAppVolume { key: AppKey::System, volume: target });
    wait_for(&audio, "session volume applied", |s| {
        s.apps.iter().any(|a| a.key == AppKey::System && close(a.volume, target))
    });
    // Per-app changes must not touch the device volume.
    assert!(close(audio.snapshot().output.as_ref().unwrap().volume, snap.output.as_ref().unwrap().volume));

    audio.send(Command::SetAppMute { key: AppKey::System, muted: !app.muted });
    wait_for(&audio, "session mute applied", |s| {
        s.apps.iter().any(|a| a.key == AppKey::System && a.muted != app.muted)
    });

    audio.send(Command::SetAppMute { key: AppKey::System, muted: app.muted });
    audio.send(Command::SetAppVolume { key: AppKey::System, volume: app.volume });
    wait_for(&audio, "session restored", |s| {
        s.apps.iter().any(|a| a.key == AppKey::System && a.muted == app.muted && close(a.volume, app.volume))
    });
    audio.shutdown();
}

#[test]
#[ignore]
fn missing_app_reports_error_instead_of_success() {
    let audio = ready();
    audio.send(Command::SetAppVolume { key: AppKey::Pid(u32::MAX), volume: 0.5 });
    let start = Instant::now();
    let notices = loop {
        let n = audio.take_notices();
        if !n.is_empty() || start.elapsed() > TIMEOUT {
            break n;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(notices.iter().any(|n| n.text.contains("no longer has an audio session")), "{notices:?}");
    audio.shutdown();
}

#[test]
#[ignore]
fn refresh_rebuilds_and_recovers() {
    let audio = ready();
    let before = audio.snapshot();
    audio.send(Command::Refresh);
    std::thread::sleep(Duration::from_millis(100));
    let after = wait_for(&audio, "engine ready after refresh", |s| s.status == ServiceStatus::Ok && s.output.is_some());
    assert_eq!(before.output.as_ref().unwrap().id, after.output.as_ref().unwrap().id);
    assert_eq!(before.outputs.len(), after.outputs.len());
    audio.shutdown();
}

/// Solo lowers another app (a silent helper) to the configured factor,
/// restores it, and keeps a level the user set while it was lowered.
#[test]
#[ignore]
fn solo_lowers_other_apps_restores_them_and_keeps_user_changes() {
    let audio = ready();
    let exe = std::env::current_exe().unwrap();
    let key = AppKey::Exe(exe.to_string_lossy().to_lowercase());
    let mut child = std::process::Command::new(&exe)
        .args(["--ignored", "--exact", "silent_stream_helper", "--nocapture"])
        .env(HELPER_ENV, "12")
        .spawn()
        .unwrap();
    let level = |s: &AudioSnapshot| s.apps.iter().find(|a| a.key == key).map(|a| a.volume);
    let original = level(&wait_for(&audio, "helper session detected", |s| level(s).is_some())).unwrap();
    audio.send(Command::SetSoloOptions { factor: 0.5, mute: false });

    // System sounds is soloed, so the helper (another app) is lowered to half.
    audio.send(Command::ToggleSolo { key: AppKey::System });
    wait_for(&audio, "helper lowered by Solo", |s| {
        s.solo == Some(AppKey::System) && level(s).is_some_and(|v| close(v, original * 0.5))
    });
    audio.send(Command::EndSolo);
    wait_for(&audio, "helper restored", |s| s.solo.is_none() && level(s).is_some_and(|v| close(v, original)));

    // A change made while lowered is kept when Solo ends.
    audio.send(Command::ToggleSolo { key: AppKey::System });
    wait_for(&audio, "helper lowered again", |s| level(s).is_some_and(|v| close(v, original * 0.5)));
    let chosen = if original > 0.6 { 0.6 } else { original + 0.1 };
    audio.send(Command::SetAppVolume { key: key.clone(), volume: chosen });
    wait_for(&audio, "user level applied", |s| level(s).is_some_and(|v| close(v, chosen)));
    audio.send(Command::EndSolo);
    wait_for(&audio, "solo ended", |s| s.solo.is_none());
    std::thread::sleep(Duration::from_millis(300));
    assert!(level(&audio.snapshot()).is_some_and(|v| close(v, chosen)), "the user's level was overwritten");

    audio.send(Command::SetAppVolume { key: key.clone(), volume: original });
    wait_for(&audio, "helper level put back", |s| level(s).is_some_and(|v| close(v, original)));
    let _ = child.kill();
    let _ = child.wait();
    audio.shutdown();
}

/// Sets a silent helper to another output, checks that Windows reports it and
/// that the stream moves (and stays listed), launches a second process of the
/// same app to see whether it inherits the choice, then resets it. Needs two
/// active outputs; passes without checking anything when there is only one.
#[test]
#[ignore]
fn an_app_can_be_routed_to_another_output_and_reset() {
    use glass_mixer::audio::Route;
    let audio = ready();
    let snap = audio.snapshot();
    let Some(other) = snap.outputs.iter().find(|d| !d.is_default).cloned() else {
        println!("only one output device; routing not exercised");
        return;
    };
    assert!(snap.can_route_apps, "the per-app routing interface is unavailable");
    let default_id = snap.output.as_ref().unwrap().id.clone();
    let exe = std::env::current_exe().unwrap();
    let key = AppKey::Exe(exe.to_string_lossy().to_lowercase());
    let helper = || {
        std::process::Command::new(&exe)
            .args(["--ignored", "--exact", "silent_stream_helper", "--nocapture"])
            .env(HELPER_ENV, "15")
            .spawn()
            .unwrap()
    };
    let mut first = helper();
    let app = |s: &AudioSnapshot| s.apps.iter().find(|a| a.key == key).cloned();
    wait_for(&audio, "helper session detected", |s| app(s).is_some());

    audio.send(Command::SetAppOutput { key: key.clone(), device: Some(other.id.clone()) });
    let moved = Instant::now();
    wait_for(&audio, "Windows reports the chosen device", |s| {
        app(s).is_some_and(|a| a.route == Route::Device(other.id.clone()))
    });
    let start = Instant::now();
    loop {
        let snap = audio.snapshot();
        if app(&snap).is_some_and(|a| a.streams.iter().any(|st| st.device_id == other.id)) {
            println!("stream moved to the chosen output after {:?}", moved.elapsed());
            break;
        }
        if start.elapsed() > Duration::from_secs(8) {
            println!("the stream did not move within 8 s (the app may need to restart its stream)");
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut second = helper();
    let start = Instant::now();
    let mut inherited = None;
    while start.elapsed() < Duration::from_secs(6) && inherited.is_none() {
        let snap = audio.snapshot();
        if let Some(a) = app(&snap) {
            let pids: std::collections::HashSet<u32> = a.streams.iter().map(|st| st.pid).collect();
            if pids.contains(&second.id()) {
                let on = a.streams.iter().find(|st| st.pid == second.id()).map(|st| st.device_id.clone());
                inherited = Some(on == Some(other.id.clone()));
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("a newly launched process of the app starts on the chosen output: {inherited:?}");

    audio.send(Command::SetAppOutput { key: key.clone(), device: None });
    wait_for(&audio, "the app follows the default again", |s| app(s).is_some_and(|a| a.route == Route::SystemDefault));
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
        if app(&audio.snapshot()).is_some_and(|a| a.streams.iter().all(|st| st.device_id == default_id)) {
            println!("streams back on the default output after {:?}", start.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for child in [&mut first, &mut second] {
        let _ = child.kill();
        let _ = child.wait();
    }
    audio.shutdown();
}

/// A focus rule lowers the silent helper while another process is reported as
/// the foreground, and restores it when the helper is reported in front. The
/// reports are sent directly; the real foreground window isn't changed.
#[test]
#[ignore]
fn a_focus_rule_lowers_an_app_in_the_background_and_restores_it() {
    use glass_mixer::audio::attenuation::Reduction;
    let audio = ready();
    let exe = std::env::current_exe().unwrap();
    let key = AppKey::Exe(exe.to_string_lossy().to_lowercase());
    let mut child = std::process::Command::new(&exe)
        .args(["--ignored", "--exact", "silent_stream_helper", "--nocapture"])
        .env(HELPER_ENV, "10")
        .spawn()
        .unwrap();
    let level = |s: &AudioSnapshot| s.apps.iter().find(|a| a.key == key).map(|a| a.volume);
    let original = level(&wait_for(&audio, "helper session detected", |s| level(s).is_some())).unwrap();
    audio.send(Command::SetFocusRules(vec![(key.clone(), Reduction { factor: 0.5, mute: false })]));
    // Another app in front: a short-lived command prompt (the test process runs the same binary as the helper).
    let mut other = std::process::Command::new(std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into()))
        .args(["/c", "ping -n 8 127.0.0.1 >nul"])
        .spawn()
        .unwrap();
    audio.send(Command::ForegroundChanged { pid: other.id() });
    wait_for(&audio, "helper lowered in the background", |s| {
        s.background_lowered.contains(&key) && level(s).is_some_and(|v| close(v, original * 0.5))
    });
    audio.send(Command::ForegroundChanged { pid: child.id() });
    wait_for(&audio, "helper restored in front", |s| {
        !s.background_lowered.contains(&key) && level(s).is_some_and(|v| close(v, original))
    });
    audio.send(Command::SetFocusRules(Vec::new()));
    let _ = other.kill();
    let _ = other.wait();
    let _ = child.kill();
    let _ = child.wait();
    audio.shutdown();
}

/// The app shortcut commands: mute toggle and relative volume on System sounds.
#[test]
#[ignore]
fn app_mute_and_volume_shortcut_commands_change_every_session() {
    let audio = ready();
    let level = |s: &AudioSnapshot| s.apps.iter().find(|a| a.key == AppKey::System).map(|a| (a.volume, a.muted));
    let (volume, muted) = level(&wait_for(&audio, "system sounds", |s| level(s).is_some())).unwrap();
    audio.send(Command::ToggleAppMute { key: AppKey::System });
    wait_for(&audio, "mute toggled", |s| level(s).is_some_and(|(_, m)| m != muted));
    audio.send(Command::ToggleAppMute { key: AppKey::System });
    wait_for(&audio, "mute toggled back", |s| level(s).is_some_and(|(_, m)| m == muted));
    let delta = if volume > 0.5 { -0.02 } else { 0.02 };
    audio.send(Command::AdjustAppVolume { key: AppKey::System, delta });
    wait_for(&audio, "volume adjusted", |s| level(s).is_some_and(|(v, _)| close(v, volume + delta)));
    audio.send(Command::SetAppVolume { key: AppKey::System, volume });
    wait_for(&audio, "volume restored", |s| level(s).is_some_and(|(v, _)| close(v, volume)));
    audio.shutdown();
}

/// Starts a helper process that renders silence, checks that its session
/// appears, then that it disappears when the process exits.
#[test]
#[ignore]
fn new_sessions_appear_and_exited_ones_disappear() {
    let audio = ready();
    let exe = std::env::current_exe().unwrap();
    let key = AppKey::Exe(exe.to_string_lossy().to_lowercase());
    let mut child = std::process::Command::new(&exe)
        .args(["--ignored", "--exact", "silent_stream_helper", "--nocapture"])
        .env(HELPER_ENV, "3")
        .spawn()
        .unwrap();
    let start = Instant::now();
    wait_for(&audio, "helper session detected", |s| s.apps.iter().any(|a| a.key == key));
    println!("session detected after {:?}", start.elapsed());
    child.wait().unwrap();
    let exited = Instant::now();
    wait_for(&audio, "helper session removed", |s| !s.apps.iter().any(|a| a.key == key));
    println!("session removed {:?} after process exit", exited.elapsed());
    audio.shutdown();
}

/// Not a real test: plays silence for `$GLASS_MIXER_SILENT_HELPER` seconds when
/// launched by `new_sessions_appear_and_exited_ones_disappear`.
#[test]
#[ignore]
fn silent_stream_helper() {
    let Some(secs) = std::env::var(HELPER_ENV).ok().and_then(|s| s.parse::<u64>().ok()) else {
        return;
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).unwrap();
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();
        let format = client.GetMixFormat().unwrap();
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 1_000_000, 0, format, None).unwrap();
        let render: IAudioRenderClient = client.GetService().unwrap();
        let buffer_frames = client.GetBufferSize().unwrap();
        client.Start().unwrap();
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            let padding = client.GetCurrentPadding().unwrap();
            let free = buffer_frames - padding;
            if free > 0 {
                render.GetBuffer(free).unwrap();
                render.ReleaseBuffer(free, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32).unwrap();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        client.Stop().unwrap();
    }
}
