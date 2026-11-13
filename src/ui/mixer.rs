//! The main mixer view: output device and master volume, per-app rows, and the
//! input (microphone) section.
//!
//! Values shown come from the latest Windows snapshot. While the user drags a
//! slider (and briefly afterwards) the locally chosen value is shown instead, so
//! the thumb doesn't jitter while the engine catches up; once the hold expires
//! the display returns to whatever Windows reports.

use super::Look;
use super::typography::{self, Role};
use super::widgets::{self, Slider, card, fallback_tile, glyph, icon_button, percent_label};
use crate::audio::{
    Activity, AppEntry, AppKey, AudioHandle, AudioSnapshot, BalanceState, Command, EndpointState, Flow, Meters, Route,
    ServiceStatus, matches_filter, show_with_idle_filter, to_percent,
};
use crate::platform::icons::{IconKey, IconResult, IconService};
use crate::settings::Settings;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Margin, Rect, RichText, Sense, Shape, TextureHandle,
    TextureOptions, Ui, Vec2, pos2, text::LayoutJob, vec2,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

const HOLD: Duration = Duration::from_millis(450);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Ctl {
    Device(String),
    App(AppKey),
    /// One session of an app, by instance identifier.
    Stream(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Target {
    Volume(Ctl),
    Balance(Ctl),
}

/// Icon textures of apps that left the list, kept so a relaunch shows its icon
/// at once without another upload.
const RECENT_ICONS: usize = 32;

/// An uploaded icon (or a failed lookup) and a hash of its pixels, so a result
/// identical to what is already on the GPU isn't uploaded again.
struct CachedIcon {
    texture: Option<TextureHandle>,
    pixels: u64,
    last_live: Instant,
}

fn pixel_hash(image: &egui::ColorImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    image.size.hash(&mut hasher);
    for pixel in &image.pixels {
        pixel.to_array().hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Default)]
pub struct MixerState {
    pub filter: String,
    expanded: HashSet<AppKey>,
    overrides: HashMap<Target, (f32, Instant)>,
    textures: HashMap<IconKey, CachedIcon>,
    names: HashMap<AppKey, String>,
    /// Device choices sent to Windows and not yet visible in a snapshot.
    routing: HashMap<AppKey, (Option<String>, Instant)>,
    /// Names the user gave apps: a copy of the setting, refreshed when it changes.
    custom_names: HashMap<AppKey, String>,
    custom_source: Vec<crate::audio::preferences::AppName>,
    rename: HashMap<AppKey, String>,
}

impl MixerState {
    /// Drops icons of other pixel sizes and failed lookups of apps that left;
    /// keeps the most recently live [`RECENT_ICONS`] icons of apps that left.
    fn prune_cache(&mut self, live: &HashSet<AppKey>, px: u32) {
        let now = Instant::now();
        self.textures.retain(|(key, size), icon| {
            if live.contains(key) {
                icon.last_live = now;
            }
            *size == px && (live.contains(key) || icon.texture.is_some())
        });
        let mut gone: Vec<_> = self
            .textures
            .iter()
            .filter(|((key, _), _)| !live.contains(key))
            .map(|(k, i)| (i.last_live, k.clone()))
            .collect();
        if gone.len() > RECENT_ICONS {
            gone.sort_by_key(|(last_live, _)| std::cmp::Reverse(*last_live));
            for (_, key) in gone.drain(RECENT_ICONS..) {
                self.textures.remove(&key);
            }
        }
        let textures = &self.textures;
        self.names.retain(|key, _| live.contains(key) || textures.contains_key(&(key.clone(), px)));
        self.rename.retain(|key, _| live.contains(key));
    }
    #[cfg(test)]
    fn cached_icon_keys(&self) -> HashSet<IconKey> {
        self.textures.keys().cloned().collect()
    }

    fn texture(&self, key: &IconKey) -> Option<&TextureHandle> {
        self.textures.get(key).and_then(|icon| icon.texture.as_ref())
    }
    fn value(&self, target: &Target, truth: f32) -> f32 {
        match self.overrides.get(target) {
            Some((v, until)) if Instant::now() < *until => *v,
            _ => truth,
        }
    }

    fn hold(&mut self, target: Target, value: f32) {
        self.overrides.insert(target, (value, Instant::now() + HOLD));
    }

    fn expire(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.overrides.retain(|_, (_, until)| *until > now);
        if let Some(next) = self.overrides.values().map(|(_, u)| *u).min() {
            ctx.request_repaint_after(next - now);
        }
    }

    /// Forgets the text typed into rename fields (settings were replaced).
    pub fn forget_drafts(&mut self) {
        self.rename.clear();
    }

    pub fn ingest_icons(&mut self, ctx: &egui::Context, results: Vec<(IconKey, IconResult)>) {
        for (key, result) in results {
            if let Some(name) = result.display_name {
                self.names.insert(key.0.clone(), name);
            }
            let pixels = result.image.as_ref().map_or(0, pixel_hash);
            if let Some(icon) = self.textures.get_mut(&key)
                && icon.texture.is_some()
                && icon.pixels == pixels
            {
                continue;
            }
            let texture =
                result.image.map(|img| ctx.load_texture(format!("app-icon-{:?}", key), img, TextureOptions::LINEAR));
            self.textures.insert(key, CachedIcon { texture, pixels, last_live: Instant::now() });
        }
    }

    /// What the row shows: the user's name, else the one Windows reports for a
    /// packaged app, else the session's name.
    fn display_name<'a>(&'a self, app: &'a AppEntry) -> &'a str {
        let custom = crate::audio::preferences::normalized_key(&app.key).and_then(|key| self.custom_names.get(&key));
        custom.map(String::as_str).unwrap_or_else(|| self.default_name(app))
    }

    /// The name without the user's change.
    fn default_name<'a>(&'a self, app: &'a AppEntry) -> &'a str {
        self.names.get(&app.key).map(String::as_str).unwrap_or(&app.name)
    }
}

pub enum Outcome {
    None,
    /// The user asked for the balance and sound-property controls, which live in Devices.
    OpenDevices,
}

pub struct MixerCtx<'a> {
    pub audio: &'a AudioHandle,
    pub snap: &'a AudioSnapshot,
    pub meters: &'a Meters,
    pub look: &'a Look,
    pub settings: &'a mut Settings,
    pub icons: &'a mut IconService,
}

pub fn show(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) -> Outcome {
    st.expire(ui.ctx());
    if st.custom_source != cx.settings.app_names {
        st.custom_source.clone_from(&cx.settings.app_names);
        st.custom_names = st.custom_source.iter().map(|n| (n.key.clone(), n.name.clone())).collect();
    }
    let live: HashSet<AppKey> = cx
        .snap
        .apps
        .iter()
        .map(|a| a.key.clone())
        .chain(cx.settings.pinned_apps.iter().map(|p| p.key.clone()))
        .collect();
    let icon_px = (cx.look.m.icon * ui.ctx().pixels_per_point()).ceil() as u32;
    st.prune_cache(&live, icon_px);
    cx.icons.prepare_pixel_size(icon_px);
    let gap = cx.look.m.gap as i8;
    let mut outcome = Outcome::None;

    egui::Panel::top("mixer-header")
        .frame(egui::Frame::NONE.inner_margin(Margin { bottom: gap, ..Margin::ZERO }))
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| {
            if header(ui, st, cx) {
                outcome = Outcome::OpenDevices;
            }
        });
    egui::Panel::bottom("mixer-footer")
        .frame(egui::Frame::NONE.inner_margin(Margin { top: gap, ..Margin::ZERO }))
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| input_section(ui, st, cx));
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| apps_section(ui, st, cx));
    outcome
}

/// Returns `true` when the Devices view (balance, sound properties) was requested.
fn header(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) -> bool {
    let look = cx.look;
    let mut open_devices = false;
    card(look).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let ServiceStatus::Unavailable(message) = &cx.snap.status {
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph::WARNING).font(widgets::icon_font(16.0)).color(look.p.danger));
                ui.label(typography::rich(message, Role::Callout, look).color(look.p.text));
            });
            ui.horizontal(|ui| {
                ui.label(
                    typography::rich("Glass Mixer keeps retrying automatically.", Role::Footnote, look)
                        .color(look.p.text_muted),
                );
                if widgets::button(ui, look, "Retry now", widgets::ButtonKind::Secondary).clicked() {
                    cx.audio.send(Command::Refresh);
                }
            });
            ui.add_space(look.m.gap * 0.5);
        }

        if let Some(solo) = &cx.snap.solo {
            let name = cx.snap.apps.iter().find(|a| &a.key == solo).map_or("an app", |a| st.display_name(a));
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph::SOLO).font(widgets::icon_font(14.0)).color(look.p.accent));
                // The button is placed first, so a long name truncates instead of pushing it out.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::scroll_on_focus(widgets::button(ui, look, "End solo", widgets::ButtonKind::Secondary))
                        .clicked()
                    {
                        cx.audio.send(Command::EndSolo);
                    }
                    let text = format!("Solo: {name}. Other apps are lowered.");
                    ui.add(
                        egui::Label::new(typography::rich(&text, Role::Callout, look).color(look.p.text)).truncate(),
                    )
                    .on_hover_text(text);
                });
            });
        }
        if cx.snap.mic_ducking {
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph::MIC).font(widgets::icon_font(14.0)).color(look.p.accent));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let off =
                        widgets::scroll_on_focus(widgets::button(ui, look, "Turn off", widgets::ButtonKind::Secondary));
                    widgets::set_name(ui, &off, "Turn off microphone ducking");
                    if off.clicked() {
                        cx.settings.mic_ducking.enabled = false;
                    }
                    let text = "Microphone ducking: apps are lowered while you talk.";
                    ui.add(egui::Label::new(typography::rich(text, Role::Callout, look).color(look.p.text)).truncate())
                        .on_hover_text(text);
                });
            });
        }
        ui.horizontal(|ui| {
            device_label(ui, look, glyph::SPEAKER, "Output");
            let width = ui.available_width();
            device_combo(ui, cx, Flow::Output, width);
        });
        match &cx.snap.output {
            Some(ep) => {
                endpoint_row(ui, st, cx, Flow::Output, ep, false);
                // Balance and sound properties need room; they live in the Devices view.
                if widgets::button(ui, look, "Balance and sound properties", widgets::ButtonKind::Plain).clicked() {
                    open_devices = true;
                }
            }
            None if cx.snap.status == ServiceStatus::Starting => {
                ui.label(
                    typography::rich("Connecting to Windows audio…", Role::Callout, look).color(look.p.text_muted),
                );
            }
            None => {
                ui.label(
                    typography::rich(
                        "No output device is available. Connect speakers or headphones.",
                        Role::Callout,
                        look,
                    )
                    .color(look.p.text_muted),
                );
            }
        }
    });
    open_devices
}

