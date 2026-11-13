//! Transient in-window notification toasts and display countdown.

use crate::audio::NoticeLevel;
use crate::ui::{Look, theme};
use egui::{Align2, Color32, CornerRadius, Margin};
use std::time::{Duration, Instant};

pub struct Toast {
    pub level: NoticeLevel,
    pub text: String,
    pub expiry: ToastExpiry,
}

/// Visible display time for a toast. The countdown runs only while the window
/// is shown, so a toast queued while hidden is displayed in full later.
pub struct ToastExpiry {
    remaining: Duration,
    /// When the toast should disappear; `None` while the countdown is paused.
    due: Option<Instant>,
}

impl ToastExpiry {
    pub fn new(visible_for: Duration) -> Self {
        Self { remaining: visible_for, due: None }
    }

    /// Tracks a visibility change; repeated calls with the same state have no effect.
    pub fn tick(&mut self, shown: bool, now: Instant) {
        if shown {
            if self.due.is_none() {
                self.due = Some(now + self.remaining);
            }
        } else if let Some(due) = self.due.take() {
            self.remaining = due.saturating_duration_since(now);
        }
    }

    pub fn due(&self) -> Option<Instant> {
        self.due
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.due.is_some_and(|due| due <= now)
    }
}

/// Appends a toast using level-based lifetimes, replacing duplicates and
/// keeping at most the three newest notices.
pub fn push_toast(toasts: &mut Vec<Toast>, level: NoticeLevel, text: String) {
    let secs = match level {
        NoticeLevel::Info => 4,
        NoticeLevel::Warning => 7,
        NoticeLevel::Error => 9,
    };
    toasts.retain(|t| t.text != text);
    toasts.push(Toast { level, text, expiry: ToastExpiry::new(Duration::from_secs(secs)) });
    while toasts.len() > 3 {
        toasts.remove(0);
    }
}

