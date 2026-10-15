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
    matches_filter, to_percent,
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
    Endpoint(Flow),
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
    OpenSettings,
}

pub struct MixerCtx<'a> {
    pub audio: &'a AudioHandle,
    pub snap: &'a AudioSnapshot,
    pub meters: &'a Meters,
    pub look: &'a Look,
    pub settings: &'a Settings,
    pub icons: &'a mut IconService,
}

pub fn show(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) -> Outcome {
    st.expire(ui.ctx());
    let gap = cx.look.m.gap as i8;
    let mut outcome = Outcome::None;

    egui::Panel::top("mixer-header")
        .frame(egui::Frame::NONE.inner_margin(Margin { bottom: gap, ..Margin::ZERO }))
        .resizable(false)
        .show_separator_line(false)
        .show(ui, |ui| {
            if header(ui, st, cx) {
                outcome = Outcome::OpenSettings;
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

/// Returns `true` when Settings was requested.
fn header(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) -> bool {
    let look = cx.look;
    let mut open_settings = false;
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
            let width = ui.available_width() - look.m.button - ui.spacing().item_spacing.x;
            device_combo(ui, cx, Flow::Output, width);
            if icon_button(ui, look, glyph::SETTINGS, "Set", "Settings", None).clicked() {
                open_settings = true;
            }
        });
        match &cx.snap.output {
            Some(ep) => endpoint_row(ui, st, cx, Flow::Output, ep),
            None if cx.snap.status == ServiceStatus::Starting => {
                ui.label(RichText::new("Connecting to Windows audioâ€¦").color(look.p.text_muted));
            }
            None => {
                ui.label(
                    RichText::new("No output device is available. Connect speakers or headphones.")
                        .color(look.p.text_muted),
                );
            }
        }
    });
    open_settings
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
            Some(ep) => endpoint_row(ui, st, cx, Flow::Input, ep),
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
    let selected = current.map_or(none, |e| e.name.as_str());
    let can_switch = cx.snap.can_switch_default && !devices.is_empty();
    let kind = match flow {
        Flow::Output => "output",
        Flow::Input => "input",
    };
    ui.add_enabled_ui(can_switch, |ui| {
        let response = egui::ComboBox::from_id_salt(("device", kind))
            .width(width.max(80.0))
            .truncate()
            .selected_text(RichText::new(selected).font(semibold(14.0)))
            .show_ui(ui, |ui| {
                for d in devices {
                    if ui.selectable_label(d.is_default, &d.name).clicked() && !d.is_default {
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
        response.on_hover_text(hint).on_disabled_hover_text(selected.to_string());
    });
}

fn endpoint_row(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, flow: Flow, ep: &EndpointState) {
    let look = cx.look;
    ui.horizontal(|ui| {
        let target = Target::Volume(Ctl::Endpoint(flow));
        let mut v = st.value(&target, ep.volume);
        let (g, what) = match flow {
            Flow::Output => (glyph::volume(v, ep.muted), "output"),
            Flow::Input => (if ep.muted { glyph::MIC_OFF } else { glyph::MIC }, "microphone"),
        };
        let label = format!("{} {what}", if ep.muted { "Unmute" } else { "Mute" });
        let emphasis = ep.muted.then_some(look.p.danger);
        if icon_button(ui, look, g, if ep.muted { "M" } else { "On" }, &label, emphasis).clicked() {
            cx.audio.send(Command::SetEndpointMute { flow, muted: !ep.muted });
        }
        let width = ui.available_width() - 40.0 - ui.spacing().item_spacing.x;
        let meter = cx.settings.show_meters.then_some(match flow {
            Flow::Output => cx.meters.output,
            Flow::Input => cx.meters.input,
        });
        let a11y = format!("{} volume", ep.name);
        let response = Slider { muted: ep.muted, meter, ..Slider::volume(&mut v, &a11y, width) }.show(ui, &look.p);
        if response.changed() {
            st.hold(target.clone(), v);
            cx.audio.send(Command::SetEndpointVolume { flow, volume: v });
        } else if response.dragged() {
            st.hold(target, v);
        }
        let pct = if ep.muted { "Muted".to_string() } else { format!("{}%", to_percent(v)) };
        percent_label(ui, look, &pct, ep.muted);
    });
}

fn apps_section(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx) {
    let look = cx.look;
    let visible: Vec<&AppEntry> = cx
        .snap
        .apps
        .iter()
        .filter(|a| cx.settings.show_inactive_apps || a.activity == Activity::Playing)
        .filter(|a| matches_filter(st.display_name(a), &st.filter))
        .collect();

    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(RichText::new("Apps").font(semibold(15.0)));
        ui.label(RichText::new(format!("{}", visible.len())).small().color(look.p.text_muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, look, glyph::REFRESH, "R", "Refresh audio devices and apps", None).clicked() {
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
            search.on_hover_text("Filter apps by name (Ctrl+F)");
        });
    });
    ui.add_space(4.0);

    if visible.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let text = if !st.filter.trim().is_empty() {
                format!("No apps match â€œ{}â€.", st.filter.trim())
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
        for app in visible {
            app_row(ui, st, cx, app);
        }
    });
}

fn app_row(ui: &mut Ui, st: &mut MixerState, cx: &mut MixerCtx, app: &AppEntry) {
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
                    text.push_str(&format!(" Â· {} streams", app.session_count));
                }
                ui.painter().text(pos2(x, sy), Align2::LEFT_TOP, text, FontId::proportional(12.0), status_color);
            }
            let details = if expanded { "Hide details" } else { "Show details" };
            widgets::focus_ring(ui, nrect, &nresp, p, 6);
            let a11y = format!("{name}, {status}. {details}");
            nresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &a11y));
            let nresp = nresp.on_hover_text(format!("{name} â€” {status}. Click for details."));
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
        (AppKey::Package(aumid), _) => format!("Packaged app Â· {aumid}"),
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
