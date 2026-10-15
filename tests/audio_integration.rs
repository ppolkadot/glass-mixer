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
