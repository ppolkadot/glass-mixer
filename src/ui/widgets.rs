//! Custom widgets, SF-Symbol-proportioned vector icons, slider, toggle
//! switch, segmented control and app tiles. All interactive controls are
//! keyboard-focusable, draw a visible focus ring, and report their role/value
//! to screen readers via AccessKit.

use super::Look;
use super::theme::{self, Palette};
use crate::platform::fonts::{ICONS, SEMIBOLD};
use egui::{
    Align2, Color32, CornerRadius, EventFilter, FontFamily, FontId, Key, Pos2, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, epaint::PathShape, epaint::PathStroke, pos2, vec2,
};

/// Segoe Fluent Icons / Segoe MDL2 Assets code points (kept for font-glyph fallback).
pub mod glyph {
    pub const MUTE: char = '\u{E74F}';
    pub const VOL0: char = '\u{E992}';
    pub const VOL1: char = '\u{E993}';
    pub const VOL2: char = '\u{E994}';
    pub const VOL3: char = '\u{E995}';
    pub const MIC: char = '\u{E720}';
    pub const MIC_OFF: char = '\u{EC54}';
    pub const REFRESH: char = '\u{E72C}';
    pub const BACK: char = '\u{E72B}';
    pub const SPEAKER: char = '\u{E7F5}';
    pub const WARNING: char = '\u{E7BA}';
    pub const CHECK: char = '\u{E73E}';
    pub const SOLO: char = '\u{E7F6}';
}

/// Procedural SF-Symbol-proportioned vector icons drawn directly via `Painter`
/// so optical anchors, stroke weights, and caps stay consistent across DPIs
/// and never shift or depend on system icon fonts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Symbol {
    SpeakerWave(u8),
    SpeakerSlash,
    Mic,
    MicSlash,
    Headphones { filled: bool },
    SlidersHorizontal,
    SliderVertical3,
    Gear,
    ArrowClockwise,
    MagnifyingGlass,
    XMark,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronDown,
    Checkmark,
    ExclamationTriangleFill,
    Display,
    HifiSpeaker,
    SystemSounds,
}

impl Symbol {
    pub fn volume(volume: f32, muted: bool) -> Self {
        if muted {
            Self::SpeakerSlash
        } else if volume <= 0.001 {
            Self::SpeakerWave(0)
        } else if volume < 0.34 {
            Self::SpeakerWave(1)
        } else if volume < 0.67 {
            Self::SpeakerWave(2)
        } else {
            Self::SpeakerWave(3)
        }
    }
}

pub fn device_symbol(flow: crate::audio::Flow, name: &str) -> Symbol {
    if flow == crate::audio::Flow::Input {
        return Symbol::Mic;
    }
    let lower = name.to_ascii_lowercase();
    if lower.contains("headphone") || lower.contains("headset") || lower.contains("earbud") || lower.contains("airpods")
    {
        Symbol::Headphones { filled: true }
    } else if lower.contains("hdmi") || lower.contains("display") || lower.contains("monitor") || lower.contains("tv") {
        Symbol::Display
    } else {
        Symbol::HifiSpeaker
    }
}

fn round_seg(painter: &egui::Painter, a: Pos2, b: Pos2, w: f32, color: Color32) {
    painter.line_segment([a, b], Stroke::new(w, color));
    painter.circle_filled(a, w * 0.5, color);
    painter.circle_filled(b, w * 0.5, color);
}

fn paint_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    angle: std::ops::RangeInclusive<f32>,
    steps: usize,
    stroke: Stroke,
) {
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = egui::lerp(angle.clone(), t);
        pts.push(pos2(center.x + radius * a.cos(), center.y + radius * a.sin()));
    }
    if let (Some(&first), Some(&last)) = (pts.first(), pts.last()) {
        painter.line(pts, PathStroke::new(stroke.width, stroke.color));
        painter.circle_filled(first, stroke.width * 0.5, stroke.color);
        painter.circle_filled(last, stroke.width * 0.5, stroke.color);
    }
}

fn paint_speaker_body(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    // Fixed optical x-anchor so the cone never jumps horizontally between states.
    let x0 = c.x - s * 0.14;
    let body = Rect::from_min_max(pos2(x0 - s * 0.22, c.y - s * 0.14), pos2(x0 - s * 0.05, c.y + s * 0.14));
    painter.rect_filled(body, CornerRadius::same(1), color);
    let cone = vec![
        pos2(x0 - s * 0.08, c.y - s * 0.14),
        pos2(x0 + s * 0.10, c.y - s * 0.30),
        pos2(x0 + s * 0.10, c.y + s * 0.30),
        pos2(x0 - s * 0.08, c.y + s * 0.14),
    ];
    painter.add(PathShape::convex_polygon(cone, color, PathStroke::NONE));
}

