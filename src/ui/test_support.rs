//! Headless egui helpers for layout and accessibility regression tests.
//!
//! A [`Harness`] runs UI code against an in-memory screen with AccessKit
//! enabled, then reports every widget's accessible name, role and bounds. That
//! lets tests assert that nothing is drawn past the window edge and that every
//! interactive control has a name, without a display or a Windows session.

use super::{Look, theme};
use crate::settings::{Accent, Density};
use egui::{Pos2, Rect, Vec2, accesskit};
use std::collections::HashMap;

/// What the accessibility tree says about one widget.
#[derive(Clone, Debug)]
pub struct Widget {
    pub role: accesskit::Role,
    pub name: String,
    pub bounds: Option<Rect>,
    pub disabled: bool,
    pub has_description: bool,
}

/// One settled frame: the accessibility tree plus how far content reached.
pub struct Frame {
    pub widgets: Vec<Widget>,
    /// Right edge of everything the root UI allocated, including painter-only items
    /// (such as value labels) that have no accessibility node.
    pub content_right: f32,
}

pub struct Harness {
    pub ctx: egui::Context,
    pub look: Look,
}

impl Harness {
    pub fn new(density: Density) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let icons = crate::platform::fonts::install(&ctx);
        let look = Look { p: theme::palette(true, false, Accent::Violet, false), m: theme::metrics(density), icons };
        theme::apply(&ctx, &look.p, &look.m, false);
        Self { ctx, look }
    }

    /// Runs `ui_fn` for several passes on a `width` × `height` screen (fonts and
    /// layout need a few frames to settle) and returns the accessibility tree.
    pub fn run(&self, width: f32, height: f32, mut ui_fn: impl FnMut(&mut egui::Ui)) -> Frame {
        let mut nodes: HashMap<accesskit::NodeId, Widget> = HashMap::new();
        // Text of every node (label or value) and who labels whom, to resolve names
        // that come from a separate label widget (`labelled_by`).
        let mut text: HashMap<accesskit::NodeId, String> = HashMap::new();
        let mut labelled_by: HashMap<accesskit::NodeId, Vec<accesskit::NodeId>> = HashMap::new();
        let mut content_right = 0.0_f32;
        for pass in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
                time: Some(pass as f64 * 0.1),
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| {
                ui_fn(ui);
                content_right = ui.min_rect().right();
            });
            let update = output.platform_output.accesskit_update.take();
            output.drop_without_applying_deltas();
            // Updates are incremental: keep the latest version of every node.
            if let Some(update) = update {
                for (id, node) in update.nodes {
                    let own_text = node.label().or_else(|| node.value()).unwrap_or_default().to_owned();
                    text.insert(id, own_text);
                    labelled_by.insert(id, node.labelled_by().to_vec());
                    nodes.insert(
                        id,
                        Widget {
                            role: node.role(),
                            // Static text carries its text as the node value; for other roles
                            // the value is user content and must not count as a name.
                            name: node
                                .label()
                                .or_else(|| {
                                    matches!(node.role(), accesskit::Role::Label | accesskit::Role::Heading)
                                        .then(|| node.value())
                                        .flatten()
                                })
                                .unwrap_or_default()
                                .to_owned(),
                            bounds: node.bounds().map(|b| {
                                Rect::from_min_max(
                                    Pos2::new(b.x0 as f32, b.y0 as f32),
                                    Pos2::new(b.x1 as f32, b.y1 as f32),
                                )
                            }),
                            disabled: node.is_disabled(),
                            has_description: node.description().is_some_and(|d| !d.is_empty()),
                        },
                    );
                }
            }
        }
        for (id, widget) in &mut nodes {
            if widget.name.trim().is_empty() {
                widget.name = labelled_by
                    .get(id)
                    .into_iter()
                    .flatten()
                    .filter_map(|label| text.get(label))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
            }
        }
        Frame { widgets: nodes.into_values().collect(), content_right }
    }
}

/// Roles a user can interact with; each must have an accessible name.
pub fn is_interactive(role: accesskit::Role) -> bool {
    use accesskit::Role::*;
    matches!(
        role,
        Button | CheckBox | RadioButton | ComboBox | TextInput | SpinButton | Slider | SearchInput | MultilineTextInput
    )
}

/// Interactive widgets that have no accessible name.
pub fn unnamed(widgets: &[Widget]) -> Vec<&Widget> {
    widgets.iter().filter(|w| is_interactive(w.role) && w.name.trim().is_empty()).collect()
}

/// Widgets whose right edge is more than half a pixel beyond `right`.
pub fn past_right_edge(widgets: &[Widget], right: f32) -> Vec<&Widget> {
    widgets.iter().filter(|w| w.bounds.is_some_and(|b| b.max.x > right + 0.5)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Sense;

    #[test]
    fn harness_sees_painter_only_overflow() {
        let harness = Harness::new(Density::Comfortable);
        let frame = harness.run(300.0, 200.0, |ui| {
            ui.allocate_exact_size(Vec2::new(500.0, 20.0), Sense::hover());
        });
        assert!(frame.content_right >= 500.0, "content_right was {}", frame.content_right);
    }

    #[test]
    fn harness_flags_unnamed_controls_and_overflowing_widgets() {
        let harness = Harness::new(Density::Comfortable);
        let mut text = String::new();
        let frame = harness.run(300.0, 200.0, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(100.0));
            ui.put(Rect::from_min_size(Pos2::new(250.0, 100.0), Vec2::new(200.0, 24.0)), egui::Button::new("Far"));
        });
        assert_eq!(unnamed(&frame.widgets).len(), 1, "an unlabeled text field must be reported");
        assert_eq!(
            past_right_edge(&frame.widgets, 300.0).len(),
            1,
            "a button placed to x=450 in a 300-wide window overflows"
        );

        let frame = harness.run(300.0, 200.0, |ui| {
            let label = ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(100.0)).labelled_by(label.id);
        });
        assert!(unnamed(&frame.widgets).is_empty(), "a labelled field is named: {:?}", unnamed(&frame.widgets));
        assert!(past_right_edge(&frame.widgets, 300.0).is_empty());
    }
}
