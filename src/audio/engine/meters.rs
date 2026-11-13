//! Peak level meter polling and aggregation.

use super::Engine;
use super::runner::{METER_INTERVAL, Runner};
use super::session::Session;
use crate::audio::model::{AppKey, Meters};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;

pub(super) fn meters_differ(a: &Meters, b: &Meters) -> bool {
    const EPS: f32 = 0.01;
    let differ = |x: f32, y: f32| (x - y).abs() > EPS;
    differ(a.output, b.output)
        || differ(a.input, b.input)
        || a.apps.iter().any(|(k, &x)| differ(x, b.apps.get(k).copied().unwrap_or(0.0)))
        || b.apps.iter().any(|(k, &y)| !a.apps.contains_key(k) && differ(0.0, y))
}

/// Stores each app's loudest session peak in `apps`, reading every session
/// once. Existing entries are reused, so a steady set of apps doesn't allocate;
/// apps without a session are removed.
pub(super) fn merge_peaks<'a, S: 'a>(
    apps: &mut HashMap<AppKey, f32>,
    sessions: impl Iterator<Item = &'a S> + Clone,
    key_of: impl Fn(&S) -> &AppKey,
    mut peak_of: impl FnMut(&S) -> f32,
) {
    for v in apps.values_mut() {
        *v = 0.0;
    }
    for s in sessions.clone() {
        let peak = peak_of(s);
        match apps.get_mut(key_of(s)) {
            Some(v) => *v = v.max(peak),
            None => {
                apps.insert(key_of(s).clone(), peak);
            }
        }
    }
    apps.retain(|k, _| sessions.clone().any(|s| key_of(s) == k));
}

impl Runner {
    pub(super) fn poll_meters(&mut self) {
        if !self.shared.meters_wanted() {
            if *self.last_meters != Meters::default() {
                self.last_meters = Arc::default();
                self.shared.publish_meters(self.last_meters.clone());
            }
            return;
        }
        if self.last_meter_poll.elapsed() < METER_INTERVAL {
            return;
        }
        self.last_meter_poll = Instant::now();
        let Some(engine) = self.engine.as_ref() else { return };
        engine.read_meters(&mut self.meter_scratch);
        // Only a visible change allocates: the published copy that triggers a repaint.
        if meters_differ(&self.meter_scratch, &self.last_meters) {
            self.last_meters = Arc::new(self.meter_scratch.clone());
            self.shared.publish_meters(self.last_meters.clone());
        }
    }
}

impl Engine {
    /// The default input's peak level, for microphone ducking.
    pub(super) fn input_peak(&self) -> f32 {
        self.input.as_ref().and_then(|e| e.meter.as_ref()).and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0)
    }

    pub(super) fn read_meters(&self, out: &mut Meters) {
        let peak = |m: &Option<IAudioMeterInformation>| {
            m.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()).unwrap_or(0.0)
        };
        merge_peaks(&mut out.apps, self.all_sessions(), |s: &Session| &s.key, |s| peak(&s.meter));
        out.output = self.output.as_ref().map_or(0.0, |e| peak(&e.meter));
        out.input = self.input.as_ref().map_or(0.0, |e| peak(&e.meter));
    }
}