fn input_section(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) {
    let look = cx.look;
    card(look).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            device_label(ui, look, glyph::MIC, "Input");
            let width = ui.available_width();
            device_combo(ui, cx, Flow::Input, width);
        });
        match &cx.snap.input {
            Some(ep) => endpoint_row(ui, st, cx, Flow::Input, ep, false),
            None => {
                ui.label(RichText::new("No microphone or other input device was found.").color(look.p.text_muted));
            }
        }
    });
}

fn device_label(ui: &mut Ui, look: &Look, g: char, fallback: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(look.m.button), Sense::hover());
    if look.icons {
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, g, widgets::icon_font(16.0), look.p.text_muted);
    } else {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            &fallback[..1],
            FontId::proportional(12.0),
            look.p.text_muted,
        );
    }
}

fn device_combo(ui: &mut Ui, cx: &mut MixerCtx, flow: Flow, width: f32) {
    let devices = cx.snap.devices(flow);
    let current = cx.snap.endpoint(flow);
    let none = match flow {
        Flow::Output => "No output device",
        Flow::Input => "No input device",
    };
    let selected = current
        .and_then(|ep| devices.iter().find(|d| d.id == ep.id))
        .map(|d| crate::audio::device_label(d, devices))
        .unwrap_or_else(|| none.into());
    let can_switch = cx.snap.can_switch_default && !devices.is_empty();
    let kind = match flow {
        Flow::Output => "output",
        Flow::Input => "input",
    };
    let look = cx.look;
    ui.add_enabled_ui(can_switch, |ui| {
        let mut combo = egui::ComboBox::from_id_salt(("device", kind))
            .width(width.max(80.0))
            .truncate()
            .selected_text(typography::rich(&selected, Role::CalloutStrong, look));
        if look.icons {
            combo = combo.icon(widgets::combo_icon(look));
        }
        let response = combo
            .show_ui(ui, |ui| {
                for d in devices {
                    if ui
                        .selectable_label(d.is_default, crate::audio::device_label(d, devices))
                        .on_hover_text(&d.id)
                        .clicked()
                        && !d.is_default
                    {
                        cx.audio.send(Command::SetDefaultDevice { flow, id: d.id.clone() });
                    }
                }
            })
            .response;
        let hint = if can_switch {
            format!("Default {kind} device: {selected}. Choose another to switch.")
        } else if devices.is_empty() {
            format!("No {kind} devices are available")
        } else {
            "Switching the default device isn't available on this system".to_string()
        };
        widgets::set_name(ui, &response, &format!("{} device", if flow == Flow::Output { "Output" } else { "Input" }));
        response.on_hover_text(hint).on_disabled_hover_text(selected.to_string());
    });
}

/// Volume row for an endpoint. `details` adds the balance and sound-property section,
/// which only the Devices view (a full-height scroll area) has room for.
fn endpoint_row(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, flow: Flow, ep: &EndpointState, details: bool) {
    let look = cx.look;
    ui.horizontal(|ui| {
        let target = Target::Volume(Ctl::Device(ep.id.clone()));
        let mut v = st.value(&target, ep.volume);
        let (g, what) = match flow {
            Flow::Output => (glyph::volume(v, ep.muted), "output"),
            Flow::Input => (if ep.muted { glyph::MIC_OFF } else { glyph::MIC }, "microphone"),
        };
        let label = format!("{} {what}: {}", if ep.muted { "Unmute" } else { "Mute" }, ep.name);
        let emphasis = ep.muted.then_some(look.p.danger);
        if icon_button(ui, look, g, if ep.muted { "M" } else { "On" }, &label, emphasis).clicked() {
            cx.audio.send(Command::SetDeviceMute { id: ep.id.clone(), muted: !ep.muted });
        }
        let width = ui.available_width() - widgets::percent_width(ui, look) - ui.spacing().item_spacing.x;
        let meter =
            (cx.settings.show_meters && cx.snap.endpoint(flow).is_some_and(|d| d.id == ep.id)).then_some(match flow {
                Flow::Output => cx.meters.output,
                Flow::Input => cx.meters.input,
            });
        let a11y = format!("{} volume", ep.name);
        let response = Slider { muted: ep.muted, meter, ..Slider::volume(&mut v, &a11y, width) }
            .wheel_percent(cx.settings.wheel_step)
            .show(ui, &look.p);
        if response.changed() {
            st.hold(target.clone(), v);
            cx.audio.send(Command::SetDeviceVolume { id: ep.id.clone(), volume: v });
        } else if response.dragged() {
            st.hold(target, v);
        }
        let pct = if ep.muted { "Muted".to_string() } else { format!("{}%", to_percent(v)) };
        percent_label(ui, look, &pct, ep.muted);
    });
    if details {
        ui.push_id((&ep.id, "properties"), |ui| {
            let resp = egui::CollapsingHeader::new("Balance and sound properties").show(ui, |ui| {
                endpoint_balance(ui, st, cx, ep);
                sound_properties(ui, cx, ep);
            });
            widgets::scroll_on_focus(resp.header_response);
        });
    }
}

fn sound_properties(ui: &mut Ui, cx: &MixerCtx, ep: &EndpointState) {
    if let Some(props) = cx.snap.properties.get(&ep.id) {
        match &props.default_format {
            Ok(format) => {
                ui.label(format!("Default format: {format}"));
            }
            Err(reason) => {
                ui.label(format!("Default format unavailable: {reason}"));
            }
        }
        if let Ok(format) = &props.mix_format {
            ui.label(format!("Internal mix format: {format}"));
        }
        match &props.enhancements {
            Ok(enabled) => {
                ui.label(format!("Legacy system effects: {}", if *enabled { "Enabled" } else { "Disabled" }));
            }
            Err(_) => {
                ui.label("Audio enhancements: state unavailable from this device");
            }
        }
        ui.label("Vendor enhancements and the selected spatial provider are managed in Windows Settings.");
        match &props.spatial {
            Ok(support) => {
                ui.label(support.to_string());
            }
            Err(reason) => {
                ui.label(reason);
            }
        }
    } else {
        ui.label("Sound properties are not available yet.");
    }
    ui.label("Changing formats may briefly interrupt playing audio. Supported choices and reliable controls are available in Windows.");
    if widgets::scroll_on_focus(ui.button("Open Windows sound settings")).clicked() {
        cx.audio.send(Command::OpenSoundSettings);
    }
    ui.label("Mono audio: live state/control unavailable. Use Windows Accessibility audio settings.");
    if widgets::scroll_on_focus(ui.button("Open mono audio settings")).clicked() {
        cx.audio.send(Command::OpenMonoSettings);
    }
}

fn endpoint_balance(ui: &mut Ui, st: &mut MixerState, cx: &MixerCtx, ep: &EndpointState) {
    if let Some(balance) = ep.balance {
        ui.horizontal(|ui| {
            ui.label("Balance");
            ui.label("L");
            let target = Target::Balance(Ctl::Device(ep.id.clone()));
            let mut b = st.value(&target, balance);
            let response =
                Slider::balance(&mut b, &format!("{} balance", ep.name), (ui.available_width() - 60.0).min(220.0))
                    .show(ui, &cx.look.p);
            if response.double_clicked() {
                b = 0.0;
            }
            if response.changed() || response.double_clicked() {
                st.hold(target.clone(), b);
                cx.audio.send(Command::SetDeviceBalance { id: ep.id.clone(), balance: b });
            } else if response.dragged() {
                st.hold(target, b);
            }
            ui.label("R");
            response.on_hover_text(
                "Left/right balance. Double-click to center. Arrow keys adjust; Home/End choose a side.",
            );
        });
    } else {
        ui.add(egui::Label::new("Balance: Not available — this device is not stereo").wrap());
    }
}

pub fn show_devices(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) {
    st.expire(ui.ctx());
    ui.horizontal(|ui| {
        ui.heading("Devices");
        if widgets::scroll_on_focus(ui.button("Refresh")).clicked() {
            cx.audio.send(Command::LoadDevices);
        }
    });
    ui.label("Apps in the Mixer use the default output. Device controls below are independent.");
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for flow in [Flow::Output, Flow::Input] {
            ui.heading(if flow == Flow::Output { "Outputs" } else { "Inputs" });
            let devices = cx.snap.devices(flow);
            if devices.is_empty() {
                ui.label("No active devices");
            }
            for d in devices {
                ui.push_id(&d.id, |ui| {
                    card(cx.look).show(ui, |ui| {
                        ui.label(RichText::new(crate::audio::device_label(d, devices)).strong()).on_hover_text(&d.id);
                        if flow == Flow::Output {
                            let mut pinned = cx.settings.pinned_devices.iter().any(|p| p.id == d.id);
                            let pin = widgets::scroll_on_focus(ui.checkbox(&mut pinned, "Pin for output cycling"));
                            widgets::set_name(ui, &pin, &format!("Pin {} for output cycling", d.name));
                            if pin.on_hover_text("The \"Next pinned output device\" shortcut switches between pinned outputs, in the order they were pinned").changed() {
                                if pinned {
                                    cx.settings.pinned_devices.push(crate::audio::preferences::PinnedDevice {
                                        id: d.id.clone(),
                                        name: d.name.clone(),
                                    });
                                } else {
                                    cx.settings.pinned_devices.retain(|p| p.id != d.id);
                                }
                            }
                            if let Some(index) = cx.settings.pinned_devices.iter().position(|p| p.id == d.id) {
                                let len = cx.settings.pinned_devices.len();
                                let mut delta = 0;
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("Cycle position {}", index + 1)).small());
                                    let names = (format!("Cycle {} earlier", d.name), format!("Cycle {} later", d.name));
                                    delta = order_buttons(ui, index, len, ("Earlier", "Later"), (&names.0, &names.1), false);
                                });
                                crate::audio::preferences::move_item(&mut cx.settings.pinned_devices, index, delta);
                            }
                        }
                        if d.is_default {
                            ui.label("Default device");
                        } else if widgets::scroll_on_focus(
                            ui.add_enabled(cx.snap.can_switch_default, egui::Button::new("Make default")),
                        )
                        .clicked()
                        {
                            cx.audio.send(Command::SetDefaultDevice { flow, id: d.id.clone() });
                        }
                        match cx.snap.device_states.iter().find(|s| s.info.id == d.id) {
                            Some(s) => match &s.state {
                                Ok(ep) => endpoint_row(ui, st, cx, flow, ep, true),
                                Err(e) => {
                                    ui.label(e);
                                }
                            },
                            None => {
                                ui.label("Opening device…");
                            }
                        }
                    })
                });
                ui.add_space(cx.look.m.gap);
            }
            if flow == Flow::Output {
                disconnected_pins(ui, cx, devices);
            }
        }
    });
}

