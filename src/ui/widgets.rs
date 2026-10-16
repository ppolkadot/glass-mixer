//! Custom widgets: volume slider, icon buttons, toggle switch, segmented
//! control and app tiles. All are keyboard-focusable, draw a visible focus
//! ring, and report their role/value to screen readers via AccessKit.

use super::Look;
use super::theme::Palette;
use crate::platform::fonts::{ICONS, SEMIBOLD};
use egui::{
    Align2, Color32, CornerRadius, EventFilter, FontFamily, FontId, Key, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

/// Segoe Fluent Icons / Segoe MDL2 Assets code points.
pub mod glyph {
    pub const MUTE: char = '\u{E74F}';
    pub const VOL0: char = '\u{E992}';
    pub const VOL1: char = '\u{E993}';
    pub const VOL2: char = '\u{E994}';
    pub const VOL3: char = '\u{E995}';
    pub const MIC: char = '\u{E720}';
    pub const MIC_OFF: char = '\u{EC54}';
    pub const SETTINGS: char = '\u{E713}';
    pub const REFRESH: char = '\u{E72C}';
    pub const SEARCH: char = '\u{E721}';
    pub const BACK: char = '\u{E72B}';
    pub const SPEAKER: char = '\u{E7F5}';
    pub const CHEVRON_DOWN: char = '\u{E70D}';
    pub const CHEVRON_UP: char = '\u{E70E}';
    pub const CLOSE: char = '\u{E711}';
    pub const INFO: char = '\u{E946}';
    pub const WARNING: char = '\u{E7BA}';
    pub const CHECK: char = '\u{E73E}';

    pub fn volume(volume: f32, muted: bool) -> char {
        if muted {
            MUTE
        } else if volume <= 0.001 {
            VOL0
        } else if volume < 0.34 {
            VOL1
        } else if volume < 0.67 {
            VOL2
        } else {
            VOL3
        }
    }
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn focus_ring(ui: &Ui, rect: Rect, response: &Response, p: &Palette, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(radius),
            Stroke::new(2.0, p.focus),
            StrokeKind::Outside,
        );
    }
}

/// A slider over `min..=max`, filled from `origin` (0 for volume, centre for balance).
pub struct Slider<'a> {
    pub value: &'a mut f32,
    pub min: f32,
    pub max: f32,
    pub origin: f32,
    /// Accessible name, e.g. "Spotify volume".
    pub label: &'a str,
    /// Accessible value scale (100 for percent).
    pub a11y_scale: f64,
    pub muted: bool,
    pub meter: Option<f32>,
    pub width: f32,
}

impl<'a> Slider<'a> {
    pub fn volume(value: &'a mut f32, label: &'a str, width: f32) -> Self {
        Self { value, min: 0.0, max: 1.0, origin: 0.0, label, a11y_scale: 100.0, muted: false, meter: None, width }
    }

    pub fn balance(value: &'a mut f32, label: &'a str, width: f32) -> Self {
        Self { value, min: -1.0, max: 1.0, origin: 0.0, label, a11y_scale: 100.0, muted: false, meter: None, width }
    }