pub fn paint_symbol(painter: &egui::Painter, rect: Rect, sym: Symbol, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let w = (s * 0.11).clamp(1.3, 2.2);
    match sym {
        Symbol::SpeakerWave(n) => {
            paint_speaker_body(painter, c, s, color);
            let origin = pos2(c.x - s * 0.02, c.y);
            let radii = [s * 0.15, s * 0.27, s * 0.39];
            for r in radii.into_iter().take(n.min(3) as usize) {
                paint_arc(painter, origin, r, -0.66..=0.66, 8, Stroke::new(w * 0.92, color));
            }
        }
        Symbol::SpeakerSlash => {
            paint_speaker_body(painter, c, s, color.gamma_multiply(0.75));
            round_seg(
                painter,
                pos2(c.x - s * 0.32, c.y - s * 0.32),
                pos2(c.x + s * 0.32, c.y + s * 0.32),
                w * 1.05,
                color,
            );
        }
        Symbol::Mic | Symbol::MicSlash => {
            let cap_color = if matches!(sym, Symbol::MicSlash) { color.gamma_multiply(0.75) } else { color };
            let cap = Rect::from_center_size(pos2(c.x, c.y - s * 0.10), vec2(s * 0.26, s * 0.44));
            painter.rect_filled(cap, CornerRadius::same(((s * 0.13).round() as u8).max(2)), cap_color);
            paint_arc(
                painter,
                pos2(c.x, c.y - s * 0.04),
                s * 0.23,
                0.0..=std::f32::consts::PI,
                10,
                Stroke::new(w * 0.92, cap_color),
            );
            round_seg(painter, pos2(c.x, c.y + s * 0.19), pos2(c.x, c.y + s * 0.34), w * 0.92, cap_color);
            round_seg(
                painter,
                pos2(c.x - s * 0.14, c.y + s * 0.34),
                pos2(c.x + s * 0.14, c.y + s * 0.34),
                w * 0.92,
                cap_color,
            );
            if matches!(sym, Symbol::MicSlash) {
                round_seg(
                    painter,
                    pos2(c.x - s * 0.32, c.y - s * 0.32),
                    pos2(c.x + s * 0.32, c.y + s * 0.32),
                    w * 1.05,
                    color,
                );
            }
        }
        Symbol::Headphones { filled } => {
            paint_arc(
                painter,
                pos2(c.x, c.y + s * 0.04),
                s * 0.29,
                std::f32::consts::PI..=std::f32::consts::TAU,
                14,
                Stroke::new(w * 0.95, color),
            );
            for dir in [-1.0_f32, 1.0_f32] {
                let cup = Rect::from_center_size(pos2(c.x + dir * s * 0.26, c.y + s * 0.13), vec2(s * 0.14, s * 0.28));
                let r = CornerRadius::same(((s * 0.06).round() as u8).max(2));
                if filled {
                    painter.rect_filled(cup, r, color);
                } else {
                    painter.rect_stroke(cup, r, Stroke::new(w * 0.9, color), StrokeKind::Middle);
                }
            }
        }
        Symbol::SlidersHorizontal => {
            for (dy, kx) in [(-0.24_f32, -0.10_f32), (0.0, 0.14), (0.24, -0.16)] {
                let y = c.y + dy * s;
                round_seg(painter, pos2(c.x - s * 0.34, y), pos2(c.x + s * 0.34, y), w * 0.85, color);
                painter.circle_filled(pos2(c.x + kx * s, y), (s * 0.095).max(2.0), color);
            }
        }
        Symbol::ArrowClockwise => {
            let r = s * 0.28;
            paint_arc(painter, c, r, -0.30..=(std::f32::consts::TAU - 1.15), 16, Stroke::new(w * 0.95, color));
            let tip = pos2(c.x + r * (-0.30_f32).cos(), c.y + r * (-0.30_f32).sin());
            let head =
                [tip + vec2(s * 0.03, s * 0.03), tip + vec2(-s * 0.15, s * 0.01), tip + vec2(-s * 0.02, -s * 0.14)];
            painter.add(PathShape::convex_polygon(head.into(), color, PathStroke::NONE));
        }
        Symbol::MagnifyingGlass => {
            let center = pos2(c.x - s * 0.06, c.y - s * 0.06);
            painter.circle_stroke(center, s * 0.22, Stroke::new(w * 0.95, color));
            round_seg(
                painter,
                pos2(center.x + s * 0.16, center.y + s * 0.16),
                pos2(c.x + s * 0.30, c.y + s * 0.30),
                w,
                color,
            );
        }
        Symbol::XMark => {
            let d = s * 0.24;
            round_seg(painter, pos2(c.x - d, c.y - d), pos2(c.x + d, c.y + d), w, color);
            round_seg(painter, pos2(c.x + d, c.y - d), pos2(c.x - d, c.y + d), w, color);
        }
        Symbol::ChevronLeft => {
            round_seg(painter, pos2(c.x + s * 0.12, c.y - s * 0.24), pos2(c.x - s * 0.14, c.y), w, color);
            round_seg(painter, pos2(c.x - s * 0.14, c.y), pos2(c.x + s * 0.12, c.y + s * 0.24), w, color);
        }
        Symbol::ChevronRight => {
            round_seg(painter, pos2(c.x - s * 0.12, c.y - s * 0.24), pos2(c.x + s * 0.14, c.y), w, color);
            round_seg(painter, pos2(c.x + s * 0.14, c.y), pos2(c.x - s * 0.12, c.y + s * 0.24), w, color);
        }
        Symbol::ChevronUp => {
            round_seg(painter, pos2(c.x - s * 0.24, c.y + s * 0.12), pos2(c.x, c.y - s * 0.14), w, color);
            round_seg(painter, pos2(c.x, c.y - s * 0.14), pos2(c.x + s * 0.24, c.y + s * 0.12), w, color);
        }
        Symbol::ChevronDown => {
            round_seg(painter, pos2(c.x - s * 0.24, c.y - s * 0.12), pos2(c.x, c.y + s * 0.14), w, color);
            round_seg(painter, pos2(c.x, c.y + s * 0.14), pos2(c.x + s * 0.24, c.y - s * 0.12), w, color);
        }
        Symbol::Checkmark => {
            let p0 = pos2(c.x - s * 0.24, c.y + s * 0.01);
            let p1 = pos2(c.x - s * 0.05, c.y + s * 0.20);
            let p2 = pos2(c.x + s * 0.26, c.y - s * 0.17);
            round_seg(painter, p0, p1, w * 1.05, color);
            round_seg(painter, p1, p2, w * 1.05, color);
        }
        Symbol::SliderVertical3 => {
            for (dx, ky) in [(-0.24_f32, -0.10_f32), (0.0, 0.14), (0.24, -0.16)] {
                let x = c.x + dx * s;
                round_seg(painter, pos2(x, c.y - s * 0.32), pos2(x, c.y + s * 0.32), w * 0.85, color);
                painter.circle_filled(pos2(x, c.y + ky * s), (s * 0.095).max(2.0), color);
            }
        }
        Symbol::Gear => {
            let r1 = s * 0.20;
            let r2 = s * 0.33;
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (sin_a, cos_a) = a.sin_cos();
                round_seg(
                    painter,
                    pos2(c.x + r1 * cos_a, c.y + r1 * sin_a),
                    pos2(c.x + r2 * cos_a, c.y + r2 * sin_a),
                    w * 1.15,
                    color,
                );
            }
            painter.circle_stroke(c, s * 0.21, Stroke::new(w * 1.1, color));
            painter.circle_stroke(c, s * 0.08, Stroke::new(w * 0.85, color));
        }
        Symbol::ExclamationTriangleFill => {
            let tri = vec![
                pos2(c.x, c.y - s * 0.34),
                pos2(c.x + s * 0.36, c.y + s * 0.28),
                pos2(c.x - s * 0.36, c.y + s * 0.28),
            ];
            painter.add(PathShape::convex_polygon(tri, color.gamma_multiply(0.24), PathStroke::new(w, color)));
            round_seg(painter, pos2(c.x, c.y - s * 0.12), pos2(c.x, c.y + s * 0.06), w, color);
            painter.circle_filled(pos2(c.x, c.y + s * 0.18), w * 0.65, color);
        }
        Symbol::Display => {
            let screen = Rect::from_center_size(pos2(c.x, c.y - s * 0.06), vec2(s * 0.68, s * 0.46));
            painter.rect_stroke(screen, CornerRadius::same(2), Stroke::new(w * 0.9, color), StrokeKind::Middle);
            round_seg(painter, pos2(c.x, c.y + s * 0.17), pos2(c.x, c.y + s * 0.31), w * 0.9, color);
            round_seg(
                painter,
                pos2(c.x - s * 0.18, c.y + s * 0.31),
                pos2(c.x + s * 0.18, c.y + s * 0.31),
                w * 0.9,
                color,
            );
        }
        Symbol::HifiSpeaker => {
            let box_rect = Rect::from_center_size(c, vec2(s * 0.48, s * 0.68));
            painter.rect_stroke(box_rect, CornerRadius::same(3), Stroke::new(w * 0.9, color), StrokeKind::Middle);
            painter.circle_filled(pos2(c.x, c.y - s * 0.16), (s * 0.065).max(1.5), color);
            painter.circle_stroke(pos2(c.x, c.y + s * 0.10), s * 0.12, Stroke::new(w * 0.85, color));
        }
        Symbol::SystemSounds => {
            let bars = [(-0.28_f32, 0.28_f32), (-0.14, 0.54), (0.0, 0.78), (0.14, 0.46), (0.28, 0.28)];
            let bw = (s * 0.085).max(1.5);
            for (dx, h) in bars {
                let r = Rect::from_center_size(pos2(c.x + dx * s, c.y), vec2(bw, h * s));
                painter.rect_filled(r, CornerRadius::same((bw * 0.5).ceil() as u8), color);
            }
        }
    }
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn count_label(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 { format!("{n} {singular}") } else { format!("{n} {plural}") }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        egui::lerp(f32::from(a.r())..=f32::from(b.r()), t).round() as u8,
        egui::lerp(f32::from(a.g())..=f32::from(b.g()), t).round() as u8,
        egui::lerp(f32::from(a.b())..=f32::from(b.b()), t).round() as u8,
        egui::lerp(f32::from(a.a())..=f32::from(b.a()), t).round() as u8,
    )
}