/// Pinned outputs that aren't connected stay in the cycle order; they can be unpinned here.
fn disconnected_pins(ui: &mut Ui, cx: &mut MixerCtx, connected: &[crate::audio::DeviceInfo]) {
    let missing: Vec<_> = cx
        .settings
        .pinned_devices
        .iter()
        .filter(|pin| !connected.iter().any(|d| d.id.eq_ignore_ascii_case(&pin.id)))
        .cloned()
        .collect();
    for pin in missing {
        ui.push_id(("pinned-offline", &pin.id), |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let unpin = widgets::scroll_on_focus(ui.small_button("Unpin"));
                widgets::set_name(ui, &unpin, &format!("Unpin {}", pin.name));
                if unpin.clicked() {
                    cx.settings.pinned_devices.retain(|p| p.id != pin.id);
                }
                if let Some(index) = cx.settings.pinned_devices.iter().position(|p| p.id == pin.id) {
                    let len = cx.settings.pinned_devices.len();
                    let names = (format!("Cycle {} earlier", pin.name), format!("Cycle {} later", pin.name));
                    let delta = order_buttons(ui, index, len, ("Earlier", "Later"), (&names.0, &names.1), true);
                    crate::audio::preferences::move_item(&mut cx.settings.pinned_devices, index, delta);
                }
                let text = format!("{} · pinned, not connected", pin.name);
                ui.add(egui::Label::new(RichText::new(&text).color(cx.look.p.text_muted)).truncate())
                    .on_hover_text(text);
            });
        });
    }
}

fn apps_section(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) {
    let look = cx.look;
    let visible: Vec<AppEntry> = crate::audio::preferences::rows_with_pins(&cx.snap.apps, &cx.settings.pinned_apps)
        .into_iter()
        .filter(|a| {
            cx.settings.pinned_apps.iter().any(|p| p.key == a.key)
                || show_with_idle_filter(&a.key, a.activity, cx.settings.show_inactive_apps)
        })
        .filter(|a| matches_filter(st.display_name(a), &st.filter))
        .collect();

    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(typography::rich("Apps", Role::Title, look).color(look.p.text));
        ui.label(typography::rich(format!("{}", visible.len()), Role::Footnote, look).color(look.p.text_muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, look, glyph::REFRESH, "R", "Refresh audio devices and apps", None).clicked() {
                cx.icons.retry_failed();
                cx.audio.send(Command::Refresh);
            }
            let search =
                widgets::search_field(ui, look, &mut st.filter, "Filter apps", ui.available_width().min(170.0));
            if ui.input(|i| i.modifiers.command && i.key_pressed(Key::F)) {
                search.request_focus();
            }
            if search.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                st.filter.clear();
            }
            widgets::set_name(ui, &search, "Filter apps");
            search.on_hover_text("Filter apps by name (Ctrl+F)");
        });
    });
    ui.add_space(4.0);

    if visible.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let text = if !st.filter.trim().is_empty() {
                format!("No apps match “{}”.", st.filter.trim())
            } else if cx.snap.status == ServiceStatus::Ok {
                "No apps have audio sessions right now.\nApps appear here when they start playing sound.".to_string()
            } else {
                String::new()
            };
            ui.label(RichText::new(text).color(look.p.text_muted));
        });
        return;
    }

    card(look).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, app) in visible.iter().enumerate() {
                if i > 0 {
                    let (sep_rect, _) =
                        ui.allocate_exact_size(egui::vec2(ui.available_width().max(1.0), 1.0), Sense::empty());
                    let left = sep_rect.left() + look.m.icon + 12.0;
                    let right = sep_rect.right() - 6.0;
                    if right > left {
                        ui.painter().line_segment(
                            [pos2(left, sep_rect.center().y), pos2(right, sep_rect.center().y)],
                            egui::Stroke::new(1.0, look.p.separator),
                        );
                    }
                }
                app_row(ui, st, cx, app);
            }
        });
    });
}

fn placeholder_row(ui: &mut Ui, st: &MixerState, cx: &mut MixerCtx, app: &AppEntry) {
    let look = cx.look;
    let name = st.display_name(app).to_owned();
    ui.push_id((&app.key, "offline"), |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
            // Fill the row, so long names truncate instead of widening the card.
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let px = (look.m.icon * ui.ctx().pixels_per_point()).ceil() as u32;
                cx.icons.request(app, px);
                ui.scope(|ui| {
                    ui.multiply_opacity(0.45);
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(look.m.icon), Sense::hover());
                    if let Some(tex) = st.texture(&(app.key.clone(), px)) {
                        ui.painter().image(
                            tex.id(),
                            rect,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::from_gray(128),
                        );
                    } else {
                        fallback_tile(ui, rect, &name, look, None);
                    }
                });
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(&name).strong().color(look.p.text_muted)).truncate());
                    ui.label("Not running");
                });
            });
            ui.add_enabled_ui(false, |ui| {
                ui.horizontal(|ui| {
                    icon_button(ui, look, glyph::MUTE, "M", &format!("{} mute unavailable", name), None);
                    widgets::unavailable_slider(ui, look, &format!("{} volume unavailable — not running", name));
                });
            });
            ui.horizontal_wrapped(|ui| {
                let unpin = widgets::scroll_on_focus(ui.button("Unpin"));
                // The visible text is short so the row fits; the accessible name says which app.
                widgets::set_name(ui, &unpin, &format!("Unpin {}", name));
                if unpin.on_hover_text(format!("Unpin {}", name)).clicked() {
                    cx.settings.pinned_apps.retain(|p| p.key != app.key);
                }
                ui.label(RichText::new("No active audio session").small().color(look.p.text_muted));
            });
            ui.add(
                egui::Label::new(
                    RichText::new("Moved or uninstalled? Unpin this row, then pin the app again when it plays.")
                        .small(),
                )
                .wrap(),
            );
        });
    });
}

fn app_row(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, app: &AppEntry) {
    if app.session_count == 0 {
        placeholder_row(ui, st, cx, app);
        return;
    }
    let look = cx.look;
    let p = &look.p;
    let m = &look.m;
    let name = st.display_name(app).to_string();
    let expanded = st.expanded.contains(&app.key);
    let bg = ui.painter().add(Shape::Noop);

    let block = ui.vertical(|ui| {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.set_min_height(m.row_height - 6.0);
            ui.add_space(4.0);
            let px = (m.icon * ui.ctx().pixels_per_point()).ceil() as u32;
            cx.icons.request(app, px);
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(m.icon), Sense::hover());
            match st.texture(&(app.key.clone(), px)) {
                Some(tex) => {
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    let tint = if app.muted { Color32::from_white_alpha(150) } else { Color32::WHITE };
                    ui.painter().image(tex.id(), icon_rect, uv, tint);
                }
                _ => {
                    let g = (app.key == AppKey::System).then_some(glyph::SPEAKER);
                    fallback_tile(ui, icon_rect, &name, look, g);
                }
            }

            let pinned = cx.settings.pinned_apps.iter().any(|pin| pin.key == app.key);
            let soloed = cx.snap.solo.as_ref() == Some(&app.key);
            // The Solo button takes its room from the name, so the slider keeps its width.
            let (name_min, solo_room) =
                if pinned || soloed { (52.0, m.button + ui.spacing().item_spacing.x) } else { (84.0, 0.0) };
            let name_w = (ui.available_width() * 0.34 - solo_room).clamp(name_min, 210.0);
            let (nrect, nresp) = ui.allocate_exact_size(vec2(name_w, m.row_height - 10.0), Sense::click());
            let mut job = LayoutJob::single_section(name.clone(), typography::format(Role::Headline, p.text, look));
            let chevron_w = if look.icons { 14.0 } else { 4.0 };
            job.wrap = egui::text::TextWrapping::truncate_at_width((name_w - chevron_w).max(20.0));
            let galley = ui.painter().layout_job(job);
            let (status, status_color) = if app.muted {
                ("Muted", p.danger)
            } else if app.activity == Activity::Playing {
                ("Playing", p.playing)
            } else {
                ("Idle", p.text_muted)
            };
            let mut extras = Vec::new();
            if app.session_count > 1 {
                extras.push(format!("{} streams", app.session_count));
            }
            match &cx.snap.solo {
                Some(solo) if *solo == app.key => extras.push("Solo".to_owned()),
                Some(_) if app.key != AppKey::System => extras.push("lowered by Solo".to_owned()),
                _ => {}
            }
            if cx.snap.background_lowered.contains(&app.key) {
                extras.push("lowered in background".to_owned());
            }
            if cx.snap.mic_ducking && cx.settings.mic_ducking.targets.contains(&app.key) {
                extras.push("lowered while you talk".to_owned());
            }
            let show_status = status != "Idle" || !extras.is_empty();
            let compact = m.row_height < 46.0 || !show_status;
            let name_y = if compact { nrect.center().y - galley.size().y / 2.0 } else { nrect.top() + 3.0 };
            ui.painter().galley(pos2(nrect.left() + 2.0, name_y), galley, p.text);
            if !compact {
                let sy = nrect.top() + 24.0;
                let mut x = nrect.left() + 2.0;
                if app.activity == Activity::Playing && !app.muted {
                    ui.painter().circle_filled(pos2(x + 3.0, sy + 7.5), 3.0, p.playing);
                    x += 10.0;
                }
                let text = if status == "Idle" {
                    extras.join(" · ")
                } else if extras.is_empty() {
                    status.to_owned()
                } else {
                    format!("{status} · {}", extras.join(" · "))
                };
                let mut sjob = LayoutJob::single_section(text, typography::format(Role::Footnote, status_color, look));
                sjob.wrap = egui::text::TextWrapping::truncate_at_width((nrect.right() - chevron_w - x).max(20.0));
                let sgalley = ui.painter().layout_job(sjob);
                ui.painter().galley(pos2(x, sy), sgalley, status_color);
            }
            if look.icons {
                let g = if expanded { glyph::CHEVRON_DOWN } else { glyph::CHEVRON_RIGHT };
                ui.painter().text(
                    pos2(nrect.right() - 6.0, nrect.center().y),
                    Align2::CENTER_CENTER,
                    g,
                    widgets::icon_font(10.0),
                    p.text_muted,
                );
            }
            let details = if expanded { "Hide details" } else { "Show details" };
            widgets::focus_ring(ui, nrect, &nresp, p, 6);
            let a11y = format!("{name}, {status}. {details}");
            nresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &a11y));
            let nresp = nresp.on_hover_text(format!("{name} — {status}. Click for details."));
            if nresp.clicked() {
                if expanded {
                    st.expanded.remove(&app.key);
                } else {
                    st.expanded.insert(app.key.clone());
                }
            }

            let target = Target::Volume(Ctl::App(app.key.clone()));
            let mut v = st.value(&target, app.volume);
            if pinned || soloed {
                let label = if soloed { format!("End solo for {name}") } else { format!("Solo {name}") };
                if icon_button(ui, look, glyph::SOLO, "S", &label, soloed.then_some(p.accent)).clicked() {
                    cx.audio.send(Command::ToggleSolo { key: app.key.clone() });
                }
            }
            let mute_label = format!("{} {name}", if app.muted { "Unmute" } else { "Mute" });
            let emphasis = app.muted.then_some(p.danger);
            if icon_button(
                ui,
                look,
                glyph::volume(v, app.muted),
                if app.muted { "M" } else { "On" },
                &mute_label,
                emphasis,
            )
            .clicked()
            {
                cx.audio.send(Command::SetAppMute { key: app.key.clone(), muted: !app.muted });
            }
            let width = ui.available_width() - widgets::percent_width(ui, look) - ui.spacing().item_spacing.x - 4.0;
            let meter = cx.settings.show_meters.then(|| cx.meters.apps.get(&app.key).copied().unwrap_or(0.0));
            let a11y = format!("{name} volume");
            let response = Slider { muted: app.muted, meter, ..Slider::volume(&mut v, &a11y, width) }
                .wheel_percent(cx.settings.wheel_step)
                .show(ui, p);
            if response.changed() {
                st.hold(target.clone(), v);
                cx.audio.send(Command::SetAppVolume { key: app.key.clone(), volume: v });
            } else if response.dragged() {
                st.hold(target, v);
            }
            percent_label(ui, look, &format!("{}%", to_percent(v)), app.muted);
        });

        if expanded {
            details(ui, st, cx, app, &name);
        }
        ui.add_space(3.0);
    });

    let rect = block.response.rect;
    if ui.rect_contains_pointer(rect) || expanded {
        ui.painter().set(
            bg,
            Shape::rect_filled(
                rect.shrink(2.0),
                CornerRadius::same(m.control_radius),
                p.surface_hover.gamma_multiply(if expanded { 0.8 } else { 1.0 }),
            ),
        );
    }
}

