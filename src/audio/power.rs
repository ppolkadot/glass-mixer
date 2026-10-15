//! Suspend/resume notifications. After resume, audio endpoints are often
//! re-initialized by their drivers, so the engine schedules a full rebuild.

use super::Msg;
use crossbeam_channel::Sender;
use std::ffi::c_void;
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY, PowerRegisterSuspendResumeNotification,
    PowerUnregisterSuspendResumeNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND};

pub struct PowerWatch {
    handle: *mut c_void,
    params: *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
    tx: *mut Sender<Msg>,
}

unsafe extern "system" fn on_power(context: *const c_void, kind: u32, _setting: *const c_void) -> u32 {
    if kind == PBT_APMRESUMEAUTOMATIC || kind == PBT_APMRESUMESUSPEND {
        let tx = unsafe { &*(context as *const Sender<Msg>) };
        let _ = tx.send(Msg::Resumed);
    }
    0
}

impl PowerWatch {
    pub fn register(tx: Sender<Msg>) -> Option<Self> {
        let tx = Box::into_raw(Box::new(tx));
        let params = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power),
            Context: tx as *mut c_void,
        }));
        let mut handle: *mut c_void = std::ptr::null_mut();
        let err = unsafe {
            PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK, HANDLE(params as *mut c_void), &mut handle)
        };
        if err != ERROR_SUCCESS {
            log::warn!("PowerRegisterSuspendResumeNotification failed: {err:?}");
            unsafe {
                drop(Box::from_raw(params));
                drop(Box::from_raw(tx));
            }
            return None;
        }
        Some(Self { handle, params, tx })
    }
}

impl Drop for PowerWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = PowerUnregisterSuspendResumeNotification(HPOWERNOTIFY(self.handle as _));
            drop(Box::from_raw(self.params));
            drop(Box::from_raw(self.tx));
        }
    }
}