pub fn scroll_on_focus(response: Response) -> Response {
    if response.gained_focus() {
        response.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
    }
    response
}

pub fn focus_ring(ui: &Ui, rect: Rect, response: &Response, p: &Palette, radius: u8) {
    if response.gained_focus() {
        response.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
    }
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
    /// Value change per mouse-wheel notch.
    pub wheel_step: f32,
}

impl<'a> Slider<'a> {
    pub fn volume(value: &'a mut f32, label: &'a str, width: f32) -> Self {
        Self {
            value,
            min: 0.0,
            max: 1.0,
            origin: 0.0,
            label,
            a11y_scale: 100.0,
            muted: false,
            meter: None,
            width,
            wheel_step: 0.02,
        }
    }

    pub fn balance(value: &'a mut f32, label: &'a str, width: f32) -> Self {
        Self {
            value,
            min: -1.0,
            max: 1.0,
            origin: 0.0,
            label,
            a11y_scale: 100.0,
            muted: false,
            meter: None,
            width,
            wheel_step: 0.04,
        }
    }

    /// Sets the wheel step from a percentage (the "Mouse wheel step" setting).
    pub fn wheel_percent(self, percent: u8) -> Self {
        Self { wheel_step: f32::from(percent.clamp(1, 20)) / 100.0, ..self }
    }