    pub fn show(self, ui: &mut Ui, p: &Palette) -> Response {
        let height = 26.0;
        let (rect, mut response) = ui.allocate_exact_size(vec2(self.width.max(60.0), height), Sense::click_and_drag());
        let enabled = ui.is_enabled();
        let span = self.max - self.min;
        let small = span / 100.0;
        let large = span / 10.0;
        let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 16.0, 4.0));
        let to_x = |v: f32| track.left() + ((v - self.min) / span).clamp(0.0, 1.0) * track.width();

        let old = *self.value;
        let mut new = old;
        if enabled {
            if let Some(pos) = response.interact_pointer_pos()
                && (response.is_pointer_button_down_on() || response.dragged())
            {
                new = self.min + ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) * span;
                if !response.has_focus() {
                    response.request_focus();
                }
            }
            if response.has_focus() {
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        response.id,
                        EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() },
                    )
                });
                ui.input(|i| {
                    let step = if i.modifiers.shift { large / 2.0 } else { small };
                    if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowDown) {
                        new = snap(new - step, small);
                    }
                    if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowUp) {
                        new = snap(new + step, small);
                    }
                    if i.key_pressed(Key::PageDown) {
                        new = snap(new - large, small);
                    }
                    if i.key_pressed(Key::PageUp) {
                        new = snap(new + large, small);
                    }
                    if i.key_pressed(Key::Home) {
                        new = self.min;
                    }
                    if i.key_pressed(Key::End) {
                        new = self.max;
                    }
                });
            }
            // Screen-reader actions (UI Automation RangeValue), in the reported scale.
            ui.input(|i| {
                use egui::accesskit::{Action, ActionData};
                let inc = i.num_accesskit_action_requests(response.id, Action::Increment) as f32;
                let dec = i.num_accesskit_action_requests(response.id, Action::Decrement) as f32;
                if inc != dec {
                    new = snap(new + (inc - dec) * small, small);
                }
                for req in i.accesskit_action_requests(response.id, Action::SetValue) {
                    if let Some(ActionData::NumericValue(v)) = req.data {
                        new = (v / self.a11y_scale) as f32;
                    }
                }
            });
            if response.hovered() {
                // One wheel notch = 2 steps. The list must not scroll while adjusting.
                let notches: f32 = ui.input(|i| {
                    i.raw
                        .events
                        .iter()
                        .map(|e| match e {
                            egui::Event::MouseWheel { unit, delta, .. } => match unit {
                                egui::MouseWheelUnit::Line => delta.y,
                                egui::MouseWheelUnit::Point => delta.y / 50.0,
                                egui::MouseWheelUnit::Page => delta.y * 5.0,
                            },
                            _ => 0.0,
                        })
                        .sum()
                });
                if notches != 0.0 {
                    new = snap(new + notches * small * 2.0, small);
                }
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }
        new = new.clamp(self.min, self.max);
        if (new - old).abs() > f32::EPSILON {
            *self.value = new;
            response.mark_changed();
        }

        // Paint.
        let painter = ui.painter();
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let fill = if self.muted { p.text_muted.gamma_multiply(0.6) } else { p.accent };
        painter.rect_filled(track, CornerRadius::same(2), fade(p.track));
        let (a, b) = {
            let (x0, x1) = (to_x(self.origin), to_x(*self.value));
            (x0.min(x1), x0.max(x1))
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(a, track.top()), pos2(b, track.bottom())),
            CornerRadius::same(2),
            fade(fill),
        );
        if let Some(level) = self.meter.filter(|l| *l > 0.003 && !self.muted) {
            let end = (track.left() + level.clamp(0.0, 1.0) * track.width()).min(to_x(*self.value));
            let meter = Rect::from_min_max(pos2(track.left(), track.bottom() + 3.0), pos2(end, track.bottom() + 5.0));
            painter.rect_filled(meter, CornerRadius::same(1), fade(p.accent.gamma_multiply(0.55)));
        }
        let hot = ui
            .ctx()
            .animate_bool(response.id.with("hot"), response.hovered() || response.dragged() || response.has_focus());
        let center = pos2(to_x(*self.value), track.center().y);
        let radius = 7.0 + 1.5 * hot;
        let thumb = if p.dark { Color32::from_rgb(0xf4, 0xf4, 0xf8) } else { Color32::WHITE };
        painter.circle_filled(
            center + vec2(0.0, 1.0),
            radius + 0.5,
            Color32::from_black_alpha(if p.dark { 90 } else { 35 }),
        );
        painter.circle_filled(center, radius, fade(thumb));
        painter.circle_filled(center, 3.0 + hot, fade(fill));
        focus_ring(ui, rect, &response, p, 8);

        let value = *self.value as f64 * self.a11y_scale;
        let label = self.label.to_string();
        response.widget_info(|| WidgetInfo::slider(enabled, value.round(), &label));
        let (min, max, step) =
            (self.min as f64 * self.a11y_scale, self.max as f64 * self.a11y_scale, small as f64 * self.a11y_scale);
        ui.ctx().accesskit_node_builder(response.id, |b| {
            use egui::accesskit::Action;
            b.set_min_numeric_value(min);
            b.set_max_numeric_value(max);
            b.set_numeric_value_step(step);
            if enabled {
                b.add_action(Action::SetValue);
                b.add_action(Action::Increment);
                b.add_action(Action::Decrement);
            }
        });
        response
    }
}