fn details(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, app: &AppEntry, name: &str) {
    let look = cx.look;
    let p = &look.p;
    let indent = look.m.icon + 12.0;
    if let Some(pin) = crate::audio::preferences::PinnedApp::from_app(app) {
        let mut pinned = cx.settings.pinned_apps.iter().any(|p| p.key == pin.key);
        if widgets::scroll_on_focus(ui.checkbox(&mut pinned, format!("Pin {name}"))).changed() {
            if pinned {
                cx.settings.pinned_apps.push(pin);
            } else {
                cx.settings.pinned_apps.retain(|p| p.key != pin.key);
            }
        }
    }
    if let Some(key) = crate::audio::preferences::normalized_key(&app.key) {
        rename_row(ui, st, cx, app, &key, indent);
        if let Some(index) = cx.settings.pinned_apps.iter().position(|pin| pin.key == key) {
            let len = cx.settings.pinned_apps.len();
            let mut delta = 0;
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.label(RichText::new("Order").color(p.text_muted));
                let names = (format!("Move {name} up"), format!("Move {name} down"));
                delta = order_buttons(ui, index, len, ("Move up", "Move down"), (&names.0, &names.1), false);
            });
            crate::audio::preferences::move_item(&mut cx.settings.pinned_apps, index, delta);
        }
    }
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.label(RichText::new("Balance").color(p.text_muted));
        match app.balance {
            BalanceState::Uniform(balance) => {
                ui.label(RichText::new("L").small().color(p.text_muted));
                let target = Target::Balance(Ctl::App(app.key.clone()));
                let mut b = st.value(&target, balance);
                let width = (ui.available_width() - 70.0).min(220.0);
                let a11y = format!("{name} balance");
                let response = Slider { muted: app.muted, ..Slider::balance(&mut b, &a11y, width) }.show(ui, p);
                if response.changed() {
                    st.hold(target.clone(), b);
                    cx.audio.send(Command::SetAppBalance { key: app.key.clone(), balance: b });
                } else if response.dragged() {
                    st.hold(target, b);
                }
                ui.label(RichText::new("R").small().color(p.text_muted));
                if response.double_clicked() {
                    cx.audio.send(Command::SetAppBalance { key: app.key.clone(), balance: 0.0 });
                }
            }
            BalanceState::Mixed => {
                ui.label(RichText::new("Differs between streams").small().color(p.text_muted)).on_hover_text(
                    "This app's audio streams use different balances. Centering sets every stream to the same balance.",
                );
                if widgets::scroll_on_focus(ui.small_button("Center all")).clicked() {
                    cx.audio.send(Command::SetAppBalance { key: app.key.clone(), balance: 0.0 });
                }
            }
            BalanceState::Unavailable => {
                ui.label(RichText::new("Not available for this app").small().color(p.text_muted));
            }
        }
    });
    if app.key != AppKey::System {
        output_choice(ui, st, cx, app, name, indent);
    }
    if cx.settings.pinned_apps.iter().any(|pin| pin.key == app.key) {
        focus_rule(ui, cx, app, name, indent);
    }
    if app.streams.len() > 1 {
        streams(ui, st, cx, app, name, indent);
    }
    let source = match (&app.key, &app.exe_path) {
        (AppKey::System, _) => "Windows system sounds".to_string(),
        (AppKey::Package(aumid), _) => format!("Packaged app · {aumid}"),
        (_, Some(path)) => path.display().to_string(),
        (AppKey::Pid(pid), None) => format!("Process {pid} (path unavailable)"),
        _ => String::new(),
    };
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let mut job = LayoutJob::simple_singleline(source.clone(), FontId::proportional(11.5), p.text_muted);
        job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width() - 8.0);
        ui.label(job).on_hover_text(source);
    });
}

/// "Lower while in the background" for a pinned app (opt-in).
fn focus_rule(ui: &mut Ui, cx: &mut MixerCtx, app: &AppEntry, name: &str, indent: f32) {
    let rules = &mut cx.settings.focus_rules;
    let index = rules.iter().position(|r| r.key == app.key);
    let mut on = index.is_some();
    ui.horizontal_wrapped(|ui| {
        ui.add_space(indent);
        let toggle = widgets::scroll_on_focus(ui.checkbox(&mut on, "Lower in background"));
        widgets::set_name(ui, &toggle, &format!("Lower {name} while another app is in front"));
        let toggled = toggle
            .on_hover_text(
                "Lower this app while another app's window is in front; its level comes back when you return to it",
            )
            .changed();
        if toggled && on {
            rules.push(crate::settings::FocusRule {
                key: app.key.clone(),
                name: name.to_owned(),
                keep_percent: 30,
                mute: false,
            });
        } else if toggled {
            rules.retain(|r| r.key != app.key);
        }
        if let Some(rule) = rules.iter_mut().find(|r| r.key == app.key) {
            let keep =
                ui.add_enabled(!rule.mute, egui::DragValue::new(&mut rule.keep_percent).range(0..=90).suffix("%"));
            widgets::set_name(ui, &keep, &format!("{name} keeps this percent of its volume in the background"));
            widgets::scroll_on_focus(keep);
            let mute = widgets::scroll_on_focus(ui.checkbox(&mut rule.mute, "Mute"));
            widgets::set_name(ui, &mute, &format!("Mute {name} in the background"));
        }
    });
}

/// How long a device choice shows as "Applying…" before the snapshot is trusted again.
const ROUTING_PENDING: Duration = Duration::from_secs(3);

/// What the output-device status line says for an app.
fn route_status(app: &AppEntry, snap: &AudioSnapshot, name: &str) -> String {
    let device_name = |id: &str| snap.outputs.iter().find(|d| d.id.eq_ignore_ascii_case(id)).map(|d| d.name.as_str());
    match &app.route {
        _ if !snap.can_route_apps => "Choosing a device per app isn't supported on this version of Windows.".into(),
        Route::SystemDefault => "Follows the system default output.".into(),
        Route::Unknown => format!("Windows didn't report {name}'s device (it may run as administrator)."),
        Route::Device(id) => match device_name(id) {
            None => "Set to a device that isn't connected; Windows uses the default until it returns.".into(),
            Some(device) if app.streams.iter().any(|s| s.device_id.eq_ignore_ascii_case(id)) => {
                format!("Plays on {device}.")
            }
            Some(device) => {
                format!("Set to {device}. Some apps switch only when they restart playback or are reopened.")
            }
        },
    }
}

/// The output device an app uses, chosen per app (Windows remembers it for the app).
fn output_choice(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, app: &AppEntry, name: &str, indent: f32) {
    let look = cx.look;
    let p = &look.p;
    let now = Instant::now();
    let snapshot_route = match &app.route {
        Route::Device(id) => Some(id.clone()),
        _ => None,
    };
    let pending =
        st.routing.get(&app.key).filter(|(want, at)| now - *at < ROUTING_PENDING && *want != snapshot_route).cloned();
    let shown = pending.as_ref().map_or(snapshot_route.clone(), |(want, _)| want.clone());
    if pending.is_none() {
        st.routing.remove(&app.key);
    } else {
        ui.ctx().request_repaint_after(ROUTING_PENDING);
    }
    let label = |id: &Option<String>| match id {
        None => "System default".to_owned(),
        Some(id) => cx
            .snap
            .outputs
            .iter()
            .find(|d| d.id.eq_ignore_ascii_case(id))
            .map_or_else(|| "Disconnected device".to_owned(), |d| crate::audio::device_label(d, &cx.snap.outputs)),
    };
    let mut choice = None;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.label(RichText::new("Output").color(p.text_muted));
        ui.add_enabled_ui(cx.snap.can_route_apps && app.route != Route::Unknown, |ui| {
            let look = cx.look;
            let mut combo = egui::ComboBox::from_id_salt(("app-output", &app.key))
                .width((ui.available_width() - 8.0).clamp(80.0, 240.0))
                .truncate()
                .selected_text(typography::rich(label(&shown), Role::CalloutStrong, look));
            if look.icons {
                combo = combo.icon(widgets::combo_icon(look));
            }
            let response = combo
                .show_ui(ui, |ui| {
                    let options = std::iter::once(None).chain(cx.snap.outputs.iter().map(|d| Some(d.id.clone())));
                    for option in options {
                        if ui.selectable_label(option == shown, label(&option)).clicked() && option != shown {
                            choice = Some(option);
                        }
                    }
                })
                .response;
            widgets::set_name(ui, &response, &format!("{name} output device"));
            response.on_hover_text("Windows remembers this for the app, also after it restarts");
        });
    });
    if let Some(device) = choice {
        st.routing.insert(app.key.clone(), (device.clone(), now));
        cx.audio.send(Command::SetAppOutput { key: app.key.clone(), device });
    }
    let status = if pending.is_some() { "Applying…".to_owned() } else { route_status(app, cx.snap, name) };
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.add(egui::Label::new(RichText::new(status).small().color(p.text_muted)).wrap());
    });
}

