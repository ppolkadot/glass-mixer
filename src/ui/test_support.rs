//! Headless egui helpers for layout and accessibility regression tests.
//!
//! A [`Harness`] runs UI code against an in-memory screen with AccessKit
//! enabled, then reports every reachable widget's accessible name, role, value
//! and bounds from the final settled tree. That lets tests assert that
//! horizontal allocation stays within the window width and that every
//! interactive control has a name, without a display or a Windows session.
//!
//! Note: [`Frame::content_right`] and [`past_right_edge`] measure unclipped
//! layout allocation extent rather than visual viewport clipping, so horizontal
//! overruns inside scroll areas or cards are still caught.

use super::{Look, theme};
use crate::settings::{Accent, Density};
use egui::{Pos2, Rect, Vec2, accesskit};
use std::collections::{HashMap, HashSet};

/// What the settled accessibility tree says about one reachable widget.
#[derive(Clone, Debug)]
pub struct Widget {
    pub role: accesskit::Role,
    pub name: String,
    pub value: Option<String>,
    pub disabled: bool,
    pub focused: bool,
    pub has_description: bool,
    pub bounds: Option<Rect>,
}

/// One settled frame: the reachable accessibility tree plus how far layout allocation reached.
pub struct Frame {
    pub widgets: Vec<Widget>,
    /// Right edge of everything the root UI allocated, including painter-only items
    /// (such as value labels) that have no accessibility node. Measures allocation
    /// extent rather than visual clipping.
    pub content_right: f32,
}

pub struct Harness {
    pub ctx: egui::Context,
    pub look: Look,
    time: std::cell::Cell<f64>,
}

impl Harness {
    pub fn new(density: Density) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let icons = crate::platform::fonts::install(&ctx);
        let look = Look { p: theme::palette(true, false, Accent::Violet, false), m: theme::metrics(density), icons };
        theme::apply(&ctx, &look.p, &look.m, false);
        Self { ctx, look, time: std::cell::Cell::new(0.0) }
    }

    /// Runs `ui_fn` for several passes on a `width` × `height` screen (fonts and
    /// layout need a few frames to settle) and returns the reachable accessibility
    /// tree of the final settled pass.
    pub fn run(&self, width: f32, height: f32, ui_fn: impl FnMut(&mut egui::Ui)) -> Frame {
        self.run_with_events(width, height, Vec::new(), ui_fn)
    }

    /// Delivers `events` on the first pass, then runs settling passes on a
    /// `width` × `height` screen and returns the settled [`Frame`].
    pub fn run_with_events(
        &self,
        width: f32,
        height: f32,
        events: Vec<egui::Event>,
        mut ui_fn: impl FnMut(&mut egui::Ui),
    ) -> Frame {
        let mut pending_events = Some(events);
        let mut last_update: Option<accesskit::TreeUpdate> = None;
        let mut content_right = 0.0_f32;
        for _ in 0..4 {
            let t = self.time.get() + 0.01;
            self.time.set(t);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
                time: Some(t),
                events: pending_events.take().unwrap_or_default(),
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| {
                ui_fn(ui);
                content_right = ui.min_rect().right();
            });
            if let Some(update) = output.platform_output.accesskit_update.take() {
                last_update = Some(update);
            }
            output.drop_without_applying_deltas();
        }
        let Some(update) = last_update else {
            return Frame { widgets: Vec::new(), content_right };
        };
        let focus_id = update.focus;
        let raw_nodes: HashMap<accesskit::NodeId, accesskit::Node> = update.nodes.into_iter().collect();
        let root_id = update.tree.map(|t| t.root).or_else(|| {
            raw_nodes.iter().find_map(|(&id, node)| (node.role() == accesskit::Role::Window).then_some(id))
        });

        let mut reachable_ids = Vec::new();
        let mut visited = HashSet::new();
        if let Some(root) = root_id {
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                if !visited.insert(id) {
                    continue;
                }
                let Some(node) = raw_nodes.get(&id) else {
                    continue;
                };
                reachable_ids.push(id);
                for &child in node.children().iter().rev() {
                    stack.push(child);
                }
            }
        } else {
            reachable_ids.extend(raw_nodes.keys().copied());
            visited.extend(reachable_ids.iter().copied());
        }

        fn label_text(node: &accesskit::Node) -> Option<&str> {
            node.label().or_else(|| {
                matches!(node.role(), accesskit::Role::Label | accesskit::Role::Heading).then(|| node.value()).flatten()
            })
        }

        let mut widgets = Vec::with_capacity(reachable_ids.len());
        for id in reachable_ids {
            let Some(node) = raw_nodes.get(&id) else {
                continue;
            };
            let mut name = label_text(node).unwrap_or_default().to_owned();
            if name.trim().is_empty() {
                name = node
                    .labelled_by()
                    .iter()
                    .filter(|label_id| visited.contains(label_id))
                    .filter_map(|label_id| raw_nodes.get(label_id).and_then(label_text))
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            widgets.push(Widget {
                role: node.role(),
                name,
                value: node.value().map(str::to_owned),
                disabled: node.is_disabled(),
                focused: id == focus_id,
                has_description: node.description().is_some_and(|d| !d.is_empty()),
                bounds: node.bounds().map(|b| {
                    Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32))
                }),
            });
        }
        Frame { widgets, content_right }
    }

    /// Sends a single key-press event and settles the frame.
    pub fn send_key(
        &self,
        width: f32,
        height: f32,
        key: egui::Key,
        modifiers: egui::Modifiers,
        ui_fn: impl FnMut(&mut egui::Ui),
    ) -> Frame {
        self.run_with_events(
            width,
            height,
            vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ui_fn,
        )
    }
}