pub fn draw_toasts(ui: &mut egui::Ui, toasts: &mut Vec<Toast>, look: &Look) {
    let now = Instant::now();
    // Toasts queued during `ui` start counting down here.
    for toast in toasts.iter_mut() {
        toast.expiry.tick(true, now);
    }
    toasts.retain(|t| !t.expiry.expired(now));
    if let Some(next) = toasts.iter().filter_map(|t| t.expiry.due()).min() {
        ui.ctx().request_repaint_after(next - now);
    }
    if toasts.is_empty() {
        return;
    }
    let look = *look;
    let p = look.p;
    let mut dismiss = None;
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::CENTER_BOTTOM, [0.0, -12.0])
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(360.0);
            for (i, t) in toasts.iter().enumerate() {
                let accent = match t.level {
                    NoticeLevel::Info => p.accent,
                    NoticeLevel::Warning => Color32::from_rgb(0xf5, 0x9e, 0x0b),
                    NoticeLevel::Error => p.danger,
                };
                let prefix = match t.level {
                    NoticeLevel::Info => "",
                    NoticeLevel::Warning => "Warning: ",
                    NoticeLevel::Error => "Error: ",
                };
                let toast_id = egui::Id::new(("toast", t.text.as_str()));
                let seen_id = toast_id.with("seen");
                if !ui.ctx().data(|d| d.get_temp::<bool>(seen_id)).unwrap_or(false) {
                    ui.ctx().data_mut(|d| d.insert_temp(seen_id, true));
                    ui.ctx().animate_bool_with_time_and_easing(toast_id, false, 0.0, theme::motion::ease);
                }
                let alpha = ui.ctx().animate_bool_with_time_and_easing(
                    toast_id,
                    true,
                    theme::duration(ui, theme::motion::VIEW),
                    theme::motion::ease,
                );
                ui.scope(|ui| {
                    ui.multiply_opacity(alpha);
                    let resp = egui::Frame::NONE
                        .fill(p.surface_strong)
                        .stroke(egui::Stroke::new(1.0, if p.high_contrast { accent } else { p.border }))
                        .corner_radius(CornerRadius::same(14))
                        .inner_margin(Margin::symmetric(12, 8))
                        .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: p.shadow })
                        .show(ui, |ui| {
                            ui.set_width(336.0);
                            ui.horizontal(|ui| {
                                if look.icons {
                                    let glyph = match t.level {
                                        NoticeLevel::Info => crate::ui::widgets::glyph::INFO,
                                        NoticeLevel::Warning => crate::ui::widgets::glyph::WARNING,
                                        NoticeLevel::Error => crate::ui::widgets::glyph::ERROR,
                                    };
                                    let (rect, _) =
                                        ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::empty());
                                    ui.painter().text(
                                        rect.center(),
                                        Align2::CENTER_CENTER,
                                        glyph,
                                        crate::ui::widgets::icon_font(15.0),
                                        accent,
                                    );
                                }
                                ui.add(
                                    egui::Label::new(
                                        crate::ui::typography::rich(
                                            format!("{prefix}{}", t.text),
                                            crate::ui::typography::Role::Callout,
                                            &look,
                                        )
                                        .color(p.text),
                                    )
                                    .wrap(),
                                );
                            });
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_text("Click to dismiss");
                    if resp.clicked() {
                        dismiss = Some(i);
                    }
                });
                ui.add_space(6.0);
            }
        });
    if let Some(i) = dismiss {
        toasts.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_toast_time_is_frozen_and_restarts_when_first_shown() {
        let created = Instant::now();

        // Queued while hidden: the full duration starts at the reveal.
        let mut queued_while_hidden = ToastExpiry::new(Duration::from_secs(7));
        queued_while_hidden.tick(true, created + Duration::from_secs(3600));
        assert!(
            !queued_while_hidden.expired(created + Duration::from_secs(3600 + 6)),
            "a warning queued while hidden must survive a long idle and be displayable on reveal"
        );
        assert!(queued_while_hidden.expired(created + Duration::from_secs(3600 + 8)));

        // Hidden mid-countdown: the remaining time resumes on reveal.
        let mut frozen = ToastExpiry::new(Duration::from_secs(7));
        frozen.tick(true, created);
        frozen.tick(false, created + Duration::from_secs(3));
        frozen.tick(true, created + Duration::from_secs(3600));
        assert!(!frozen.expired(created + Duration::from_secs(3600 + 3)), "7s minus 3s shown = 4s left");
        assert!(frozen.expired(created + Duration::from_secs(3600 + 5)));
    }

    #[test]
    fn repeated_visible_passes_do_not_extend_a_toast() {
        let created = Instant::now();
        let mut expiry = ToastExpiry::new(Duration::from_secs(4));
        expiry.tick(true, created);
        expiry.tick(true, created + Duration::from_secs(2));
        assert!(!expiry.expired(created + Duration::from_millis(3900)));
        assert!(expiry.expired(created + Duration::from_secs(4)));
    }

    #[test]
    fn a_toast_expired_while_visible_stays_gone_across_hide_and_reveal() {
        let created = Instant::now();
        let mut expiry = ToastExpiry::new(Duration::from_secs(4));
        expiry.tick(true, created);
        assert!(expiry.expired(created + Duration::from_secs(4)));
        expiry.tick(false, created + Duration::from_secs(5));
        expiry.tick(true, created + Duration::from_secs(100));
        assert!(expiry.expired(created + Duration::from_secs(100)));
    }

    #[test]
    fn push_toast_deduplicates_and_caps_at_three() {
        let mut toasts = Vec::new();
        push_toast(&mut toasts, NoticeLevel::Info, "first".into());
        push_toast(&mut toasts, NoticeLevel::Warning, "second".into());
        push_toast(&mut toasts, NoticeLevel::Error, "first".into());
        assert_eq!(toasts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), vec!["second", "first"]);
        push_toast(&mut toasts, NoticeLevel::Info, "third".into());
        push_toast(&mut toasts, NoticeLevel::Info, "fourth".into());
        assert_eq!(toasts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), vec!["first", "third", "fourth"]);
    }
}
