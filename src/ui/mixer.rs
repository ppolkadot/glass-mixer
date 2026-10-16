//! The main mixer view: output device and master volume, per-app rows, and the
//! input (microphone) section.
//!
//! Values shown come from the latest Windows snapshot. While the user drags a
//! slider (and briefly afterwards) the locally chosen value is shown instead, so
//! the thumb doesn't jitter while the engine catches up; once the hold expires
//! the display returns to whatever Windows reports.

use super::Look;
use super::widgets::{self, Slider, card, fallback_tile, glyph, icon_button, percent_label, semibold};
use crate::audio::{
    Activity, AppEntry, AppKey, AudioHandle, AudioSnapshot, Command, EndpointState, Flow, Meters, ServiceStatus,
    matches_filter, show_with_idle_filter, to_percent,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Target {
    Volume(Ctl),
    Balance(Ctl),
}

#[derive(Default)]
pub struct MixerState {
    pub filter: String,
    expanded: HashSet<AppKey>,
    overrides: HashMap<Target, (f32, Instant)>,
    textures: HashMap<IconKey, Option<TextureHandle>>,
    names: HashMap<AppKey, String>,
}

impl MixerState {
    fn prune_cache(&mut self, live: &HashSet<AppKey>, px: u32) {
        self.textures.retain(|(key, size), _| live.contains(key) && *size == px);
        self.names.retain(|key, _| live.contains(key));
    }
    #[cfg(test)]
    fn cached_icon_keys(&self) -> HashSet<IconKey> {
        self.textures.keys().cloned().collect()
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

    pub fn ingest_icons(&mut self, ctx: &egui::Context, results: Vec<(IconKey, IconResult)>) {
        for (key, result) in results {
            let texture =
                result.image.map(|img| ctx.load_texture(format!("app-icon-{:?}", key), img, TextureOptions::LINEAR));
            if let Some(name) = result.display_name {
                self.names.insert(key.0.clone(), name);
            }
            self.textures.insert(key, texture);
        }
    }

    fn display_name<'a>(&'a self, app: &'a AppEntry) -> &'a str {
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
                ui.label(RichText::new(message).color(look.p.text));
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Glass Mixer keeps retrying automatically.").small().color(look.p.text_muted));
                if ui.button("Retry now").clicked() {
                    cx.audio.send(Command::Refresh);
                }
            });
            ui.add_space(look.m.gap * 0.5);
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
                if ui.link("Balance and sound properties").clicked() {
                    open_devices = true;
                }
            }
            None if cx.snap.status == ServiceStatus::Starting => {
                ui.label(RichText::new("Connecting to Windows audio…").color(look.p.text_muted));
            }
            None => {
                ui.label(
                    RichText::new("No output device is available. Connect speakers or headphones.")
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
    ui.add_enabled_ui(can_switch, |ui| {
        let response = egui::ComboBox::from_id_salt(("device", kind))
            .width(width.max(80.0))
            .truncate()
            .selected_text(RichText::new(&selected).font(semibold(14.0)))
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
        let width = ui.available_width() - 40.0 - ui.spacing().item_spacing.x;
        let meter =
            (cx.settings.show_meters && cx.snap.endpoint(flow).is_some_and(|d| d.id == ep.id)).then_some(match flow {
                Flow::Output => cx.meters.output,
                Flow::Input => cx.meters.input,
            });
        let a11y = format!("{} volume", ep.name);
        let response = Slider { muted: ep.muted, meter, ..Slider::volume(&mut v, &a11y, width) }.show(ui, &look.p);
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
            egui::CollapsingHeader::new("Balance and sound properties").show(ui, |ui| {
                endpoint_balance(ui, st, cx, ep);
                sound_properties(ui, cx, ep);
            });
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
    if ui.button("Open Windows sound settings").clicked() {
        cx.audio.send(Command::OpenSoundSettings);
    }
    ui.label("Mono audio: live state/control unavailable. Use Windows Accessibility audio settings.");
    if ui.button("Open mono audio settings").clicked() {
        cx.audio.send(Command::OpenMonoSettings);
    }
}

fn endpoint_balance(ui: &mut Ui, st: &mut MixerState, cx: &MixerCtx, ep: &EndpointState) {
    ui.horizontal(|ui| {
        ui.label("Balance");
        if let Some(balance) = ep.balance {
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
        } else {
            ui.label("Not available — this device is not stereo");
        }
    });
}

pub fn show_devices(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) {
    st.expire(ui.ctx());
    ui.horizontal(|ui| {
        ui.heading("Devices");
        if ui.button("Refresh").clicked() {
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
                        if d.is_default {
                            ui.label("Default device");
                        } else if ui
                            .add_enabled(cx.snap.can_switch_default, egui::Button::new("Make default"))
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
        }
    });
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
        ui.label(RichText::new("Apps").font(semibold(15.0)));
        ui.label(RichText::new(format!("{}", visible.len())).small().color(look.p.text_muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, look, glyph::REFRESH, "R", "Refresh audio devices and apps", None).clicked() {
                cx.icons.retry_failed();
                cx.audio.send(Command::Refresh);
            }
            let search = ui.add(
                egui::TextEdit::singleline(&mut st.filter)
                    .hint_text("Filter apps")
                    .desired_width(ui.available_width().min(170.0)),
            );
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

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for app in &visible {
            app_row(ui, st, cx, app);
        }
    });
}

fn placeholder_row(ui: &mut Ui, st: &MixerState, cx: &mut MixerCtx, app: &AppEntry) {
    let look = cx.look;
    ui.push_id((&app.key, "offline"), |ui| {
        card(look).show(ui, |ui| {
            // Fill the row, so long names truncate instead of widening the card.
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let px = (look.m.icon * ui.ctx().pixels_per_point()).ceil() as u32;
                cx.icons.request(app, px);
                ui.scope(|ui| {
                    ui.multiply_opacity(0.45);
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(look.m.icon), Sense::hover());
                    if let Some(Some(tex)) = st.textures.get(&(app.key.clone(), px)) {
                        ui.painter().image(
                            tex.id(),
                            rect,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::from_gray(128),
                        );
                    } else {
                        fallback_tile(ui, rect, &app.name, look, None);
                    }
                });
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(&app.name).strong().color(look.p.text_muted)).truncate());
                    ui.label("Not running");
                });
            });
            ui.add_enabled_ui(false, |ui| {
                ui.horizontal(|ui| {
                    icon_button(ui, look, glyph::MUTE, "M", &format!("{} mute unavailable", app.name), None);
                    widgets::unavailable_slider(ui, look, &format!("{} volume unavailable — not running", app.name));
                });
            });
            ui.horizontal_wrapped(|ui| {
                let unpin = ui.button("Unpin");
                // The visible text is short so the row fits; the accessible name says which app.
                widgets::set_name(ui, &unpin, &format!("Unpin {}", app.name));
                if unpin.on_hover_text(format!("Unpin {}", app.name)).clicked() {
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
        ui.add_space(4.0);
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
            // Icon.
            let px = (m.icon * ui.ctx().pixels_per_point()).ceil() as u32;
            cx.icons.request(app, px);
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(m.icon), Sense::hover());
            match st.textures.get(&(app.key.clone(), px)) {
                Some(Some(tex)) => {
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    let tint = if app.muted { Color32::from_white_alpha(150) } else { Color32::WHITE };
                    ui.painter().image(tex.id(), icon_rect, uv, tint);
                }
                _ => {
                    let g = (app.key == AppKey::System).then_some(glyph::SPEAKER);
                    fallback_tile(ui, icon_rect, &name, look, g);
                }
            }

            // Name and status; click to expand details.
            let name_w = (ui.available_width() * 0.34).clamp(84.0, 210.0);
            let (nrect, nresp) = ui.allocate_exact_size(vec2(name_w, m.row_height - 10.0), Sense::click());
            let mut job = LayoutJob::simple_singleline(name.clone(), semibold(14.0), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(name_w - 4.0);
            let galley = ui.painter().layout_job(job);
            let compact = m.row_height < 46.0;
            let name_y = if compact { nrect.center().y - galley.size().y / 2.0 } else { nrect.top() + 3.0 };
            ui.painter().galley(pos2(nrect.left() + 2.0, name_y), galley, p.text);
            let (status, status_color) = if app.muted {
                ("Muted", p.danger)
            } else if app.activity == Activity::Playing {
                ("Playing", p.playing)
            } else {
                ("Idle", p.text_muted)
            };
            if !compact {
                let sy = nrect.top() + 24.0;
                let mut x = nrect.left() + 2.0;
                if app.activity == Activity::Playing && !app.muted {
                    ui.painter().circle_filled(pos2(x + 3.0, sy + 7.5), 3.0, p.playing);
                    x += 10.0;
                }
                let mut text = status.to_string();
                if app.session_count > 1 {
                    text.push_str(&format!(" · {} streams", app.session_count));
                }
                ui.painter().text(pos2(x, sy), Align2::LEFT_TOP, text, FontId::proportional(12.0), status_color);
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

            // Mute, slider, percentage.
            let target = Target::Volume(Ctl::App(app.key.clone()));
            let mut v = st.value(&target, app.volume);
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
            let width = ui.available_width() - 40.0 - ui.spacing().item_spacing.x - 4.0;
            let meter = cx.settings.show_meters.then(|| cx.meters.apps.get(&app.key).copied().unwrap_or(0.0));
            let a11y = format!("{name} volume");
            let response = Slider { muted: app.muted, meter, ..Slider::volume(&mut v, &a11y, width) }.show(ui, p);
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
                rect,
                CornerRadius::same(8),
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
        if ui.checkbox(&mut pinned, format!("Pin {name}")).changed() {
            if pinned {
                cx.settings.pinned_apps.push(pin);
            } else {
                cx.settings.pinned_apps.retain(|p| p.key != pin.key);
            }
        }
    }
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.label(RichText::new("Balance").color(p.text_muted));
        match app.balance {
            Some(balance) => {
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
            None => {
                ui.label(RichText::new("Not available for this app").small().color(p.text_muted));
            }
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_prunes_missing_apps_and_old_pixel_sizes() {
        let a = AppKey::Exe("a.exe".into());
        let b = AppKey::Exe("b.exe".into());
        let mut state = MixerState::default();
        state.textures.insert((a.clone(), 28), None);
        state.textures.insert((a.clone(), 42), None);
        state.textures.insert((b.clone(), 42), None);
        state.names.insert(b.clone(), "B".into());
        state.prune_cache(&HashSet::from([a.clone()]), 42);
        assert_eq!(state.cached_icon_keys(), HashSet::from([(a, 42)]));
        assert!(!state.names.contains_key(&b));
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::audio::preferences::PinnedApp;
    use crate::audio::{DeviceInfo, DeviceState, EndpointState};
    use crate::settings::{Density, MIN_WIDTH};
    use crate::ui::test_support::{Frame, Harness, past_right_edge, unnamed};

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
            balance: None,
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

    fn device_snapshot() -> AudioSnapshot {
        let mut snap = snapshot();
        snap.device_states = vec![DeviceState {
            flow: Flow::Output,
            info: device("out", LONG_DEVICE),
            state: Ok(endpoint("out", LONG_DEVICE, 1.0)),
        }];
        snap
    }

    /// Balance and sound properties now live in the Devices view, which is one
    /// full-height scroll area, not a nested one in a fixed panel.
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
}