/// Roles a user can interact with; each must have an accessible name.
pub fn is_interactive(role: accesskit::Role) -> bool {
    use accesskit::Role::*;
    matches!(
        role,
        Button
            | CheckBox
            | RadioButton
            | ComboBox
            | TextInput
            | SpinButton
            | Slider
            | SearchInput
            | MultilineTextInput
            | Link
    )
}

/// Interactive widgets that have no accessible name.
pub fn unnamed(widgets: &[Widget]) -> Vec<&Widget> {
    widgets.iter().filter(|w| is_interactive(w.role) && w.name.trim().is_empty()).collect()
}

/// Widgets whose allocated right edge is more than half a pixel beyond `right`.
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
    fn harness_flags_unnamed_controls_links_and_overflowing_widgets() {
        let harness = Harness::new(Density::Comfortable);
        let mut text = String::from("user typed content");
        let frame = harness.run(300.0, 200.0, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(100.0));
            let _ = ui.link("");
            ui.put(Rect::from_min_size(Pos2::new(250.0, 100.0), Vec2::new(200.0, 24.0)), egui::Button::new("Far"));
        });
        assert_eq!(
            unnamed(&frame.widgets).len(),
            2,
            "an unlabeled text field (even with typed value) and an empty link must both be reported: {:?}",
            unnamed(&frame.widgets)
        );
        assert_eq!(
            past_right_edge(&frame.widgets, 300.0).len(),
            1,
            "a button placed to x=450 in a 300-wide window overflows"
        );

        let frame = harness.run(300.0, 200.0, |ui| {
            let label = ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(100.0)).labelled_by(label.id);
            let _ = ui.link("Balance and sound properties");
        });
        assert!(
            unnamed(&frame.widgets).is_empty(),
            "a labelled field and named link have names: {:?}",
            unnamed(&frame.widgets)
        );
        assert!(past_right_edge(&frame.widgets, 300.0).is_empty());
        let input = frame.widgets.iter().find(|w| w.role == accesskit::Role::TextInput).expect("text input");
        assert_eq!(input.name, "Name");
        assert_eq!(input.value.as_deref(), Some("user typed content"));
    }

    #[test]
    fn harness_drops_first_pass_only_and_disappearing_controls_from_settled_frame() {
        let harness = Harness::new(Density::Comfortable);
        let mut pass = 0;
        let mut text = String::from("typed");
        let frame = harness.run(300.0, 200.0, |ui| {
            if pass == 0 {
                let _ = ui.button("Transient first-pass button");
                ui.add(egui::TextEdit::singleline(&mut text).desired_width(100.0));
                ui.put(
                    Rect::from_min_size(Pos2::new(260.0, 80.0), Vec2::new(200.0, 24.0)),
                    egui::Button::new("Transient overflow"),
                );
            }
            let _ = ui.add_enabled(false, egui::Button::new("Persistent button"));
            pass += 1;
        });
        assert!(
            !frame.widgets.iter().any(|w| w.name == "Transient first-pass button"),
            "first-pass-only button must not remain in settled frame"
        );
        assert!(
            unnamed(&frame.widgets).is_empty(),
            "disappearing unnamed control must not fail final assertions: {:?}",
            unnamed(&frame.widgets)
        );
        assert!(
            past_right_edge(&frame.widgets, 300.0).is_empty(),
            "disappearing overflowing control must not fail final assertions"
        );
        let persistent = frame.widgets.iter().find(|w| w.name == "Persistent button").expect("persistent button");
        assert_eq!(persistent.role, accesskit::Role::Button);
        assert!(persistent.disabled);
        assert!(persistent.bounds.is_some());
    }
}