fn snap(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

/// An unavailable slider has no numeric value, thumb or live meter.
pub fn unavailable_slider(ui: &mut Ui, look: &Look, label: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().max(60.0), 26.0), Sense::hover());
    let track = Rect::from_center_size(rect.center(), vec2((rect.width() - 16.0).max(10.0), 4.0));
    ui.painter().rect_filled(track, CornerRadius::same(2), look.p.text_muted.gamma_multiply(0.35));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Slider, false, label));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_disabled();
    });
    response.on_disabled_hover_text("Not running — volume and level meter unavailable");
}

/// A square icon-only button with a tooltip and an accessible name.
pub fn icon_button(
    ui: &mut Ui,
    look: &Look,
    glyph: char,
    fallback: &str,
    label: &str,
    emphasized: Option<Color32>,
) -> Response {
    let p = &look.p;
    let size = look.m.button;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let enabled = ui.is_enabled();
    let bg = if response.is_pointer_button_down_on() {
        p.surface_hover.gamma_multiply(1.4)
    } else if response.hovered() || response.has_focus() {
        p.surface_hover
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), bg);
    let color = emphasized.unwrap_or(p.text);
    let color = if enabled { color } else { color.gamma_multiply(0.4) };
    if look.icons {
        painter.text(rect.center(), Align2::CENTER_CENTER, glyph, icon_font(size * 0.48), color);
    } else {
        painter.text(rect.center(), Align2::CENTER_CENTER, fallback, FontId::proportional(11.0), color);
    }
    focus_ring(ui, rect, &response, p, 6);
    let label_owned = label.to_string();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &label_owned));
    response.on_hover_text(label)
}

/// A full-width settings row with a label (and optional description) and a switch.
pub fn toggle_row(ui: &mut Ui, look: &Look, on: &mut bool, label: &str, description: Option<&str>) -> Response {
    let p = &look.p;
    let width = ui.available_width();
    // Text column leaves room for the switch; descriptions wrap instead of running under it.
    let text_w = (width - 64.0).max(80.0);
    let desc_galley = description.map(|d| {
        let mut job = egui::text::LayoutJob::simple(d.to_string(), FontId::proportional(12.0), p.text_muted, text_w);
        job.halign = egui::Align::LEFT;
        ui.painter().layout_job(job)
    });
    let height = match &desc_galley {
        Some(g) => 26.0 + g.size().y + 6.0,
        None => 32.0,
    };
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let enabled = ui.is_enabled();
    if response.clicked() && enabled {
        *on = !*on;
        response.mark_changed();
    }
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect.expand2(vec2(4.0, 0.0)), CornerRadius::same(6), p.surface_hover);
    }
    let text = if enabled { p.text } else { p.text_muted };
    let left = rect.left() + 2.0;
    if let Some(galley) = desc_galley {
        painter.text(pos2(left, rect.top() + 5.0), Align2::LEFT_TOP, label, FontId::proportional(14.0), text);
        painter.galley(pos2(left, rect.top() + 25.0), galley, p.text_muted);
    } else {
        painter.text(pos2(left, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), text);
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let track = Rect::from_center_size(pos2(rect.right() - 22.0, rect.center().y), vec2(38.0, 20.0));
    let on_fill = if enabled { p.accent } else { p.accent.gamma_multiply(0.4) };
    if *on {
        painter.rect_filled(track, CornerRadius::same(10), on_fill);
    } else {
        painter.rect_stroke(track, CornerRadius::same(10), Stroke::new(1.5, p.text_muted), StrokeKind::Inside);
    }
    let knob_x = egui::lerp(track.left() + 10.0..=track.right() - 10.0, t);
    let knob = if *on { p.on_accent } else { p.text_muted };
    painter.circle_filled(pos2(knob_x, track.center().y), if *on { 6.0 } else { 5.0 }, knob);
    focus_ring(ui, rect, &response, p, 6);
    let (label_owned, selected) = (label.to_string(), *on);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, selected, &label_owned));
    response
}