/// Two small buttons that move a list item; returns -1 (earlier), 1 (later) or 0.
/// The ends of the list have their button disabled. `right_to_left` is for rows
/// laid out right to left, where the buttons are added in reverse so they still
/// read Earlier, Later.
fn order_buttons(
    ui: &mut Ui,
    index: usize,
    len: usize,
    texts: (&str, &str),
    names: (&str, &str),
    right_to_left: bool,
) -> isize {
    let mut delta = 0;
    let mut add = |ui: &mut Ui, enabled: bool, text: &str, name: &str, step: isize| {
        let button = ui.add_enabled(enabled, egui::Button::new(text).small());
        widgets::set_name(ui, &button, name);
        if widgets::scroll_on_focus(button).clicked() {
            delta = step;
        }
    };
    let (earlier, later) = ((index > 0, texts.0, names.0, -1), (index + 1 < len, texts.1, names.1, 1));
    for (enabled, text, name, step) in if right_to_left { [later, earlier] } else { [earlier, later] } {
        add(ui, enabled, text, name, step);
    }
    delta
}

/// A name of the user's own for an app. It is applied when the field loses focus
/// (Enter, or clicking elsewhere); an empty field, or Reset, goes back to the
/// name Windows reports.
fn rename_row(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, app: &AppEntry, key: &AppKey, indent: f32) {
    let p = &cx.look.p;
    let original = st.default_name(app).to_owned();
    let current = crate::audio::preferences::custom_name(&cx.settings.app_names, key).map(str::to_owned);
    let draft = st.rename.entry(key.clone()).or_insert_with(|| current.clone().unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.label(RichText::new("Name").color(p.text_muted));
        let width = (ui.available_width() - 90.0).clamp(80.0, 200.0);
        let field = ui.add(egui::TextEdit::singleline(draft).hint_text(&original).desired_width(width).char_limit(60));
        widgets::set_name(ui, &field, &format!("Display name for {original}"));
        let field = widgets::scroll_on_focus(field);
        if field.lost_focus() && crate::audio::preferences::set_custom_name(&mut cx.settings.app_names, key, draft) {
            *draft = crate::audio::preferences::custom_name(&cx.settings.app_names, key).unwrap_or_default().to_owned();
        }
        let reset = ui.add_enabled(current.is_some(), egui::Button::new("Reset").small());
        widgets::set_name(ui, &reset, &format!("Reset the name of {original}"));
        if widgets::scroll_on_focus(reset).clicked() {
            crate::audio::preferences::set_custom_name(&mut cx.settings.app_names, key, "");
            draft.clear();
        }
    });
}

