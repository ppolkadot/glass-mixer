//! Real endpoint tests. RAII restores every changed channel, master and mute on panic too.
use glass_mixer::audio::{AudioHandle, Command, Flow, ServiceStatus};
use std::time::{Duration, Instant};
use windows::Win32::{
    Media::Audio::{Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator},
    System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
};

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct Restore {
    ep: IAudioEndpointVolume,
    master: f32,
    muted: bool,
    channels: Vec<f32>,
}
impl Restore {
    fn new(id: &str) -> Self {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let device = enumerator.GetDevice(&windows::core::HSTRING::from(id)).unwrap();
            let ep: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).unwrap();
            let master = ep.GetMasterVolumeLevelScalar().unwrap();
            let muted = ep.GetMute().unwrap().as_bool();
            let channels =
                (0..ep.GetChannelCount().unwrap()).map(|i| ep.GetChannelVolumeLevelScalar(i).unwrap()).collect();
            Self { ep, master, muted, channels }
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        unsafe {
            let results = [
                self.ep.SetMasterVolumeLevelScalar(self.master, std::ptr::null()),
                self.ep.SetMute(self.muted, std::ptr::null()),
            ];
            for result in results {
                if let Err(e) = result {
                    eprintln!("RESTORE FAILED: {e}");
                }
            }
            for (i, v) in self.channels.iter().enumerate() {
                if let Err(e) = self.ep.SetChannelVolumeLevelScalar(i as u32, *v, std::ptr::null()) {
                    eprintln!("RESTORE FAILED: {e}");
                }
            }
        }
    }
}
struct Audio(AudioHandle);
impl Drop for Audio {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}
fn wait(audio: &AudioHandle, condition: impl Fn(&glass_mixer::audio::AudioSnapshot) -> bool) {
    let start = Instant::now();
    while !condition(&audio.snapshot()) {
        assert!(start.elapsed() < Duration::from_secs(8), "snapshot timeout: {:?}", audio.snapshot());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires real Windows audio endpoints; restores all changed levels"]
fn independent_devices_balance_and_master_restore() {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
    let _com = Com;
    let audio = Audio(AudioHandle::start());
    wait(&audio.0, |s| s.status == ServiceStatus::Ok);
    audio.0.send(Command::LoadDevices);
    wait(&audio.0, |s| !s.device_states.is_empty() && s.device_states.len() == s.outputs.len() + s.inputs.len());
    let before = audio.0.snapshot();
    let states = before.device_states.clone();
    for d in states {
        let ep = d.state.expect("active device must be controllable on this test machine");
        let restore = Restore::new(&ep.id);
        let target = if ep.volume > 0.5 { ep.volume - 0.01 } else { ep.volume + 0.01 };
        audio.0.send(Command::SetDeviceVolume { id: ep.id.clone(), volume: target });
        wait(&audio.0, |s| {
            s.device_states
                .iter()
                .any(|d| d.info.id == ep.id && d.state.as_ref().is_ok_and(|e| (e.volume - target).abs() < 0.005))
        });
        if d.flow == Flow::Output && !d.info.is_default {
            assert_eq!(audio.0.snapshot().output, before.output, "non-default control must not affect default output");
        }
        if ep.balance.is_some() && target > 0.02 {
            audio.0.send(Command::SetDeviceBalance { id: ep.id.clone(), balance: 0.1 });
            wait(&audio.0, |s| {
                s.device_states.iter().any(|d| {
                    d.info.id == ep.id
                        && d.state.as_ref().is_ok_and(|e| e.balance.is_some_and(|b| (b - 0.1).abs() < 0.02))
                })
            });
            let later = if target > 0.5 { target - 0.01 } else { target + 0.01 };
            audio.0.send(Command::SetDeviceVolume { id: ep.id.clone(), volume: later });
            wait(&audio.0, |s| {
                s.device_states
                    .iter()
                    .any(|d| d.info.id == ep.id && d.state.as_ref().is_ok_and(|e| (e.volume - later).abs() < 0.005))
            });
            let actual = unsafe {
                glass_mixer::audio::balance_from_channels(
                    restore.ep.GetChannelVolumeLevelScalar(0).unwrap(),
                    restore.ep.GetChannelVolumeLevelScalar(1).unwrap(),
                )
            };
            println!("{} balance after master change: {actual}", ep.name);
            assert!((actual - 0.1).abs() < 0.02, "master volume should preserve channel ratio on tested hardware");
        }
        drop(restore);
        wait(&audio.0, |s| {
            s.device_states.iter().any(|d| {
                d.info.id == ep.id
                    && d.state.as_ref().is_ok_and(|e| {
                        (e.volume - ep.volume).abs() < 0.005
                            && e.balance.zip(ep.balance).is_none_or(|(a, b)| (a - b).abs() < 0.02)
                    })
            })
        });
    }
    audio.0.send(Command::Refresh);
    wait(&audio.0, |s| s.device_states.len() == before.device_states.len());
}