    pub fn show(self, ui: &mut Ui, p: &Palette) -> Response {
        let height = 26.0;
        let (rect, mut response) = ui.allocate_exact_size(vec2(self.width.max(60.0), height), Sense::click_and_drag());
        let enabled = ui.is_enabled();
        let span = self.max - self.min;
        let small = span / 100.0;
        let large = span / 10.0;
        let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 16.0, 6.0));
        let to_x = |v: f32| track.left() + ((v - self.min) / span).clamp(0.0, 1.0) * track.width();

        let old = *self.value;
        let mut new = old;
        if enabled {
            if let Some(pos) = response.interact_pointer_pos()
                && (response.is_pointer_button_down_on() || response.dragged())
            {
                new = self.min + ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) * span;
                if self.min < 0.0 && new.abs() < 0.03 {
                    new = 0.0;
                }
                if !response.has_focus() {
                    response.request_focus();
                }
            }
            if self.min == 0.0 && response.double_clicked() {
                new = self.max;
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
                // The list must not scroll while adjusting.
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
                    new = snap(new + notches * self.wheel_step, small);
                }
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }
        new = new.clamp(self.min, self.max);
        if (new - old).abs() > f32::EPSILON {
            *self.value = new;
            response.mark_changed();
        }

        let painter = ui.painter();
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let fill = if self.muted { p.text_muted.gamma_multiply(0.6) } else { p.accent };
        painter.rect_filled(track, CornerRadius::same(3), fade(p.track));
        let (a, b) = {
            let (x0, x1) = (to_x(self.origin), to_x(*self.value));
            (x0.min(x1), x0.max(x1))
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(a, track.top()), pos2(b, track.bottom())),
            CornerRadius::same(3),
            fade(fill),
        );
        if self.min < 0.0 {
            let cx = to_x(0.0);
            painter.vline(cx, (track.top() - 2.0)..=(track.bottom() + 2.0), Stroke::new(1.0, fade(p.text_muted)));
        }
        if let Some(level) = self.meter.filter(|l| *l > 0.003 && !self.muted) {
            let end = (track.left() + level.clamp(0.0, 1.0) * track.width()).min(to_x(*self.value));
            if p.high_contrast {
                let meter =
                    Rect::from_min_max(pos2(track.left(), track.bottom() + 2.0), pos2(end, track.bottom() + 4.0));
                painter.rect_filled(meter, CornerRadius::same(1), fade(p.accent));
            } else if end > track.left() {
                let meter = Rect::from_min_max(pos2(track.left(), track.top()), pos2(end, track.bottom()));
                let glow = Color32::from_white_alpha(if p.dark { 70 } else { 110 });
                painter.rect_filled(meter, CornerRadius::same(3), fade(glow));
            }
        }
        let hot = ui.ctx().animate_bool_with_time_and_easing(
            response.id.with("hot"),
            response.hovered() || response.dragged() || response.has_focus(),
            ui.style().animation_time,
            theme::ease,
        );
        let center = pos2(to_x(*self.value), track.center().y);
        let radius = 8.0 + hot;
        let thumb = if p.dark { Color32::from_rgb(0xf4, 0xf4, 0xf8) } else { Color32::WHITE };
        if !p.high_contrast {
            painter.circle_filled(
                center + vec2(0.0, 1.0),
                radius + 0.5,
                Color32::from_black_alpha(if p.dark { 70 } else { 28 }),
            );
        }
        painter.circle_filled(center, radius, fade(thumb));
        if !p.dark || p.high_contrast {
            painter.circle_stroke(center, radius, Stroke::new(1.0, fade(p.border)));
        }
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

/// Sets the accessible name of a widget that AccessKit would otherwise leave unnamed
/// (combo boxes, text fields), or renames one whose visible text is too short to be
/// meaningful to a screen reader.
pub fn set_name(ui: &Ui, response: &Response, name: &str) {
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_label(name));
}

/// An unavailable slider has no numeric value, thumb or live meter.
pub fn unavailable_slider(ui: &mut Ui, look: &Look, label: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().max(60.0), 26.0), Sense::hover());
    let track = Rect::from_center_size(rect.center(), vec2((rect.width() - 16.0).max(10.0), 6.0));
    ui.painter().rect_filled(track, CornerRadius::same(3), look.p.text_muted.gamma_multiply(0.35));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Slider, false, label));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_disabled();
    });
    response.on_disabled_hover_text("Not running. Volume and level meter unavailable.");
}

