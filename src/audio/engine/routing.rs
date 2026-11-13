//! Output-device cycling and per-application output routing.

use super::Engine;
use super::error::{ApplyError, Context};
use crate::audio::model::{AppEntry, AppKey, Flow, Route};
use crate::audio::preferences::{CycleOutcome, next_pinned_output};
use crate::audio::process;
use std::time::{Duration, Instant};

/// How long after a shortcut's device switch the resulting "Output switched"
/// notice is still treated as its answer (and shown in the tray).
const ANNOUNCE_WINDOW: Duration = Duration::from_secs(3);

impl Engine {
    /// Switches to the next connected pinned output. Returns an announcement
    /// when nothing was switched; a switch is announced once Windows reports it.
    pub(super) fn cycle_output(&mut self) -> Result<Option<String>, ApplyError> {
        let next = match next_pinned_output(&self.pinned_outputs, &self.outputs) {
            CycleOutcome::Switch(device) => device.clone(),
            CycleOutcome::AlreadyDefault(device) => {
                return Ok(Some(format!("{} is the only pinned output connected.", device.name)));
            }
            CycleOutcome::NothingPinned => {
                return Ok(Some("Pin output devices in Devices to cycle through them.".into()));
            }
            CycleOutcome::NoneConnected => return Ok(Some("None of the pinned output devices is connected.".into())),
        };
        let policy = self.policy.as_ref().ok_or_else(|| {
            ApplyError::Message("Switching the default device isn't supported on this system.".into())
        })?;
        policy.set_default(&next.id).context(|| format!("Couldn't switch to {}", next.name))?;
        self.announce_output_until = Some(Instant::now() + ANNOUNCE_WINDOW);
        Ok(None)
    }

    /// Sets (or resets) the output of every process of the app.
    pub(super) fn route_app(&mut self, key: &AppKey, device: Option<&str>) -> Result<(), ApplyError> {
        let routing = self.routing.as_ref().ok_or_else(|| {
            ApplyError::Message("Choosing a device per app isn't supported on this version of Windows.".into())
        })?;
        let processes = self.processes_of(key);
        if processes.is_empty() {
            return Err(ApplyError::Message(
                "That app has no audio session right now; start it playing, then choose its device.".into(),
            ));
        }
        let name = self.app_name(key).unwrap_or_else(|| "the app".into());
        let mut set = 0;
        let mut first_error = None;
        for (pid, created) in processes {
            match routing.set(pid, Flow::Output, device) {
                Ok(()) => {
                    set += 1;
                    let route = device.map_or(Route::SystemDefault, |id| Route::Device(id.to_owned()));
                    self.routes.insert((pid, created), route);
                }
                // A helper process may exit between listing and routing it.
                Err(e) if created.is_none_or(|t| process::is_running(pid, t)) => {
                    first_error.get_or_insert(e);
                }
                Err(_) => {}
            }
        }
        match first_error {
            Some(e) if set == 0 => Err(ApplyError::Windows(format!("Couldn't change the device of {name}"), e)),
            _ => Ok(()),
        }
    }

    /// Every process with a session of the app, with its creation time when known.
    pub(super) fn processes_of(&self, key: &AppKey) -> Vec<(u32, Option<u64>)> {
        let mut found: Vec<(u32, Option<u64>)> = Vec::new();
        for set in &self.sessions {
            for s in set.sessions.iter().filter(|s| &s.key == key && s.pid != 0) {
                let created = set.processes.get(&s.pid).and_then(|p| p.started_at);
                if !found.iter().any(|(pid, _)| *pid == s.pid) {
                    found.push((s.pid, created));
                }
            }
        }
        found
    }

    /// Fills each app's device preference, querying Windows only for processes
    /// not seen before. An app's first process that answers decides.
    pub(super) fn read_routes(&mut self, apps: &mut [AppEntry]) {
        let Some(routing) = &self.routing else {
            for app in apps.iter_mut() {
                app.route = Route::Unknown;
            }
            return;
        };
        let live: Vec<(u32, Option<u64>)> =
            apps.iter().filter(|a| a.key != AppKey::System).flat_map(|a| self.processes_of(&a.key)).collect();
        self.routes.retain(|process, _| live.contains(process));
        for app in apps.iter_mut() {
            if app.key == AppKey::System {
                continue;
            }
            let mut route = Route::Unknown;
            for process in self.processes_of(&app.key) {
                let known = if let Some(cached) = self.routes.get(&process) {
                    cached.clone()
                } else {
                    match routing.get(process.0, Flow::Output) {
                        Ok(Some(id)) => {
                            let r = Route::Device(id);
                            self.routes.insert(process, r.clone());
                            r
                        }
                        Ok(None) => {
                            self.routes.insert(process, Route::SystemDefault);
                            Route::SystemDefault
                        }
                        Err(e) => {
                            log::debug!("couldn't read the device preference of process {}: {e}", process.0);
                            Route::Unknown
                        }
                    }
                };
                if known != Route::Unknown {
                    route = known;
                    break;
                }
            }
            app.route = route;
        }
    }
}