/// Per-session controls, for apps whose streams need different levels.
fn streams(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, app: &AppEntry, name: &str, indent: f32) {
    let look = cx.look;
    let p = &look.p;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.label(RichText::new("Streams").color(p.text_muted));
    });
    for (i, stream) in app.streams.iter().enumerate() {
        let n = i + 1;
        let activity = if stream.activity == Activity::Playing { "playing" } else { "idle" };
        ui.push_id(("stream", &stream.id), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.label(RichText::new(format!("{n}")).small().color(p.text_muted))
                    .on_hover_text(format!("Stream {n} · process {} · {activity}", stream.pid));
                let mute = format!("{} {name} stream {n}", if stream.muted { "Unmute" } else { "Mute" });
                let target = Target::Volume(Ctl::Stream(stream.id.clone()));
                let mut v = st.value(&target, stream.volume);
                let g = glyph::volume(v, stream.muted);
                if icon_button(
                    ui,
                    look,
                    g,
                    if stream.muted { "M" } else { "On" },
                    &mute,
                    stream.muted.then_some(p.danger),
                )
                .clicked()
                {
                    cx.audio.send(Command::SetStreamMute { id: stream.id.clone(), muted: !stream.muted });
                }
                let width = ui.available_width() - widgets::percent_width(ui, look) - ui.spacing().item_spacing.x - 4.0;
                let a11y = format!("{name} stream {n} volume");
                let response = Slider { muted: stream.muted, ..Slider::volume(&mut v, &a11y, width) }
                    .wheel_percent(cx.settings.wheel_step)
                    .show(ui, p);
                if response.changed() {
                    st.hold(target.clone(), v);
                    cx.audio.send(Command::SetStreamVolume { id: stream.id.clone(), volume: v });
                } else if response.dragged() {
                    st.hold(target, v);
                }
                percent_label(ui, look, &format!("{}%", to_percent(v)), stream.muted);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(shade: u8) -> egui::ColorImage {
        egui::ColorImage::new([2, 2], vec![Color32::from_gray(shade); 4])
    }

    fn icon(image: Option<egui::ColorImage>) -> IconResult {
        IconResult { image, display_name: None }
    }

    #[test]
    fn cache_prunes_old_pixel_sizes_and_failed_lookups_of_apps_that_left() {
        let ctx = egui::Context::default();
        let (a, b, c) = (AppKey::Exe("a.exe".into()), AppKey::Exe("b.exe".into()), AppKey::Exe("c.exe".into()));
        let mut state = MixerState::default();
        state.ingest_icons(
            &ctx,
            vec![((a.clone(), 28), icon(None)), ((a.clone(), 42), icon(None)), ((b.clone(), 42), icon(None))],
        );
        state.ingest_icons(&ctx, vec![((c.clone(), 42), icon(Some(image(9))))]);
        state.names.insert(b.clone(), "B".into());
        state.names.insert(c.clone(), "C".into());
        state.prune_cache(&HashSet::from([a.clone()]), 42);
        assert_eq!(
            state.cached_icon_keys(),
            HashSet::from([(a, 42), (c.clone(), 42)]),
            "b's failed lookup goes; c's icon stays for a relaunch"
        );
        assert!(!state.names.contains_key(&b));
        assert!(state.names.contains_key(&c));
    }

    #[test]
    fn the_output_status_says_what_windows_actually_does() {
        let device =
            |id: &str, name: &str| crate::audio::DeviceInfo { id: id.into(), name: name.into(), is_default: false };
        let stream = |device_id: &str| crate::audio::StreamEntry {
            id: "s".into(),
            pid: 1,
            device_id: device_id.into(),
            volume: 1.0,
            muted: false,
            activity: Activity::Playing,
        };
        let mut snap =
            AudioSnapshot { outputs: vec![device("hp", "Headphones"), device("tv", "TV")], ..Default::default() };
        let mut app = crate::audio::preferences::PinnedApp {
            key: AppKey::Exe("a.exe".into()),
            name: "A".into(),
            exe_path: None,
            aumid: None,
        }
        .placeholder();
        app.streams = vec![stream("hp")];
        assert!(route_status(&app, &snap, "A").contains("isn't supported"));
        snap.can_route_apps = true;
        assert_eq!(route_status(&app, &snap, "A"), "Follows the system default output.");
        app.route = Route::Device("tv".into());
        assert!(route_status(&app, &snap, "A").contains("switch only when they restart"), "not moved yet");
        app.streams = vec![stream("tv")];
        assert_eq!(route_status(&app, &snap, "A"), "Plays on TV.");
        app.route = Route::Device("gone".into());
        assert!(route_status(&app, &snap, "A").contains("isn't connected"));
        app.route = Route::Unknown;
        assert!(route_status(&app, &snap, "A").contains("administrator"));
    }

    #[test]
    fn identical_icons_are_not_uploaded_again_but_changed_ones_are() {
        let ctx = egui::Context::default();
        let key = (AppKey::Exe("a.exe".into()), 28);
        let mut state = MixerState::default();
        state.ingest_icons(&ctx, vec![(key.clone(), icon(Some(image(10))))]);
        let first = state.texture(&key).expect("uploaded").id();
        state.ingest_icons(&ctx, vec![(key.clone(), icon(Some(image(10))))]);
        assert_eq!(state.texture(&key).unwrap().id(), first, "a relaunch with the same icon reuses the texture");
        state.ingest_icons(&ctx, vec![(key.clone(), icon(Some(image(200))))]);
        assert_ne!(state.texture(&key).unwrap().id(), first, "an updated executable's icon replaces it");
        state.ingest_icons(&ctx, vec![(key.clone(), icon(None))]);
        assert!(state.texture(&key).is_none(), "a failed lookup is shown as the fallback tile");
    }

    #[test]
    fn only_the_most_recent_icons_of_apps_that_left_are_kept() {
        let ctx = egui::Context::default();
        let mut state = MixerState::default();
        let keys: Vec<_> = (0..RECENT_ICONS + 8).map(|i| AppKey::Exe(format!("app{i}.exe"))).collect();
        for key in &keys {
            state.ingest_icons(&ctx, vec![((key.clone(), 28), icon(Some(image(1))))]);
            state.prune_cache(&HashSet::from([key.clone()]), 28);
            std::thread::sleep(Duration::from_millis(1));
        }
        state.prune_cache(&HashSet::new(), 28);
        assert_eq!(state.textures.len(), RECENT_ICONS);
        assert!(state.texture(&(keys.last().unwrap().clone(), 28)).is_some(), "the latest is kept");
        assert!(state.texture(&(keys[0].clone(), 28)).is_none(), "the oldest is evicted");
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::audio::preferences::PinnedApp;
    use crate::audio::{DeviceInfo, DeviceState, EndpointState};
    use crate::settings::{Density, MIN_WIDTH};
    use crate::ui::test_support::{Frame, Harness, is_interactive, past_right_edge, unnamed};

    const LONG_DEVICE: &str = "HDMI-1 (NVIDIA High Definition Audio)";
    const LONG_MIC: &str = "Microphone (USB Audio Device With A Long Name)";

    fn endpoint(id: &str, name: &str, volume: f32) -> EndpointState {
        EndpointState { id: id.into(), name: name.into(), volume, muted: false, balance: Some(0.0) }
    }

    fn device(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo { id: id.into(), name: name.into(), is_default: true }
    }

    fn app(key: AppKey, name: &str) -> AppEntry {
        AppEntry {
            key,
            name: name.into(),
            exe_path: None,
            aumid: None,
            icon_resource: None,
            volume: 1.0,
            muted: false,
            activity: Activity::Idle,
            session_count: 1,
            balance: BalanceState::Unavailable,
            streams: Vec::new(),
            route: crate::audio::Route::SystemDefault,
        }
    }

    pub(super) fn snapshot() -> AudioSnapshot {
        AudioSnapshot {
            status: ServiceStatus::Ok,
            output: Some(endpoint("out", LONG_DEVICE, 1.0)),
            input: Some(endpoint("in", LONG_MIC, 0.5)),
            outputs: vec![device("out", LONG_DEVICE)],
            inputs: vec![device("in", LONG_MIC)],
            apps: vec![
                app(AppKey::System, "System sounds"),
                app(AppKey::Exe(r"c:\apps\player.exe".into()), "A Media Player With A Remarkably Long Display Name"),
            ],
            can_switch_default: true,
            ..Default::default()
        }
    }

    fn offline_pin() -> PinnedApp {
        PinnedApp {
            key: AppKey::Exe(r"c:\apps\gone.exe".into()),
            name: "Gone App With A Rather Long Display Name".into(),
            exe_path: None,
            aumid: None,
        }
    }

    /// Renders the Mixer view at the content width of a window `window_width` wide.
    fn render_mixer(density: Density, window_width: f32, settings: Settings) -> (f32, Frame) {
        let harness = Harness::new(density);
        let look = harness.look;
        let width = window_width - 2.0 * look.m.pad;
        let (audio, snap, meters) = (AudioHandle::detached(), snapshot(), Meters::default());
        let (mut settings, mut icons, mut state) = (settings, IconService::detached(), MixerState::default());
        let frame = harness.run(width, 560.0, |ui| {
            let mut cx = MixerCtx {
                audio: &audio,
                snap: &snap,
                meters: &meters,
                look: &look,
                settings: &mut settings,
                icons: &mut icons,
            };
            show(ui, &mut state, &mut cx);
        });
        (width, frame)
    }

    fn assert_fits(what: &str, width: f32, frame: &Frame) {
        let clipped: Vec<_> = past_right_edge(&frame.widgets, width)
            .iter()
            .map(|w| format!("{:?} '{}' {:?}", w.role, w.name, w.bounds))
            .collect();
        assert!(clipped.is_empty(), "{what}: controls past the right edge ({width}): {clipped:#?}");
        assert!(
            frame.content_right <= width + 0.5,
            "{what}: content reaches {} in a {width}-wide window",
            frame.content_right
        );
    }

    #[test]
    fn mixer_view_stays_inside_a_minimum_width_window() {
        for density in [Density::Comfortable, Density::Compact] {
            let (width, frame) = render_mixer(density, MIN_WIDTH, Settings::default());
            assert_fits(&format!("{density:?}"), width, &frame);
        }
    }

    #[test]
    fn offline_pin_with_a_long_name_stays_inside_the_window() {
        for window in [MIN_WIDTH, 480.0] {
            let mut settings = Settings::default();
            settings.pinned_apps.push(offline_pin());
            let (width, frame) = render_mixer(Density::Comfortable, window, settings);
            assert_fits(&format!("window {window}"), width, &frame);
        }
    }

    #[test]
    fn every_mixer_control_has_an_accessible_name() {
        let mut settings = Settings::default();
        settings.pinned_apps.push(offline_pin());
        let (_, frame) = render_mixer(Density::Comfortable, MIN_WIDTH, settings);
        let nameless: Vec<_> = unnamed(&frame.widgets).iter().map(|w| format!("{:?} {:?}", w.role, w.bounds)).collect();
        assert!(nameless.is_empty(), "controls without an accessible name: {nameless:#?}");
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        for expected in
            ["Output device", "Input device", "Filter apps", "Unpin Gone App With A Rather Long Display Name"]
        {
            assert!(names.contains(&expected), "missing accessible name '{expected}' in {names:?}");
        }
    }

    #[test]
    fn mixer_view_has_a_single_settings_route_and_no_properties_panel() {
        let (_, frame) = render_mixer(Density::Comfortable, 480.0, Settings::default());
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(!names.contains(&"Settings"), "the header gear duplicated the Settings tab: {names:?}");
        assert!(
            !names.iter().any(|n| n.contains("Open Windows sound settings")),
            "sound properties belong to the Devices view: {names:?}"
        );
    }

    #[test]
    fn each_balance_state_shows_its_own_control_and_fits() {
        let key = AppKey::Exe(r"c:\apps\player.exe".into());
        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let slider = "A Media Player With A Remarkably Long Display Name balance";
        for (balance, has_slider, has_center) in [
            (BalanceState::Uniform(0.3), true, false),
            (BalanceState::Mixed, false, true),
            (BalanceState::Unavailable, false, false),
        ] {
            let mut snap = snapshot();
            snap.apps[1].balance = balance;
            let (audio, meters) = (AudioHandle::detached(), Meters::default());
            let (mut settings, mut icons, mut state) =
                (Settings::default(), IconService::detached(), MixerState::default());
            state.expanded.insert(key.clone());
            let frame = harness.run(width, 560.0, |ui| {
                let mut cx = MixerCtx {
                    audio: &audio,
                    snap: &snap,
                    meters: &meters,
                    look: &look,
                    settings: &mut settings,
                    icons: &mut icons,
                };
                show(ui, &mut state, &mut cx);
            });
            let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
            assert_eq!(names.contains(&slider), has_slider, "{balance:?}: {names:?}");
            assert_eq!(names.contains(&"Center all"), has_center, "{balance:?}: {names:?}");
            assert_fits(&format!("{balance:?}"), width, &frame);
        }
    }

    fn render_with(snap: &AudioSnapshot, settings: &mut Settings, expanded: &[AppKey], devices: bool) -> (f32, Frame) {
        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let (audio, meters) = (AudioHandle::detached(), Meters::default());
        let (mut icons, mut state) = (IconService::detached(), MixerState::default());
        state.expanded.extend(expanded.iter().cloned());
        let frame = harness.run(width, 900.0, |ui| {
            let mut cx = MixerCtx {
                audio: &audio,
                snap,
                meters: &meters,
                look: &look,
                settings: &mut *settings,
                icons: &mut icons,
            };
            if devices {
                show_devices(ui, &mut state, &mut cx);
            } else {
                show(ui, &mut state, &mut cx);
            }
        });
        (width, frame)
    }

    #[test]
    fn solo_streams_and_device_pins_have_named_controls_that_fit() {
        let player = AppKey::Exe(r"c:\apps\player.exe".into());
        let player_name = "A Media Player With A Remarkably Long Display Name";
        let mut snap = snapshot();
        let stream = |id: &str, volume: f32| crate::audio::StreamEntry {
            id: id.into(),
            pid: 42,
            device_id: "out".into(),
            volume,
            muted: false,
            activity: Activity::Playing,
        };
        snap.apps[1].session_count = 2;
        snap.apps[1].streams = vec![stream("s1", 0.3), stream("s2", 0.9)];

        let mut settings = Settings::default();
        let (width, frame) = render_with(&snap, &mut settings, std::slice::from_ref(&player), false);
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(!names.iter().any(|n| n.starts_with("Solo ")), "unpinned apps have no Solo button: {names:?}");
        for expected in [format!("{player_name} stream 1 volume"), format!("Mute {player_name} stream 2")] {
            assert!(names.contains(&expected.as_str()), "missing {expected}: {names:?}");
        }
        assert_fits("streams", width, &frame);

        settings.pinned_apps.push(PinnedApp::from_app(&snap.apps[1]).unwrap());
        let (width, frame) = render_with(&snap, &mut settings, &[], false);
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&format!("Solo {player_name}").as_str()), "{names:?}");
        assert_fits("pinned", width, &frame);

        snap.solo = Some(player.clone());
        let (width, frame) = render_with(&snap, &mut settings, &[], false);
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&format!("End solo for {player_name}").as_str()), "{names:?}");
        assert!(names.contains(&"End solo"), "the header offers a way out: {names:?}");
        assert_fits("solo", width, &frame);

        // A pinned, expanded app offers its focus rule; mic ducking shows a way to turn it off.
        snap.solo = None;
        snap.mic_ducking = true;
        settings.focus_rules.push(crate::settings::FocusRule {
            key: player.clone(),
            name: player_name.into(),
            keep_percent: 30,
            mute: false,
        });
        let (width, frame) = render_with(&snap, &mut settings, std::slice::from_ref(&player), false);
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        for expected in [
            format!("Lower {player_name} while another app is in front"),
            format!("{player_name} keeps this percent of its volume in the background"),
            format!("{player_name} output device"),
            "Turn off microphone ducking".to_owned(),
        ] {
            assert!(names.contains(&expected.as_str()), "missing {expected}: {names:?}");
        }
        assert_fits("focus and ducking", width, &frame);

        settings
            .pinned_devices
            .push(crate::audio::preferences::PinnedDevice { id: "gone".into(), name: "Bluetooth Headphones".into() });
        let mut devices = device_snapshot();
        devices.solo = None;
        let (width, frame) = render_with(&devices, &mut settings, &[], true);
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&format!("Pin {LONG_DEVICE} for output cycling").as_str()), "{names:?}");
        assert!(names.contains(&"Unpin Bluetooth Headphones"), "a disconnected pin can be removed: {names:?}");
        assert_fits("devices", width, &frame);
    }

    fn named_widgets(frame: &Frame) -> Vec<(&str, bool)> {
        frame.widgets.iter().map(|w| (w.name.as_str(), w.disabled)).collect()
    }

    fn pin(key: &AppKey, name: &str) -> PinnedApp {
        PinnedApp { key: key.clone(), name: name.into(), exe_path: None, aumid: None }
    }

    #[test]
    fn custom_names_and_order_controls_are_named_disabled_at_the_ends_and_fit() {
        use crate::audio::preferences::AppName;
        let (player, other) = (AppKey::Exe(r"c:\apps\player.exe".into()), AppKey::Exe(r"c:\apps\other.exe".into()));
        let player_name = "A Media Player With A Remarkably Long Display Name";
        let mut snap = snapshot();
        snap.apps.push(app(other.clone(), "Other"));
        let mut settings =
            Settings { pinned_apps: vec![pin(&player, player_name), pin(&other, "Other")], ..Default::default() };
        settings.app_names.push(AppName { key: player.clone(), name: "Player".into() });

        let (width, frame) = render_with(&snap, &mut settings, &[player.clone(), other.clone()], false);
        let widgets = named_widgets(&frame);
        let has = |name: &str, disabled: bool| widgets.contains(&(name, disabled));
        assert!(
            frame.widgets.iter().any(|w| w.name.starts_with("Player, ")),
            "the custom name replaces the reported one: {widgets:?}"
        );
        assert!(!frame.widgets.iter().any(|w| w.name.starts_with(player_name) && w.name.contains("details")));
        assert!(has(&format!("Display name for {player_name}"), false), "{widgets:?}");
        assert!(has(&format!("Reset the name of {player_name}"), false), "a renamed app can be reset");
        assert!(has("Reset the name of Other", true), "an app with no custom name has nothing to reset");
        assert!(has("Move Player up", true) && has("Move Player down", false), "the first pin can only move down");
        assert!(has("Move Other up", false) && has("Move Other down", true), "the last pin can only move up");
        assert_fits("rename and order", width, &frame);
    }

    #[test]
    fn typing_a_name_and_pressing_enter_renames_the_app_and_reset_clears_it() {
        use egui::{Event, Key, Modifiers, PointerButton};
        let player = AppKey::Exe(r"c:\apps\player.exe".into());
        let player_name = "A Media Player With A Remarkably Long Display Name";
        let snap = snapshot();
        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let (audio, meters) = (AudioHandle::detached(), Meters::default());
        let (mut icons, mut state) = (IconService::detached(), MixerState::default());
        let settings = std::cell::RefCell::new(Settings::default());
        state.expanded.insert(player.clone());
        let mut frame = |events: Vec<Event>| {
            harness.run_with_events(width, 900.0, events, |ui| {
                let mut cx = MixerCtx {
                    audio: &audio,
                    snap: &snap,
                    meters: &meters,
                    look: &look,
                    settings: &mut settings.borrow_mut(),
                    icons: &mut icons,
                };
                show(ui, &mut state, &mut cx);
            })
        };
        let centre = |frame: &Frame, name: &str| {
            frame.widgets.iter().find(|w| w.name == name).and_then(|w| w.bounds).expect(name).center()
        };
        let click = |pos: egui::Pos2, pressed: bool| {
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
            ]
        };
        let field = format!("Display name for {player_name}");

        let first = frame(Vec::new());
        let pos = centre(&first, &field);
        frame(click(pos, true));
        frame(click(pos, false));
        frame(vec![Event::Text("My Player".into())]);
        for pressed in [true, false] {
            frame(vec![Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
        }
        let after = frame(Vec::new());
        assert!(after.widgets.iter().any(|w| w.name.starts_with("My Player, ")), "the row shows the new name");
        assert_eq!(
            settings.borrow().app_names,
            vec![crate::audio::preferences::AppName { key: player.clone(), name: "My Player".into() }]
        );

        let reset = centre(&after, &format!("Reset the name of {player_name}"));
        frame(click(reset, true));
        frame(click(reset, false));
        let cleared = frame(Vec::new());
        assert!(settings.borrow().app_names.is_empty(), "Reset removes the custom name");
        let names: Vec<&str> = cleared.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(names.iter().any(|n| n.starts_with(player_name)), "the row is back to the reported name: {names:?}");
        assert!(!names.iter().any(|n| n.starts_with("My Player")));
    }

    #[test]
    fn pinned_output_cycle_buttons_are_named_and_fit_for_connected_and_missing_devices() {
        let mut settings = Settings {
            pinned_devices: vec![
                crate::audio::preferences::PinnedDevice { id: "out".into(), name: LONG_DEVICE.into() },
                crate::audio::preferences::PinnedDevice { id: "gone".into(), name: "Bluetooth Headphones".into() },
            ],
            ..Default::default()
        };
        let (width, frame) = render_with(&device_snapshot(), &mut settings, &[], true);
        let widgets = named_widgets(&frame);
        let has = |name: &str, disabled: bool| widgets.contains(&(name, disabled));
        assert!(has(&format!("Cycle {LONG_DEVICE} earlier"), true), "the first pin can't go earlier: {widgets:?}");
        assert!(has(&format!("Cycle {LONG_DEVICE} later"), false));
        assert!(has("Cycle Bluetooth Headphones earlier", false), "{widgets:?}");
        assert!(has("Cycle Bluetooth Headphones later", true), "the last pin can't go later");
        assert_fits("cycle order", width, &frame);
    }

    fn device_snapshot() -> AudioSnapshot {
        let mut snap = snapshot();
        snap.device_states = vec![DeviceState {
            flow: Flow::Output,
            info: device("out", LONG_DEVICE),
            state: Ok(endpoint("out", LONG_DEVICE, 1.0)),
        }];
        snap
    }

    #[test]
    fn devices_view_has_the_properties_section_and_fits() {
        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let (snap, audio, meters) = (device_snapshot(), AudioHandle::detached(), Meters::default());
        let (mut settings, mut icons, mut state) =
            (Settings::default(), IconService::detached(), MixerState::default());
        let frame = harness.run(width, 600.0, |ui| {
            let mut cx = MixerCtx {
                audio: &audio,
                snap: &snap,
                meters: &meters,
                look: &look,
                settings: &mut settings,
                icons: &mut icons,
            };
            show_devices(ui, &mut state, &mut cx);
        });
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&"Balance and sound properties"), "properties section missing: {names:?}");
        assert_fits("devices view", width, &frame);
        assert!(unnamed(&frame.widgets).is_empty(), "unnamed controls: {:?}", unnamed(&frame.widgets));
    }

    #[test]
    fn sound_property_controls_are_named_and_fit_at_minimum_width() {
        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let (snap, audio, meters) = (device_snapshot(), AudioHandle::detached(), Meters::default());
        let (mut settings, mut icons, mut state) =
            (Settings::default(), IconService::detached(), MixerState::default());
        let ep = endpoint("out", LONG_DEVICE, 1.0);
        let frame = harness.run(width, 900.0, |ui| {
            let cx = MixerCtx {
                audio: &audio,
                snap: &snap,
                meters: &meters,
                look: &look,
                settings: &mut settings,
                icons: &mut icons,
            };
            endpoint_balance(ui, &mut state, &cx, &ep);
            sound_properties(ui, &cx, &ep);
        });
        let names: Vec<_> = frame.widgets.iter().map(|w| w.name.as_str()).collect();
        for expected in
            ["Open Windows sound settings", "Open mono audio settings", "HDMI-1 (NVIDIA High Definition Audio) balance"]
        {
            assert!(names.contains(&expected), "missing '{expected}' in {names:?}");
        }
        assert_fits("sound properties", width, &frame);
    }

    #[test]
    fn expanded_devices_view_with_mono_stereo_and_error_states_fits_and_operates() {
        use crate::audio::properties::DeviceProperties;

        for density in [Density::Comfortable, Density::Compact] {
            let harness = Harness::new(density);
            let look = harness.look;
            let width = MIN_WIDTH - 2.0 * look.m.pad;
            let height = 440.0;
            let mut snap = snapshot();
            snap.outputs = vec![
                DeviceInfo { id: "out-stereo".into(), name: LONG_DEVICE.into(), is_default: true },
                DeviceInfo { id: "out-mono".into(), name: "Mono Speaker Dock".into(), is_default: false },
                DeviceInfo { id: "out-err".into(), name: "Disconnected Optical Output".into(), is_default: false },
            ];
            snap.inputs = vec![
                DeviceInfo { id: "in-mono".into(), name: LONG_MIC.into(), is_default: true },
                DeviceInfo { id: "in-stereo".into(), name: "Studio Stereo Line In".into(), is_default: false },
            ];
            snap.device_states = vec![
                DeviceState {
                    flow: Flow::Output,
                    info: snap.outputs[0].clone(),
                    state: Ok(EndpointState {
                        id: "out-stereo".into(),
                        name: LONG_DEVICE.into(),
                        volume: 0.8,
                        muted: false,
                        balance: Some(0.4),
                    }),
                },
                DeviceState {
                    flow: Flow::Output,
                    info: snap.outputs[1].clone(),
                    state: Ok(EndpointState {
                        id: "out-mono".into(),
                        name: "Mono Speaker Dock".into(),
                        volume: 0.6,
                        muted: false,
                        balance: None,
                    }),
                },
                DeviceState {
                    flow: Flow::Output,
                    info: snap.outputs[2].clone(),
                    state: Err("Device unavailable: 0x88890004".into()),
                },
                DeviceState {
                    flow: Flow::Input,
                    info: snap.inputs[0].clone(),
                    state: Ok(EndpointState {
                        id: "in-mono".into(),
                        name: LONG_MIC.into(),
                        volume: 0.5,
                        muted: false,
                        balance: None,
                    }),
                },
                DeviceState {
                    flow: Flow::Input,
                    info: snap.inputs[1].clone(),
                    state: Ok(EndpointState {
                        id: "in-stereo".into(),
                        name: "Studio Stereo Line In".into(),
                        volume: 0.7,
                        muted: false,
                        balance: Some(-0.25),
                    }),
                },
            ];
            snap.properties.insert(
                "out-mono".into(),
                DeviceProperties {
                    default_format: Err("Exclusive mode active".into()),
                    mix_format: Err("Mix format unavailable".into()),
                    enhancements: Err("Property unavailable".into()),
                    spatial: Err("Spatial audio state unavailable on this endpoint.".into()),
                },
            );

            let (audio, meters) = (AudioHandle::detached(), Meters::default());
            let (mut settings, mut icons, mut state) =
                (Settings::default(), IconService::detached(), MixerState::default());

            // Expand all 4 "Balance and sound properties" collapsing sections inside the cards via Tab + Space.
            let _ = harness.run(width, height, |ui| {
                let mut cx = MixerCtx {
                    audio: &audio,
                    snap: &snap,
                    meters: &meters,
                    look: &look,
                    settings: &mut settings,
                    icons: &mut icons,
                };
                show_devices(ui, &mut state, &mut cx);
            });
            let mut opened = 0;
            for _ in 0..40 {
                let frame = harness.send_key(width, height, Key::Tab, egui::Modifiers::NONE, |ui| {
                    let mut cx = MixerCtx {
                        audio: &audio,
                        snap: &snap,
                        meters: &meters,
                        look: &look,
                        settings: &mut settings,
                        icons: &mut icons,
                    };
                    show_devices(ui, &mut state, &mut cx);
                });
                if frame.widgets.iter().any(|w| w.focused && w.name == "Balance and sound properties") {
                    let _ = harness.send_key(width, height, Key::Space, egui::Modifiers::NONE, |ui| {
                        let mut cx = MixerCtx {
                            audio: &audio,
                            snap: &snap,
                            meters: &meters,
                            look: &look,
                            settings: &mut settings,
                            icons: &mut icons,
                        };
                        show_devices(ui, &mut state, &mut cx);
                    });
                    opened += 1;
                    if opened == 4 {
                        break;
                    }
                }
            }
            assert_eq!(opened, 4, "all 4 Balance and sound properties headers must be reachable and expandable");

            let frame = harness.run(width, height, |ui| {
                let mut cx = MixerCtx {
                    audio: &audio,
                    snap: &snap,
                    meters: &meters,
                    look: &look,
                    settings: &mut settings,
                    icons: &mut icons,
                };
                show_devices(ui, &mut state, &mut cx);
            });
            assert_fits(&format!("expanded devices view ({density:?})"), width, &frame);
            assert!(unnamed(&frame.widgets).is_empty(), "unnamed controls: {:?}", unnamed(&frame.widgets));
            let unavailable_count =
                frame.widgets.iter().filter(|w| w.name == "Balance: Not available — this device is not stereo").count();
            assert_eq!(unavailable_count, 2, "both mono output and mono input must expose unavailable balance text");
            assert!(
                frame.widgets.iter().any(|w| w.name.contains("Default format unavailable: Exclusive mode active")),
                "error property text must be present"
            );

            // Shift+Tab back up until the top stereo balance slider is focused and inside the viewport,
            // then double-click off-center so only double_clicked() recenters it to 0.0.
            let mut bal_rect = None;
            for _ in 0..24 {
                let f = harness.send_key(width, height, Key::Tab, egui::Modifiers::SHIFT, |ui| {
                    let mut cx = MixerCtx {
                        audio: &audio,
                        snap: &snap,
                        meters: &meters,
                        look: &look,
                        settings: &mut settings,
                        icons: &mut icons,
                    };
                    show_devices(ui, &mut state, &mut cx);
                });
                if let Some(w) = f.widgets.iter().find(|w| w.focused && w.name == format!("{LONG_DEVICE} balance")) {
                    bal_rect = w.bounds;
                    break;
                }
            }
            let bal_rect = bal_rect.expect("stereo balance slider bounds in viewport");
            let click_pos = pos2(bal_rect.left() + bal_rect.width() * 0.75, bal_rect.center().y);
            for pressed in [true, false, true, false] {
                let _ = harness.run_with_events(
                    width,
                    height,
                    vec![
                        egui::Event::PointerMoved(click_pos),
                        egui::Event::PointerButton {
                            pos: click_pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    |ui| {
                        let mut cx = MixerCtx {
                            audio: &audio,
                            snap: &snap,
                            meters: &meters,
                            look: &look,
                            settings: &mut settings,
                            icons: &mut icons,
                        };
                        show_devices(ui, &mut state, &mut cx);
                    },
                );
            }
            assert_eq!(
                state.value(&Target::Balance(Ctl::Device("out-stereo".into())), 0.4),
                0.0,
                "double-clicking stereo balance slider must recenter it to 0.0"
            );
        }
    }

    #[test]
    fn keyboard_tab_shift_tab_and_slider_keys_keep_focus_visible_in_mixer_and_devices() {
        for density in [Density::Comfortable, Density::Compact] {
            for (window_width, height) in [(MIN_WIDTH, 440.0), (480.0, 720.0)] {
                let harness = Harness::new(density);
                let look = harness.look;
                let width = window_width - 2.0 * look.m.pad;
                let mut snap = device_snapshot();
                snap.apps = (0..14)
                    .map(|i| {
                        let mut entry =
                            app(AppKey::Exe(format!(r"c:\apps\app_{i}.exe")), &format!("Application Number {i}"));
                        entry.balance = BalanceState::Uniform(0.0);
                        entry
                    })
                    .collect();
                let (audio, meters) = (AudioHandle::detached(), Meters::default());
                let (mut settings, mut icons, mut state) =
                    (Settings::default(), IconService::detached(), MixerState::default());
                settings.pinned_apps.push(offline_pin());
                state.expanded.insert(AppKey::Exe(r"c:\apps\app_6.exe".into()));

                let _ = harness.run(width, height, |ui| {
                    let mut cx = MixerCtx {
                        audio: &audio,
                        snap: &snap,
                        meters: &meters,
                        look: &look,
                        settings: &mut settings,
                        icons: &mut icons,
                    };
                    let _ = show(ui, &mut state, &mut cx);
                });

                // Tab forward through the offline pin and all live app rows to the last live app at the bottom.
                let mut reached_unpin = false;
                let mut reached_last_live_app = false;
                for _ in 0..80 {
                    let frame = harness.send_key(width, height, Key::Tab, egui::Modifiers::NONE, |ui| {
                        let mut cx = MixerCtx {
                            audio: &audio,
                            snap: &snap,
                            meters: &meters,
                            look: &look,
                            settings: &mut settings,
                            icons: &mut icons,
                        };
                        let _ = show(ui, &mut state, &mut cx);
                    });
                    let focused = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role));
                    if let Some(widget) = focused {
                        if let Some(b) = widget.bounds {
                            assert!(
                                b.min.y >= -1.0 && b.max.y <= height + 1.0,
                                "focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                                widget.name
                            );
                        }
                        if widget.name == "Unpin Gone App With A Rather Long Display Name" {
                            reached_unpin = true;
                        }
                        if widget.name == "Application Number 13 volume" {
                            reached_last_live_app = true;
                            break;
                        }
                    }
                }
                assert!(reached_unpin, "Tab traversal in Mixer must reach the offline pin control");
                assert!(reached_last_live_app, "Tab traversal in Mixer must reach the bottom live app row");

                // Shift+Tab backward all the way back to the first app volume slider.
                let mut reached_first_app = false;
                for _ in 0..80 {
                    let frame = harness.send_key(width, height, Key::Tab, egui::Modifiers::SHIFT, |ui| {
                        let mut cx = MixerCtx {
                            audio: &audio,
                            snap: &snap,
                            meters: &meters,
                            look: &look,
                            settings: &mut settings,
                            icons: &mut icons,
                        };
                        let _ = show(ui, &mut state, &mut cx);
                    });
                    let focused = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role));
                    if let Some(widget) = focused {
                        if let Some(b) = widget.bounds {
                            assert!(
                                b.min.y >= -1.0 && b.max.y <= height + 1.0,
                                "Shift+Tab focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                                widget.name
                            );
                        }
                        if widget.name == "Application Number 0 volume" {
                            reached_first_app = true;
                            break;
                        }
                    }
                }
                assert!(reached_first_app, "Shift+Tab traversal must return to the top app volume slider");

                // Exercise slider keyboard keys (Home, ArrowRight, PageUp, End, ArrowLeft, PageDown)
                // while Application Number 0 volume slider is focused.
                let target = Target::Volume(Ctl::App(AppKey::Exe(r"c:\apps\app_0.exe".into())));
                let mut key_step = |k: Key| -> f32 {
                    let _ = harness.send_key(width, height, k, egui::Modifiers::NONE, |ui| {
                        let mut cx = MixerCtx {
                            audio: &audio,
                            snap: &snap,
                            meters: &meters,
                            look: &look,
                            settings: &mut settings,
                            icons: &mut icons,
                        };
                        let _ = show(ui, &mut state, &mut cx);
                    });
                    state.value(&target, 1.0)
                };
                assert!((key_step(Key::Home) - 0.0).abs() < 0.005);
                assert!((key_step(Key::ArrowRight) - 0.01).abs() < 0.005);
                assert!((key_step(Key::PageUp) - 0.11).abs() < 0.005);
                assert!((key_step(Key::End) - 1.0).abs() < 0.005);
                assert!((key_step(Key::ArrowLeft) - 0.99).abs() < 0.005);
                assert!((key_step(Key::PageDown) - 0.89).abs() < 0.005);

                // Exercise Tab, Space (on CollapsingHeader), and Shift+Tab inside show_devices.
                let dev_harness = Harness::new(density);
                let dev_look = dev_harness.look;
                let mut dev_state = MixerState::default();
                let _ = dev_harness.run(width, height, |ui| {
                    let mut cx = MixerCtx {
                        audio: &audio,
                        snap: &snap,
                        meters: &meters,
                        look: &dev_look,
                        settings: &mut settings,
                        icons: &mut icons,
                    };
                    show_devices(ui, &mut dev_state, &mut cx);
                });
                let mut reached_mono_settings = false;
                for _ in 0..24 {
                    let frame = dev_harness.send_key(width, height, Key::Tab, egui::Modifiers::NONE, |ui| {
                        let mut cx = MixerCtx {
                            audio: &audio,
                            snap: &snap,
                            meters: &meters,
                            look: &dev_look,
                            settings: &mut settings,
                            icons: &mut icons,
                        };
                        show_devices(ui, &mut dev_state, &mut cx);
                    });
                    if let Some(widget) = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role)) {
                        if let Some(b) = widget.bounds {
                            assert!(
                                b.min.y >= -1.0 && b.max.y <= height + 1.0,
                                "Devices focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                                widget.name
                            );
                        }
                        if widget.name == "Balance and sound properties" {
                            let _ = dev_harness.send_key(width, height, Key::Space, egui::Modifiers::NONE, |ui| {
                                let mut cx = MixerCtx {
                                    audio: &audio,
                                    snap: &snap,
                                    meters: &meters,
                                    look: &dev_look,
                                    settings: &mut settings,
                                    icons: &mut icons,
                                };
                                show_devices(ui, &mut dev_state, &mut cx);
                            });
                        }
                        if widget.name == "Open mono audio settings" {
                            reached_mono_settings = true;
                        }
                    }
                }
                assert!(reached_mono_settings, "Tab + Space in Devices must reach Open mono audio settings");
                for _ in 0..16 {
                    let frame = dev_harness.send_key(width, height, Key::Tab, egui::Modifiers::SHIFT, |ui| {
                        let mut cx = MixerCtx {
                            audio: &audio,
                            snap: &snap,
                            meters: &meters,
                            look: &dev_look,
                            settings: &mut settings,
                            icons: &mut icons,
                        };
                        show_devices(ui, &mut dev_state, &mut cx);
                    });
                    if let Some(widget) = frame.widgets.iter().find(|w| w.focused && is_interactive(w.role))
                        && let Some(b) = widget.bounds
                    {
                        assert!(
                            b.min.y >= -1.0 && b.max.y <= height + 1.0,
                            "Devices Shift+Tab focused control '{}' off-screen at {b:?} in {width}x{height} ({density:?})",
                            widget.name
                        );
                    }
                }
            }
        }
    }
}