/// A square icon-only button with a tooltip and an accessible name.
pub fn icon_button(ui: &mut Ui, look: &Look, symbol: Symbol, label: &str, emphasized: Option<Color32>) -> Response {
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
    painter.rect_filled(rect, CornerRadius::same(look.m.radius_inner), bg);
    let color = emphasized.unwrap_or(p.text);
    let color = if enabled { color } else { color.gamma_multiply(0.4) };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size * 0.56));
    paint_symbol(painter, icon_rect, symbol, color);
    focus_ring(ui, rect, &response, p, look.m.radius_inner);
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
        Some(g) => 24.0 + g.size().y + 8.0,
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
        painter.rect_filled(rect.expand2(vec2(4.0, 0.0)), CornerRadius::same(look.m.radius_inner), p.surface_hover);
    }
    let text = if enabled { p.text } else { p.text_muted };
    let left = rect.left() + 2.0;
    if let Some(galley) = desc_galley {
        painter.text(pos2(left, rect.top() + 4.0), Align2::LEFT_TOP, label, FontId::proportional(14.0), text);
        painter.galley(pos2(left, rect.top() + 24.0), galley, p.text_muted);
    } else {
        painter.text(pos2(left, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), text);
    }
    let t = ui.ctx().animate_bool_with_time_and_easing(response.id, *on, ui.style().animation_time, theme::ease);
    let track = Rect::from_center_size(pos2(rect.right() - 24.0, rect.center().y), vec2(44.0, 24.0));
    let track_r = CornerRadius::same(12);
    let on_fill = if enabled { p.accent } else { p.accent.gamma_multiply(0.4) };
    let off_fill = if p.high_contrast {
        Color32::BLACK
    } else if enabled {
        p.switch_off
    } else {
        p.switch_off.gamma_multiply(0.5)
    };
    if p.high_contrast {
        painter.rect_filled(track, track_r, lerp_color(off_fill, on_fill, t));
        if !*on {
            painter.rect_stroke(track, track_r, Stroke::new(1.5, p.border), StrokeKind::Inside);
        }
    } else {
        if t > 0.05 {
            let glow_alpha = ((if p.dark { 72.0 } else { 44.0 }) * t).round() as u8;
            let track_shadow = egui::Shadow {
                offset: [0, 2],
                blur: 6,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), glow_alpha),
            };
            painter.add(track_shadow.as_shape(track, track_r));
        }
        if p.dark {
            painter.rect_filled(track, track_r, Color32::from_black_alpha(((1.0 - t) * 56.0).round() as u8));
        }
        painter.rect_filled(track, track_r, lerp_color(off_fill, on_fill, t));
        let sheen = Rect::from_min_max(
            pos2(track.left() + 2.0, track.top() + 1.5),
            pos2(track.right() - 2.0, track.center().y),
        );
        let sheen_alpha = egui::lerp(if p.dark { 18.0..=48.0 } else { 32.0..=64.0 }, t).round() as u8;
        painter.rect_filled(sheen, CornerRadius::same(10), Color32::from_white_alpha(sheen_alpha));
        let off_rim = if p.dark { Color32::from_white_alpha(64) } else { Color32::from_black_alpha(38) };
        let on_rim = lerp_color(p.accent, Color32::WHITE, if p.dark { 0.34 } else { 0.24 });
        painter.rect_stroke(track, track_r, Stroke::new(1.0, lerp_color(off_rim, on_rim, t)), StrokeKind::Inside);
    }

    let press_t = ui.ctx().animate_bool_with_time_and_easing(
        response.id.with("press"),
        enabled && (response.is_pointer_button_down_on() || response.hovered()),
        0.12,
        theme::ease,
    );
    let in_flight = 4.0 * t * (1.0 - t);
    let stretch = in_flight.max(if response.is_pointer_button_down_on() { 0.75 * press_t } else { 0.22 * press_t });
    let thumb_w = (20.0 + 4.5 * stretch).round();
    let thumb_h = 20.0;
    let min_x = track.left() + 2.0;
    let max_x = track.right() - 2.0 - thumb_w;
    let thumb_x = egui::lerp(min_x..=max_x, t).round();
    let thumb_rect = Rect::from_min_size(pos2(thumb_x, track.center().y - thumb_h * 0.5), vec2(thumb_w, thumb_h));
    let thumb_r = CornerRadius::same(10);
    if p.high_contrast {
        let knob = if *on { p.on_accent } else { Color32::WHITE };
        painter.rect_filled(thumb_rect, thumb_r, knob);
    } else {
        let shadow = egui::Shadow {
            offset: [0, 1],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(if p.dark { 96 } else { 44 }),
        };
        painter.add(shadow.as_shape(thumb_rect, thumb_r));
        let body = if p.dark { Color32::from_rgb(0xfb, 0xfb, 0xff) } else { Color32::WHITE };
        let body = if enabled { body } else { body.gamma_multiply(0.75) };
        painter.rect_filled(thumb_rect, thumb_r, body);
        painter.hline(
            (thumb_rect.left() + 5.0)..=(thumb_rect.right() - 5.0),
            thumb_rect.top() + 1.5,
            Stroke::new(1.0, Color32::from_white_alpha(240)),
        );
        let rim = if p.dark { Color32::from_white_alpha(220) } else { Color32::from_black_alpha(22) };
        painter.rect_stroke(thumb_rect, thumb_r, Stroke::new(1.0, rim), StrokeKind::Inside);
    }
    focus_ring(ui, rect, &response, p, look.m.radius_inner);
    let (label_owned, selected) = (label.to_string(), *on);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, selected, &label_owned));
    // The description is painted, so give assistive technology the same text.
    if let Some(description) = description {
        ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(description));
    }
    response
}

/// A segmented control with a neutral sliding pill; returns `true` when the value changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    look: &Look,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> bool {
    let p = &look.p;
    let height = 30.0;
    let (rect, container) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let enabled = ui.is_enabled();
    let outer_r = CornerRadius::same(look.m.radius_inner);
    let inner_px = look.m.radius_inner.saturating_sub(2).max(4);
    let inner_r = CornerRadius::same(inner_px);
    ui.painter().rect_filled(rect, outer_r, p.surface);
    ui.painter().rect_stroke(rect, outer_r, Stroke::new(1.0, p.border), StrokeKind::Inside);

    let n = options.len().max(1) as f32;
    let seg_w = rect.width() / n;
    let selected_idx = options.iter().position(|(opt, _)| opt == value).unwrap_or(0);
    let anim_idx =
        ui.ctx().animate_value_with_time(container.id.with("seg_pill"), selected_idx as f32, ui.style().animation_time);
    let pill_x = (rect.left() + seg_w * anim_idx + 2.0).round();
    let pill_w = (seg_w - 4.0).round().max(8.0);
    let pill = Rect::from_min_size(pos2(pill_x, rect.top() + 2.0), vec2(pill_w, height - 4.0));
    if !p.high_contrast {
        let shadow = egui::Shadow { offset: [0, 1], blur: 3, spread: 0, color: p.shadow };
        ui.painter().add(shadow.as_shape(pill, inner_r));
    }
    ui.painter().rect_filled(pill, inner_r, p.selection);
    if p.high_contrast {
        ui.painter().rect_stroke(pill, inner_r, Stroke::new(1.0, p.accent), StrokeKind::Inside);
    } else {
        ui.painter().rect_stroke(pill, inner_r, Stroke::new(1.0, p.border), StrokeKind::Inside);
        let inset = f32::from(inner_px);
        if pill.width() > inset * 2.0 + 2.0 {
            ui.painter().hline(
                (pill.left() + inset)..=(pill.right() - inset),
                pill.top() + 0.5,
                Stroke::new(1.0, p.highlight),
            );
        }
    }

    let mut changed = false;
    for (i, (opt, label)) in options.iter().enumerate() {
        let seg =
            Rect::from_min_size(pos2(rect.left() + seg_w * i as f32, rect.top()), vec2(seg_w, height)).shrink(2.0);
        let id = container.id.with(("segment", i));
        let response = ui.interact(seg, id, Sense::click());
        let selected = *value == *opt;
        if response.clicked() && enabled && !selected {
            *value = *opt;
            changed = true;
        }
        let painter = ui.painter();
        if !selected && response.hovered() {
            painter.rect_filled(seg, inner_r, p.surface_hover);
        }
        let color = if !enabled {
            p.text_muted.gamma_multiply(0.6)
        } else if selected {
            p.selection_text
        } else {
            p.text_muted
        };
        painter.text(seg.center(), Align2::CENTER_CENTER, *label, semibold(13.0), color);
        focus_ring(ui, seg, &response, p, inner_px);
        let label_owned = label.to_string();
        response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, &label_owned));
    }
    changed
}

