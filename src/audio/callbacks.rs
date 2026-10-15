//! COM callback objects registered with Core Audio.
//!
//! Callbacks run on arbitrary MTA threads owned by the audio service. They do no
//! work besides forwarding a message to the engine thread, which then re-reads
//! the authoritative state from Windows. This keeps callbacks fast, avoids
//! re-entrancy into Core Audio from inside a notification (which can deadlock),
//! and means the objects only hold a `Sender`.

use super::{Flow, Msg};
use crossbeam_channel::Sender;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, AudioSessionDisconnectReason, AudioSessionState, AudioSessionStateExpired,
    DEVICE_STATE, EDataFlow, ERole, IAudioSessionControl, IAudioSessionEvents, IAudioSessionEvents_Impl,
    IAudioSessionNotification, IAudioSessionNotification_Impl, IMMNotificationClient, IMMNotificationClient_Impl,
    eCapture, eConsole, eRender,
};
use windows::core::{BOOL, GUID, PCWSTR, Ref, implement};

/// Event context GUID passed with every change made by Glass Mixer, so that the
/// resulting notifications can be recognised and not re-processed.
pub const OWN_EVENT_CONTEXT: GUID = GUID::from_u128(0x6f1c_2b7e_8d4a_4c59_9e3b_5a7d_21c0_f1a3);

fn is_own(ctx: *const GUID) -> bool {
    !ctx.is_null() && unsafe { *ctx } == OWN_EVENT_CONTEXT
}

#[implement(IMMNotificationClient)]
pub(crate) struct DeviceNotifier {
    tx: Sender<Msg>,
}

impl DeviceNotifier {
    pub(crate) fn create(tx: Sender<Msg>) -> IMMNotificationClient {
        Self { tx }.into()
    }
}

impl IMMNotificationClient_Impl for DeviceNotifier_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::DevicesChanged);
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::DevicesChanged);
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::DevicesChanged);
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> windows::core::Result<()> {
        // Windows fires this once per role; the console role is the one shown as "default".
        if role == eConsole {
            let flow = if flow == eRender {
                Some(Flow::Output)
            } else if flow == eCapture {
                Some(Flow::Input)
            } else {
                None
            };
            if let Some(flow) = flow {
                let _ = (flow, self.tx.send(Msg::DefaultDeviceChanged));
            }
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        // Only the friendly name matters to us; other property changes are frequent and irrelevant.
        if *key == windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName {
            let _ = self.tx.send(Msg::DevicesChanged);
        }
        Ok(())
    }
}

#[implement(IAudioEndpointVolumeCallback)]
pub(crate) struct EndpointVolumeNotifier {
    tx: Sender<Msg>,
}

impl EndpointVolumeNotifier {
    pub(crate) fn create(tx: Sender<Msg>) -> IAudioEndpointVolumeCallback {
        Self { tx }.into()
    }
}

impl IAudioEndpointVolumeCallback_Impl for EndpointVolumeNotifier_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        let own = !data.is_null() && unsafe { (*data).guidEventContext } == OWN_EVENT_CONTEXT;
        if !own {
            let _ = self.tx.send(Msg::EndpointVolumeChanged);
        }
        Ok(())
    }
}

#[implement(IAudioSessionNotification)]
pub(crate) struct SessionCreatedNotifier {
    tx: Sender<Msg>,
}

impl SessionCreatedNotifier {
    pub(crate) fn create(tx: Sender<Msg>) -> IAudioSessionNotification {
        Self { tx }.into()
    }
}

impl IAudioSessionNotification_Impl for SessionCreatedNotifier_Impl {
    fn OnSessionCreated(&self, _session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        // The engine re-enumerates; the interface pointer is not carried across threads.
        let _ = self.tx.send(Msg::SessionsChanged);
        Ok(())
    }
}

#[implement(IAudioSessionEvents)]
pub(crate) struct SessionEvents {
    tx: Sender<Msg>,
}

impl SessionEvents {
    pub(crate) fn create(tx: Sender<Msg>) -> IAudioSessionEvents {
        Self { tx }.into()
    }
}

impl IAudioSessionEvents_Impl for SessionEvents_Impl {
    fn OnDisplayNameChanged(&self, _name: &PCWSTR, ctx: *const GUID) -> windows::core::Result<()> {
        if !is_own(ctx) {
            let _ = self.tx.send(Msg::SessionsChanged);
        }
        Ok(())
    }

    fn OnIconPathChanged(&self, _path: &PCWSTR, ctx: *const GUID) -> windows::core::Result<()> {
        if !is_own(ctx) {
            let _ = self.tx.send(Msg::SessionsChanged);
        }
        Ok(())
    }

    fn OnSimpleVolumeChanged(&self, _volume: f32, _mute: BOOL, ctx: *const GUID) -> windows::core::Result<()> {
        if !is_own(ctx) {
            let _ = self.tx.send(Msg::SessionValuesChanged);
        }
        Ok(())
    }

    fn OnChannelVolumeChanged(
        &self,
        _count: u32,
        _volumes: *const f32,
        _changed: u32,
        ctx: *const GUID,
    ) -> windows::core::Result<()> {
        if !is_own(ctx) {
            let _ = self.tx.send(Msg::SessionValuesChanged);
        }
        Ok(())
    }

    fn OnGroupingParamChanged(&self, _param: *const GUID, _ctx: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnStateChanged(&self, state: AudioSessionState) -> windows::core::Result<()> {
        let msg = if state == AudioSessionStateExpired { Msg::SessionsChanged } else { Msg::SessionValuesChanged };
        let _ = self.tx.send(msg);
        Ok(())
    }

    fn OnSessionDisconnected(&self, _reason: AudioSessionDisconnectReason) -> windows::core::Result<()> {
        let _ = self.tx.send(Msg::SessionsChanged);
        Ok(())
    }
}
