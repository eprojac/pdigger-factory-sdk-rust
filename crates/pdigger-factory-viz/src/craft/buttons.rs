// Vendored from https://github.com/storytold/craft-libs, crates/ui/src/buttons/mod.rs at
// 390fe98b0dd3fb84db7c3af7cbd958db4e8cc585.
// Copyright (c) 2026 ArtCraft Team and the craft-libs contributors.
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Changes for pdigger-factory-viz: `IconButtonStyle::from_tokens`; the single-shape frame option
// is dropped.

//! Icon buttons with application-supplied colours and painting.

use egui::{Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::theme::Tokens;

/// The background, border and icon colour of one interaction state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonVisuals {
    /// Face.
    pub fill: Color32,
    /// Border.
    pub stroke: Stroke,
    /// Icon tint.
    pub icon: Color32,
}

/// A small style adapter, independent of the application's complete theme.
#[derive(Clone, Copy, Debug)]
pub struct IconButtonStyle {
    /// At rest.
    pub normal: ButtonVisuals,
    /// Hovered.
    pub hovered: ButtonVisuals,
    /// Pressed (`None`: as hovered).
    pub pressed: Option<ButtonVisuals>,
    /// Selected (wins over press and hover).
    pub selected: ButtonVisuals,
    /// Corner radius.
    pub corner_radius: CornerRadius,
    /// An inset outline while the button has keyboard focus; `Stroke::NONE` opts out.
    pub focus_stroke: Stroke,
}

impl IconButtonStyle {
    /// The Craft tool button of `t`: flat, the accent well when selected.
    pub fn from_tokens(t: &Tokens) -> IconButtonStyle {
        let flat = |fill: Color32, icon: Color32| ButtonVisuals { fill, stroke: Stroke::NONE, icon };
        IconButtonStyle {
            normal: flat(Color32::TRANSPARENT, t.text),
            hovered: flat(t.row_hover, t.text_strong),
            pressed: Some(flat(t.tool_active, t.text_strong)),
            selected: ButtonVisuals { fill: t.tool_active, stroke: Stroke::new(1.0, t.accent), icon: t.accent },
            corner_radius: t.cr(),
            focus_stroke: Stroke::new(1.0, t.accent),
        }
    }

    fn visuals(&self, selected: bool, hovered: bool, pressed: bool) -> ButtonVisuals {
        if selected {
            self.selected
        } else if let Some(visuals) = self.pressed.filter(|_| pressed) {
            visuals
        } else if hovered {
            self.hovered
        } else {
            self.normal
        }
    }
}

/// An icon-only button. The accessible label is independent of the icon and the widget id.
pub struct IconButton<'a> {
    label: &'a str,
    size: Vec2,
    selected: bool,
    style: &'a IconButtonStyle,
}

impl<'a> IconButton<'a> {
    /// A button of `size` with the accessible `label`.
    pub fn new(label: &'a str, size: Vec2, style: &'a IconButtonStyle) -> Self {
        Self { label, size, selected: false, style }
    }

    /// Shows it selected (the caller owns the state).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Paints one button and returns its response. `paint_icon` receives the button rectangle
    /// and the icon tint. Non-finite, non-positive or huge sizes (over 4096 points) give an inert
    /// zero-size response without painting.
    pub fn show(self, ui: &mut Ui, paint_icon: impl FnOnce(&Ui, Rect, Color32)) -> Response {
        let valid = self.size.is_finite()
            && self.size.x > 0.0
            && self.size.y > 0.0
            && self.size.x <= 4096.0
            && self.size.y <= 4096.0;
        let (rect, response) = ui.allocate_exact_size(
            if valid { self.size } else { Vec2::ZERO },
            if valid { Sense::click() } else { Sense::hover() },
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, valid && ui.is_enabled(), self.selected, self.label)
        });
        if !valid {
            return response;
        }
        let visuals = self.style.visuals(self.selected, response.hovered(), response.is_pointer_button_down_on());
        let stroke = bounded_stroke(visuals.stroke, rect);
        ui.painter().rect(rect, self.style.corner_radius, visuals.fill, stroke, StrokeKind::Inside);
        paint_icon(ui, rect, visuals.icon);
        if response.has_focus() {
            let focus_rect = rect.shrink(2.0);
            ui.painter().rect_stroke(
                focus_rect,
                self.style.corner_radius,
                bounded_stroke(self.style.focus_stroke, focus_rect),
                StrokeKind::Inside,
            );
        }
        response
    }
}

/// A stroke no wider than half the rectangle (none when not finite or not positive).
fn bounded_stroke(stroke: Stroke, rect: Rect) -> Stroke {
    if stroke.width.is_finite() && stroke.width > 0.0 {
        Stroke::new(stroke.width.min(rect.width().min(rect.height()) * 0.5), stroke.color)
    } else {
        Stroke::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_pick_their_visuals() {
        let style = IconButtonStyle::from_tokens(&Tokens::default());
        assert_eq!(style.visuals(true, true, true), style.selected);
        assert_eq!(style.visuals(false, true, true), style.pressed.unwrap());
        assert_eq!(style.visuals(false, true, false), style.hovered);
        assert_eq!(style.visuals(false, false, false), style.normal);
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(4.0, 10.0));
        assert_eq!(bounded_stroke(Stroke::new(9.0, Color32::RED), rect).width, 2.0);
        assert_eq!(bounded_stroke(Stroke::new(f32::NAN, Color32::RED), rect), Stroke::NONE);
    }

    fn no_icon(_: &Ui, _: Rect, _: Color32) {}

    #[test]
    fn bad_sizes_are_inert_and_focus_is_drawn() {
        let ctx = egui::Context::default();
        let style = IconButtonStyle::from_tokens(&Tokens::default());
        let mut focused = Vec::new();
        for pass in 0..3 {
            let input = egui::RawInput { time: Some(f64::from(pass) / 60.0), ..Default::default() };
            ctx.run_ui(input, |ui| {
                for size in [egui::vec2(f32::NAN, 4.0), egui::vec2(0.0, 4.0), egui::vec2(5000.0, 4.0)] {
                    let inert = IconButton::new("Bad", size, &style).show(ui, no_icon);
                    assert_eq!((inert.rect.size(), inert.sense.senses_click()), (Vec2::ZERO, false));
                }
                let button = IconButton::new("Play", egui::vec2(20.0, 20.0), &style).show(ui, no_icon);
                if pass == 0 {
                    button.request_focus();
                }
                focused.push(button.has_focus());
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(focused, [true, true, true], "focus stays and is drawn in the next passes");
    }
}