/// Hover-revealed top dock menu bar with macOS Dock icon magnification and
/// `Salted Flutters Polaris Semibold` labels. Returns `true` when the active
/// selection changed.
pub fn top_dock_menu<T: PartialEq + Copy>(
    ui: &mut Ui,
    look: &Look,
    value: &mut T,
    options: &[(T, Symbol, &str)],
    width: f32,
) -> bool {
    let p = &look.p;
    let height = 36.0;
    let (rect, bar_resp) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let enabled = ui.is_enabled();
    let n = options.len().max(1) as f32;
    let seg_w = rect.width() / n;
    let outer_px = look.m.radius_inner;
    let outer_r = CornerRadius::same(outer_px);
    let inner_px = look.m.radius_inner.saturating_sub(2).max(4);
    let inner_r = CornerRadius::same(inner_px);

    let mut changed = false;
    let mut selected_idx = 0;
    let mut responses = Vec::with_capacity(options.len());
    for (i, (opt, _, label)) in options.iter().enumerate() {
        let seg =
            Rect::from_min_size(pos2(rect.left() + seg_w * i as f32, rect.top()), vec2(seg_w, height)).shrink(2.0);
        let response = ui.interact(seg, bar_resp.id.with(("dock_seg", i)), Sense::click());
        let selected = *value == *opt;
        if selected {
            selected_idx = i;
        }
        if response.clicked() && enabled && !selected {
            *value = *opt;
            selected_idx = i;
            changed = true;
        }
        let label_owned = (*label).to_string();
        response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, &label_owned));
        responses.push(response);
    }

    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    let hover_zone = rect.expand2(vec2(20.0, 10.0));
    let pointer_in_zone = pointer_pos.is_some_and(|pt| hover_zone.contains(pt));
    let any_active = pointer_in_zone || responses.iter().any(|r| r.has_focus() || r.hovered());
    let reveal_t = ui.ctx().animate_bool_with_time_and_easing(
        bar_resp.id.with("reveal"),
        any_active,
        ui.style().animation_time,
        theme::ease,
    );

    if reveal_t < 0.99
        && let Some(&(_, active_sym, active_label)) = options.get(selected_idx).or_else(|| options.first())
    {
        let collapsed_alpha = (1.0 - reveal_t).clamp(0.0, 1.0);
        let painter = ui.painter();
        let text_color = (if enabled { p.text } else { p.text_muted }).gamma_multiply(collapsed_alpha);
        let galley = painter.layout_no_wrap(active_label.to_string(), semibold(12.5), text_color);
        let pill_w = (116.0_f32).max(galley.size().x + 42.0).min(rect.width());
        let trigger = Rect::from_center_size(rect.center(), vec2(pill_w, 28.0));
        let trigger_r = CornerRadius::same(14);
        if !p.high_contrast {
            let shadow =
                egui::Shadow { offset: [0, 1], blur: 4, spread: 0, color: p.shadow.gamma_multiply(collapsed_alpha) };
            painter.add(shadow.as_shape(trigger, trigger_r));
        }
        painter.rect_filled(trigger, trigger_r, p.surface.gamma_multiply(collapsed_alpha));
        painter.rect_stroke(
            trigger,
            trigger_r,
            Stroke::new(1.0, p.border.gamma_multiply(collapsed_alpha)),
            StrokeKind::Inside,
        );
        if !p.high_contrast {
            painter.hline(
                (trigger.left() + 14.0)..=(trigger.right() - 14.0),
                trigger.top() + 1.0,
                Stroke::new(1.0, p.highlight.gamma_multiply(collapsed_alpha)),
            );
        }
        let content_w = 14.0 + 6.0 + galley.size().x;
        let left_x = (trigger.center().x - content_w * 0.5).round();
        let sym_rect = Rect::from_center_size(pos2(left_x + 7.0, trigger.center().y), Vec2::splat(14.0));
        paint_symbol(painter, sym_rect, active_sym, text_color);
        painter.galley(pos2(left_x + 20.0, (trigger.center().y - galley.size().y * 0.5).round()), galley, text_color);
    }

    if reveal_t > 0.01 {
        let alpha = reveal_t.clamp(0.0, 1.0);
        let collapsed_w = 116.0_f32.min(rect.width());
        let dock_w = egui::lerp(collapsed_w..=rect.width(), reveal_t).round();
        let dock_h = egui::lerp(28.0..=height, reveal_t).round();
        let dock_rect = Rect::from_center_size(rect.center(), vec2(dock_w, dock_h));

        let painter = ui.painter();
        if !p.high_contrast {
            let shadow = egui::Shadow { offset: [0, 3], blur: 10, spread: 0, color: p.shadow.gamma_multiply(alpha) };
            painter.add(shadow.as_shape(dock_rect, outer_r));
        }
        painter.rect_filled(dock_rect, outer_r, p.surface.gamma_multiply(alpha));
        painter.rect_stroke(dock_rect, outer_r, Stroke::new(1.0, p.border.gamma_multiply(alpha)), StrokeKind::Inside);
        if !p.high_contrast && dock_rect.width() > f32::from(outer_px) * 2.0 + 4.0 {
            painter.hline(
                (dock_rect.left() + f32::from(outer_px))..=(dock_rect.right() - f32::from(outer_px)),
                dock_rect.top() + 1.0,
                Stroke::new(1.0, p.highlight.gamma_multiply(alpha)),
            );
        }

        let dock_painter = painter.with_clip_rect(dock_rect.expand2(vec2(4.0, 8.0)));
        let anim_idx = ui.ctx().animate_value_with_time(
            bar_resp.id.with("dock_pill"),
            selected_idx as f32,
            ui.style().animation_time,
        );
        let pill_x = (rect.left() + seg_w * anim_idx + 2.0).round();
        let pill_w = (seg_w - 4.0).round().max(8.0);
        let pill = Rect::from_min_size(pos2(pill_x, rect.top() + 2.0), vec2(pill_w, height - 4.0));
        if !p.high_contrast {
            let shadow = egui::Shadow { offset: [0, 1], blur: 3, spread: 0, color: p.shadow.gamma_multiply(alpha) };
            dock_painter.add(shadow.as_shape(pill, inner_r));
        }
        dock_painter.rect_filled(pill, inner_r, p.selection.gamma_multiply(alpha));
        if p.high_contrast {
            dock_painter.rect_stroke(
                pill,
                inner_r,
                Stroke::new(1.0, p.accent.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
        } else {
            dock_painter.rect_stroke(
                pill,
                inner_r,
                Stroke::new(1.0, p.border.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
            let inset = f32::from(inner_px);
            if pill.width() > inset * 2.0 + 2.0 {
                dock_painter.hline(
                    (pill.left() + inset)..=(pill.right() - inset),
                    pill.top() + 0.5,
                    Stroke::new(1.0, p.highlight.gamma_multiply(alpha)),
                );
            }
        }

        for (i, (opt, sym, label)) in options.iter().enumerate() {
            let seg =
                Rect::from_min_size(pos2(rect.left() + seg_w * i as f32, rect.top()), vec2(seg_w, height)).shrink(2.0);
            let selected = *value == *opt;
            let resp = &responses[i];
            let target_mag = if resp.has_focus() {
                1.0
            } else if pointer_in_zone && let Some(pt) = pointer_pos {
                let dist = ((pt.x - seg.center().x).abs() / (seg_w * 1.25)).clamp(0.0, 1.0);
                0.5 * (1.0 + (dist * std::f32::consts::PI).cos())
            } else {
                0.0
            };
            let mag = ui.ctx().animate_value_with_time(bar_resp.id.with(("mag", i)), target_mag, 0.12);

            if !selected && resp.hovered() {
                dock_painter.rect_filled(seg, inner_r, p.surface_hover.gamma_multiply(alpha));
            }
            let base_color = if !enabled {
                p.text_muted.gamma_multiply(0.6)
            } else if selected {
                p.selection_text
            } else if mag > 0.2 {
                lerp_color(p.text_muted, p.text, (mag - 0.2) / 0.8)
            } else {
                p.text_muted
            };
            let color = base_color.gamma_multiply(alpha);
            let galley = dock_painter.layout_no_wrap((*label).to_string(), semibold(13.0), color);
            let icon_size = 13.0 + 6.0 * mag;
            let gap = 5.0;
            let group_w = 13.0 + gap + galley.size().x;
            let base_left = (seg.center().x - group_w * 0.5).round();
            let icon_center = pos2((base_left + 6.5 - 1.5 * mag).round(), (seg.center().y - 4.5 * mag).round());
            let text_pos = pos2(
                (base_left + 13.0 + gap + 1.5 * mag).round(),
                (seg.center().y - galley.size().y * 0.5 - 1.5 * mag).round(),
            );

            if mag > 0.05 {
                let badge = Rect::from_center_size(icon_center, Vec2::splat(icon_size + 7.0));
                let badge_r = CornerRadius::same(((badge.width() * 0.28).round() as u8).max(4));
                dock_painter.rect_filled(badge, badge_r, p.selection.gamma_multiply(mag * alpha));
                dock_painter.rect_stroke(
                    badge,
                    badge_r,
                    Stroke::new(1.0, p.highlight.gamma_multiply(mag * alpha)),
                    StrokeKind::Inside,
                );
            }
            paint_symbol(&dock_painter, Rect::from_center_size(icon_center, Vec2::splat(icon_size)), *sym, color);
            dock_painter.galley(text_pos, galley, color);
            focus_ring(ui, seg, resp, p, inner_px);
        }
    }

    changed
}

/// Generic app tile used when no icon could be retrieved: a rounded square with
/// a calm tonal fill and the app's initial or vector symbol.
pub fn fallback_tile(ui: &Ui, rect: Rect, name: &str, look: &Look, symbol: Option<Symbol>) {
    let painter = ui.painter();
    let r = CornerRadius::same((rect.width() * 0.24) as u8);
    if let Some(sym) = symbol {
        painter.rect_filled(rect, r, look.p.accent_soft);
        painter.rect_stroke(rect, r, Stroke::new(1.0, look.p.border), StrokeKind::Inside);
        paint_symbol(painter, rect.shrink(rect.width() * 0.22), sym, look.p.accent);
        return;
    }
    let hue = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 360;
    let base = egui::ecolor::Hsva::new(
        hue as f32 / 360.0,
        if look.p.dark { 0.38 } else { 0.45 },
        if look.p.dark { 0.56 } else { 0.54 },
        1.0,
    );
    painter.rect_filled(rect, r, Color32::from(base));
    painter.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_white_alpha(24)), StrokeKind::Inside);
    let initial: String =
        name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    painter.text(rect.center(), Align2::CENTER_CENTER, initial, semibold(rect.width() * 0.5), Color32::WHITE);
}

/// A rounded surface used to group related controls.
pub fn card(look: &Look) -> egui::Frame {
    let pad = if look.m.row_height > 46.0 { 12 } else { 8 };
    egui::Frame::NONE
        .fill(look.p.surface)
        .stroke(Stroke::new(1.0, look.p.border))
        .corner_radius(CornerRadius::same(look.m.radius))
        .inner_margin(egui::Margin::same(pad))
}

/// Draws a subtle 1 px top-edge inner specular highlight on a card when not in high-contrast mode.
pub fn card_highlight(ui: &Ui, look: &Look, rect: Rect) {
    if look.p.high_contrast {
        return;
    }
    let inset = f32::from(look.m.radius);
    if rect.width() > inset * 2.0 + 4.0 {
        ui.painter().hline(
            (rect.left() + inset)..=(rect.right() - inset),
            rect.top() + 1.0,
            Stroke::new(1.0, look.p.highlight),
        );
    }
}

/// Hairline separator between rows inside a card.
pub fn row_divider(ui: &mut Ui, look: &Look) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
    ui.painter().hline(rect.left()..=rect.right(), rect.center().y, Stroke::new(1.0, look.p.border));
}