/// A segmented control; returns `true` when the value changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    look: &Look,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    let p = &look.p;
    let height = 30.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let enabled = ui.is_enabled();
    ui.painter().rect_filled(rect, CornerRadius::same(8), p.surface);
    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, p.border), StrokeKind::Inside);
    let seg_w = width / options.len() as f32;
    let mut changed = false;
    for (i, (opt, label)) in options.iter().enumerate() {
        let seg =
            Rect::from_min_size(pos2(rect.left() + seg_w * i as f32, rect.top()), vec2(seg_w, height)).shrink(3.0);
        let id = ui.id().with(("segment", label));
        let response = ui.interact(seg, id, Sense::click());
        let selected = *value == *opt;
        if response.clicked() && enabled && !selected {
            *value = *opt;
            changed = true;
        }
        let painter = ui.painter();
        if selected {
            painter.rect_filled(seg, CornerRadius::same(6), p.accent_soft);
            painter.rect_stroke(seg, CornerRadius::same(6), Stroke::new(1.0, p.accent), StrokeKind::Inside);
        } else if response.hovered() {
            painter.rect_filled(seg, CornerRadius::same(6), p.surface_hover);
        }
        let color = if !enabled {
            p.text_muted.gamma_multiply(0.6)
        } else if selected {
            p.text
        } else {
            p.text_muted
        };
        let font = if selected { semibold(13.0) } else { FontId::proportional(13.0) };
        painter.text(seg.center(), Align2::CENTER_CENTER, *label, font, color);
        focus_ring(ui, seg, &response, p, 6);
        let label_owned = label.to_string();
        response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, &label_owned));
    }
    changed
}

/// Generic app tile used when no icon could be retrieved: a rounded square with
/// the app's initial, coloured consistently from its name.
pub fn fallback_tile(ui: &Ui, rect: Rect, name: &str, look: &Look, glyph_char: Option<char>) {
    let hue = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360;
    let base = egui::ecolor::Hsva::new(
        hue as f32 / 360.0,
        if look.p.dark { 0.45 } else { 0.55 },
        if look.p.dark { 0.62 } else { 0.58 },
        1.0,
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same((rect.width() * 0.24) as u8), Color32::from(base));
    match glyph_char {
        Some(g) if look.icons => {
            painter.text(rect.center(), Align2::CENTER_CENTER, g, icon_font(rect.width() * 0.52), Color32::WHITE);
        }
        _ => {
            let initial: String = name
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_else(|| "?".into());
            painter.text(rect.center(), Align2::CENTER_CENTER, initial, semibold(rect.width() * 0.5), Color32::WHITE);
        }
    }
}

/// A rounded surface used to group related controls.
pub fn card(look: &Look) -> egui::Frame {
    egui::Frame::NONE
        .fill(look.p.surface)
        .stroke(Stroke::new(1.0, look.p.border))
        .corner_radius(CornerRadius::same(look.m.radius))
        .inner_margin(egui::Margin::same((look.m.pad * 0.85) as i8))
}

/// Right-aligned fixed-width percentage label.
pub fn percent_label(ui: &mut Ui, look: &Look, text: &str, dim: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(40.0, look.m.button), Sense::hover());
    let color = if dim { look.p.text_muted } else { look.p.text };
    ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, text, semibold(13.5), color);
}