pub fn percent_label(ui: &mut Ui, look: &Look, text: &str, dim: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(42.0, look.m.button), Sense::hover());
    let color = if dim { look.p.text_muted } else { look.p.text };
    ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, text, semibold(13.0), color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Density, MIN_WIDTH};
    use crate::ui::test_support::{Harness, past_right_edge, unnamed};

    fn collect_shapes(shape: &egui::epaint::Shape, fills: &mut Vec<(Rect, Color32)>, texts: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Vec(list) => {
                for s in list {
                    collect_shapes(s, fills, texts);
                }
            }
            egui::epaint::Shape::Rect(r) => fills.push((r.rect, r.fill)),
            egui::epaint::Shape::Text(t) => texts.push(t.galley.text().to_owned()),
            _ => {}
        }
    }

    #[test]
    fn segmented_controls_in_same_scope_keep_distinct_pill_ids() {
        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        theme::apply(&harness.ctx, &look.p, &look.m, true);

        let mut a = 0usize;
        let mut b = 2usize;
        let opts_a = [(0usize, "A0"), (1, "A1"), (2, "A2")];
        let opts_b = [(0usize, "B0"), (1, "B1"), (2, "B2")];
        let mut fills = Vec::new();
        for step in 0..25 {
            fills.clear();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 200.0))),
                time: Some(step as f64 * 0.02),
                ..Default::default()
            };
            let output = harness.ctx.run_ui(input, |ui| {
                segmented(ui, &look, &mut a, &opts_a, 210.0);
                segmented(ui, &look, &mut b, &opts_b, 210.0);
            });
            let mut texts = Vec::new();
            for clipped in &output.shapes {
                collect_shapes(&clipped.shape, &mut fills, &mut texts);
            }
            output.drop_without_applying_deltas();
        }
        let pills: Vec<Rect> = fills.into_iter().filter(|(_, c)| *c == look.p.selection).map(|(r, _)| r).collect();
        assert_eq!(pills.len(), 2, "expected one active selection pill per control: {pills:?}");
        assert!(
            pills[0].center().x < 80.0 && pills[1].center().x > 130.0,
            "pills must stay on segment 0 and segment 2 cleanly without colliding: {pills:?}"
        );
    }

    #[test]
    fn top_dock_menu_exposes_selectors_fits_and_expands_on_hover() {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Tab {
            Mixer,
            Devices,
            Profiles,
            Settings,
        }

        let harness = Harness::new(Density::Comfortable);
        let look = harness.look;
        let width = MIN_WIDTH - 2.0 * look.m.pad;
        let options = [
            (Tab::Mixer, Symbol::SlidersHorizontal, "Mixer"),
            (Tab::Devices, Symbol::HifiSpeaker, "Devices"),
            (Tab::Profiles, Symbol::SliderVertical3, "Profiles"),
            (Tab::Settings, Symbol::Gear, "Settings"),
        ];

        let mut active = Tab::Mixer;
        let idle = harness.run(width, 120.0, |ui| {
            top_dock_menu(ui, &look, &mut active, &options, width.min(348.0));
        });
        assert!(unnamed(&idle.widgets).is_empty(), "{:?}", unnamed(&idle.widgets));
        assert!(past_right_edge(&idle.widgets, width).is_empty(), "{:?}", past_right_edge(&idle.widgets, width));
        let names: Vec<_> = idle
            .widgets
            .iter()
            .filter(|w| w.role == egui::accesskit::Role::RadioButton)
            .map(|w| w.name.as_str())
            .collect();
        assert_eq!(names, ["Mixer", "Devices", "Profiles", "Settings"]);

        // Idle frame should only paint the active view's label in the collapsed trigger pill.
        let idle_out = harness.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 120.0))),
                ..Default::default()
            },
            |ui| {
                top_dock_menu(ui, &look, &mut active, &options, width.min(348.0));
            },
        );
        let (mut fills, mut idle_texts) = (Vec::new(), Vec::new());
        for c in &idle_out.shapes {
            collect_shapes(&c.shape, &mut fills, &mut idle_texts);
        }
        idle_out.drop_without_applying_deltas();
        assert!(idle_texts.iter().any(|t| t == "Mixer"), "{idle_texts:?}");
        assert!(!idle_texts.iter().any(|t| t == "Devices"), "idle bar must stay collapsed: {idle_texts:?}");

        // Hover over Devices expands the dock and renders every label; clicking Profiles switches view.
        let devices_center =
            idle.widgets.iter().find(|w| w.name == "Devices").and_then(|w| w.bounds).expect("Devices").center();
        let hover_out = harness.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 120.0))),
                events: vec![egui::Event::PointerMoved(devices_center)],
                ..Default::default()
            },
            |ui| {
                top_dock_menu(ui, &look, &mut active, &options, width.min(348.0));
            },
        );
        let (mut hover_fills, mut hover_texts) = (Vec::new(), Vec::new());
        for c in &hover_out.shapes {
            collect_shapes(&c.shape, &mut hover_fills, &mut hover_texts);
        }
        hover_out.drop_without_applying_deltas();
        for expected in ["Mixer", "Devices", "Profiles", "Settings"] {
            assert!(hover_texts.iter().any(|t| t == expected), "expanded dock missing {expected}: {hover_texts:?}");
        }

        let profiles_center =
            idle.widgets.iter().find(|w| w.name == "Profiles").and_then(|w| w.bounds).expect("Profiles").center();
        for pressed in [true, false] {
            harness.run_with_events(
                width,
                120.0,
                vec![
                    egui::Event::PointerMoved(profiles_center),
                    egui::Event::PointerButton {
                        pos: profiles_center,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                |ui| {
                    top_dock_menu(ui, &look, &mut active, &options, width.min(348.0));
                },
            );
        }
        assert_eq!(active, Tab::Profiles);
    }
}
