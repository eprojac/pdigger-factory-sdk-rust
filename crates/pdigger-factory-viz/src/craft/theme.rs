// Vendored from https://github.com/storytold/texturecraft, crates/ui-egui/src/theme.rs at
// 97f6b819846b40b1f7f5272858f643ca285068b9, and https://github.com/storytold/craft-libs,
// crates/ui/src/theme/mod.rs (`WidgetPalette`) at 390fe98b0dd3fb84db7c3af7cbd958db4e8cc585.
// Copyright (c) 2026 ArtCraft Team and the TextureCraft and craft-libs contributors.
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Changes for pdigger-factory-viz: no translation markers and no settings serialisation; egui's
// default fonts (no Inter or Phosphor icon font); `WidgetPalette` moved into this file.

//! Design tokens and egui styling: the Craft look (cool slate greys in four brightness levels,
//! flat faces with soft corners, panels as rounded cards and one accent colour).

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals, style::Widgets};

/// The greys of the interface. The 3-D and video views keep their studio slate in every level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Brightness {
    /// Near black.
    Dark,
    /// Charcoal, the default.
    #[default]
    MediumDark,
    /// Light grey.
    MediumLight,
    /// Near white.
    Light,
}

impl Brightness {
    /// Every level, darkest first.
    pub const ALL: [Brightness; 4] =
        [Brightness::Dark, Brightness::MediumDark, Brightness::MediumLight, Brightness::Light];

    /// The English label.
    pub fn label(self) -> &'static str {
        match self {
            Brightness::Dark => "Dark",
            Brightness::MediumDark => "Medium dark",
            Brightness::MediumLight => "Medium light",
            Brightness::Light => "Light",
        }
    }
}

/// The one colour for what is on, chosen or focused. Copper is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Accent {
    /// The default.
    #[default]
    Copper,
    /// Coral.
    Coral,
    /// Blue.
    Blue,
    /// Violet.
    Violet,
    /// Amber.
    Amber,
    /// Pink.
    Pink,
}

impl Accent {
    /// Every accent.
    pub const ALL: [Accent; 6] =
        [Accent::Copper, Accent::Coral, Accent::Blue, Accent::Violet, Accent::Amber, Accent::Pink];

    /// The English label.
    pub fn label(self) -> &'static str {
        match self {
            Accent::Copper => "Copper",
            Accent::Coral => "Coral",
            Accent::Blue => "Blue",
            Accent::Violet => "Violet",
            Accent::Amber => "Amber",
            Accent::Pink => "Pink",
        }
    }

    /// (accent, accent_strong) on dark panels and on light ones; the strong one fills buttons
    /// with white lettering.
    pub(crate) fn colors(self, dark: bool) -> (Color32, Color32) {
        let (a, strong) = match (self, dark) {
            (Accent::Copper, true) => (0xe58a4e, 0xc2672b),
            (Accent::Copper, false) => (0xae5419, 0xb85c22),
            (Accent::Coral, true) => (0xf06a52, 0xe8573f),
            (Accent::Coral, false) => (0xd9472f, 0xd14a33),
            (Accent::Blue, true) => (0x5b96ff, 0x2f74f0),
            (Accent::Blue, false) => (0x2563eb, 0x1f5ad6),
            (Accent::Violet, true) => (0xa98bff, 0x7c55f0),
            (Accent::Violet, false) => (0x7146dc, 0x6a3fd4),
            (Accent::Amber, true) => (0xf5b544, 0xc47f0e),
            (Accent::Amber, false) => (0xb06d06, 0xb06d06),
            (Accent::Pink, true) => (0xf36aa3, 0xdb3f7c),
            (Accent::Pink, false) => (0xcc3672, 0xc22e69),
        };
        (hex(a), hex(strong))
    }
}

/// A colour from `0xRRGGBB`.
pub const fn hex(s: u32) -> Color32 {
    Color32::from_rgb((s >> 16) as u8, (s >> 8) as u8, s as u8)
}

/// Colours and metrics used across the UI, fetched each frame with [`Tokens::get`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Light lettering on dark faces (false for the light brightness levels).
    pub dark: bool,
    /// The window's frame: menu bar, status bar and the margins around the cards.
    pub chrome: Color32,
    /// Card faces.
    pub panel: Color32,
    /// Insets on a card: list rows.
    pub row: Color32,
    /// A hovered row.
    pub row_hover: Color32,
    /// Selected rows: the panel tinted towards the accent.
    pub row_selected: Color32,
    /// Hairlines inside cards.
    pub separator: Color32,
    /// Text.
    pub text: Color32,
    /// Secondary text.
    pub text_dim: Color32,
    /// Emphasised text.
    pub text_strong: Color32,
    /// What is on, chosen or focused.
    pub accent: Color32,
    /// Buttons filled with the accent (white lettering).
    pub accent_strong: Color32,
    /// Accent-tinted fills: selected buttons and text.
    pub accent_dim: Color32,
    /// Text fields.
    pub field: Color32,
    /// Text field borders.
    pub field_border: Color32,
    /// Flat button faces.
    pub button: Color32,
    /// Slider rails and outlines that must show on a card.
    pub button_border: Color32,
    /// The well behind a toggled button (accent-tinted).
    pub tool_active: Color32,
    /// Errors.
    pub danger: Color32,
    /// Warnings.
    pub warning: Color32,
    /// The 3-D and video views' background: a dark, faintly cool slate in every brightness.
    pub viewport: Color32,
    /// Lettering drawn over the views (always on their dark slate).
    pub viewport_text: Color32,
    /// The accent drawn over the views.
    pub viewport_accent: Color32,
    /// Corner radius of controls.
    pub radius: u8,
}

impl Default for Tokens {
    fn default() -> Self {
        Tokens::for_brightness(Brightness::default()).with_accent(Accent::default())
    }
}

impl Tokens {
    /// The greys of brightness `b`; the accent colours are placeholders until
    /// [`Tokens::with_accent`].
    pub fn for_brightness(b: Brightness) -> Self {
        let base = Tokens {
            dark: true,
            chrome: hex(0x0b0d12),
            panel: hex(0x181b22),
            row: hex(0x13161c),
            row_hover: hex(0x262b35),
            row_selected: hex(0x3a2a2c),
            separator: hex(0x232832),
            text: hex(0xdfe4ec),
            text_dim: hex(0x969faf),
            text_strong: hex(0xffffff),
            accent: hex(0xe58a4e),
            accent_strong: hex(0xc2672b),
            accent_dim: hex(0x5a3e30),
            field: hex(0x0f1116),
            field_border: hex(0x2c323e),
            button: hex(0x282d37),
            button_border: hex(0x4a5363),
            tool_active: hex(0x2f3541),
            danger: hex(0xe0554a),
            warning: hex(0xe0b040),
            viewport: hex(0x272b31),
            viewport_text: hex(0x9aa1ad),
            viewport_accent: hex(0xe58a4e),
            radius: 5,
        };
        let light = Tokens {
            dark: false,
            text: hex(0x1b1f27),
            text_strong: hex(0x000000),
            danger: hex(0xc8372d),
            warning: hex(0x8f5f00),
            ..base
        };
        match b {
            Brightness::Dark => base,
            Brightness::MediumDark => Tokens {
                chrome: hex(0x1b1f27),
                panel: hex(0x272c36),
                row: hex(0x21262f),
                row_hover: hex(0x353c49),
                separator: hex(0x323946),
                text: hex(0xe4e9f1),
                text_dim: hex(0x99a2b2),
                field: hex(0x1a1e26),
                field_border: hex(0x3a414e),
                button: hex(0x39404d),
                button_border: hex(0x5a6476),
                tool_active: hex(0x3d4553),
                ..base
            },
            Brightness::MediumLight => Tokens {
                chrome: hex(0xc4cad4),
                panel: hex(0xd3d8e0),
                row: hex(0xc9cfd8),
                row_hover: hex(0xc5ccd6),
                separator: hex(0xbcc3ce),
                text_dim: hex(0x4a5160),
                field: hex(0xe9ecf1),
                field_border: hex(0x9aa2b0),
                button: hex(0xc9d0da),
                button_border: hex(0x8a93a2),
                tool_active: hex(0xb6bec9),
                ..light
            },
            Brightness::Light => Tokens {
                chrome: hex(0xe6e9ee),
                panel: hex(0xf3f5f8),
                row: hex(0xe9ecf1),
                row_hover: hex(0xe3e7ed),
                separator: hex(0xdde1e8),
                text_dim: hex(0x58606e),
                field: hex(0xffffff),
                field_border: hex(0xc3c9d3),
                button: hex(0xe3e7ed),
                button_border: hex(0xa4abb8),
                tool_active: hex(0xd5dbe3),
                ..light
            },
        }
    }

    /// These tokens with `accent`: the accent, buttons filled with it, and selected rows and
    /// toggled buttons (the panel tinted towards it).
    pub fn with_accent(self, accent: Accent) -> Self {
        let (a, strong) = accent.colors(self.dark);
        let tint = |k: f32| {
            let mix = |p: u8, q: u8| (f32::from(p) + (f32::from(q) - f32::from(p)) * k).round() as u8;
            Color32::from_rgb(mix(self.panel.r(), a.r()), mix(self.panel.g(), a.g()), mix(self.panel.b(), a.b()))
        };
        Tokens {
            accent: a,
            accent_strong: strong,
            accent_dim: tint(0.4),
            row_selected: tint(if self.dark { 0.24 } else { 0.2 }),
            tool_active: tint(if self.dark { 0.3 } else { 0.24 }),
            viewport_accent: accent.colors(true).0,
            ..self
        }
    }

    /// The tokens [`apply`] installed (the defaults before that).
    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(egui::Id::NULL)).unwrap_or_default()
    }

    /// The corner radius of controls.
    pub fn cr(&self) -> CornerRadius {
        CornerRadius::same(self.radius)
    }
}

/// The corner radius of floating windows and menus.
pub const CARD_RADIUS: u8 = 8;

/// The window-frame gap between neighbouring cards.
pub const GUTTER: f32 = 6.0;

/// A card: `body` drawn `padding` inside the panel colour, `margin` of the window's frame colour
/// around it.
pub fn card<R>(
    ui: &mut egui::Ui,
    t: &Tokens,
    margin: egui::Margin,
    padding: egui::Margin,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::NONE
        .fill(t.chrome)
        .inner_margin(margin)
        .show(ui, |ui| {
            egui::Frame::NONE
                .fill(t.panel)
                .corner_radius(t.cr())
                .inner_margin(padding)
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    body(ui)
                })
                .inner
        })
        .inner
}

/// Interface text in the semibold weight: headings and window titles.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// The semibold family (egui's proportional faces).
pub(crate) const SEMIBOLD: &str = "ui-semibold";

/// egui's default fonts plus the semibold family.
pub fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let list = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), list);
    fonts
}

/// Background colours for egui's five widget interaction states (craft-ui's `WidgetPalette`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidgetPalette {
    /// Labels and frames.
    pub noninteractive: Color32,
    /// Controls at rest.
    pub inactive: Color32,
    /// Hovered controls.
    pub hovered: Color32,
    /// Pressed controls.
    pub active: Color32,
    /// Open menus and combo boxes.
    pub open: Color32,
}

impl WidgetPalette {
    /// Sets each state's strong and weak background fill, with a common radius and foreground.
    /// Borders, expansion and every other widget property are kept.
    pub fn apply_to(self, widgets: &mut Widgets, radius: CornerRadius, foreground: Stroke) {
        for (widget, fill) in [
            (&mut widgets.noninteractive, self.noninteractive),
            (&mut widgets.inactive, self.inactive),
            (&mut widgets.hovered, self.hovered),
            (&mut widgets.active, self.active),
            (&mut widgets.open, self.open),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.corner_radius = radius;
            widget.fg_stroke = foreground;
        }
    }
}

/// Installs the fonts, stores the tokens of `b` and `accent` for [`Tokens::get`] and styles egui
/// with them. Cheap enough for a settings change, not meant for every frame.
pub fn apply(ctx: &egui::Context, b: Brightness, accent: Accent) {
    let t = Tokens::for_brightness(b).with_accent(accent);
    ctx.set_fonts(font_definitions());
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, t));
    ctx.all_styles_mut(|s| {
        // The type scale: 11 small print, 13 body and controls, 15 headings, 11.5 values.
        s.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Heading, semibold(15.0)),
            (TextStyle::Monospace, FontId::new(11.5, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.interact_size = egui::vec2(24.0, 24.0);
        s.spacing.slider_width = 120.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.window_margin = egui::Margin::same(12);
        s.animation_time = 0.08;
        let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
        v.panel_fill = t.panel;
        v.window_fill = t.panel;
        v.extreme_bg_color = t.field;
        v.text_edit_bg_color = Some(t.field);
        v.faint_bg_color = t.row;
        v.code_bg_color = t.field;
        v.window_stroke = Stroke::new(1.0, t.separator);
        v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.selection.bg_fill = t.accent_dim;
        v.selection.stroke = Stroke::new(1.0, t.text_strong);
        v.hyperlink_color = t.accent;
        let w = &mut v.widgets;
        WidgetPalette {
            noninteractive: t.panel,
            inactive: t.button,
            hovered: t.row_hover,
            active: t.tool_active,
            open: t.row_hover,
        }
        .apply_to(w, t.cr(), Stroke::new(1.0, t.text));
        w.hovered.fg_stroke.color = t.text_strong;
        w.active.fg_stroke.color = t.text_strong;
        w.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
        // Flat buttons: the outline is the face's own colour, so only text fields show it.
        w.inactive.bg_stroke = Stroke::new(1.0, t.button);
        w.hovered.bg_stroke = Stroke::new(1.0, t.field_border);
        w.active.bg_stroke = Stroke::new(1.0, t.accent);
        v.slider_trailing_fill = true;
        v.striped = false;
        s.visuals = v;
    });
}

/// A section caption inside a panel: semibold in the strong colour, as written.
pub fn caption(text: &str) -> egui::RichText {
    egui::RichText::new(text).font(semibold(12.5)).strong()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance (WCAG) and contrast ratio.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let ch = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_brightness_and_accent_reads() {
        for b in Brightness::ALL {
            assert!(!b.label().is_empty());
            for a in Accent::ALL {
                assert!(!a.label().is_empty());
                let t = Tokens::for_brightness(b).with_accent(a);
                assert_eq!(t.dark, matches!(b, Brightness::Dark | Brightness::MediumDark));
                assert!(contrast(t.text, t.panel) >= 7.0, "{b:?}: text on panel");
                assert!(contrast(t.text_dim, t.panel) >= 4.5, "{b:?}: dim text on panel");
                assert!(contrast(t.text_strong, t.accent_dim) >= 4.5, "{b:?} {a:?}: a selected option");
                assert!(contrast(t.accent, t.panel) >= 2.5, "{b:?} {a:?}: accent on panel");
                assert!(contrast(t.viewport_text, t.viewport) >= 3.0, "{b:?}: lettering on the views");
            }
        }
        assert_eq!(
            Tokens::for_brightness(Brightness::Light).viewport,
            Tokens::for_brightness(Brightness::Dark).viewport
        );
    }

    #[test]
    fn apply_styles_the_context_and_stores_the_tokens() {
        let ctx = egui::Context::default();
        assert_eq!(Tokens::get(&ctx), Tokens::default());
        apply(&ctx, Brightness::Light, Accent::Blue);
        let t = Tokens::get(&ctx);
        assert!(!t.dark);
        assert_eq!(t.accent, Accent::Blue.colors(false).0);
        assert_eq!(ctx.global_style().visuals.panel_fill, t.panel);
        assert_eq!(t.cr(), CornerRadius::same(5));
        assert!(caption("Hands").text() == "Hands");
        assert!(font_definitions().families.contains_key(&FontFamily::Name(SEMIBOLD.into())));
    }

    #[test]
    fn palettes_keep_borders_and_expansion() {
        let mut widgets = egui::Visuals::light().widgets;
        widgets.hovered.expansion = 7.0;
        let palette = WidgetPalette {
            noninteractive: Color32::BLACK,
            inactive: Color32::RED,
            hovered: Color32::GREEN,
            active: Color32::BLUE,
            open: Color32::TRANSPARENT,
        };
        palette.apply_to(&mut widgets, CornerRadius::same(4), Stroke::new(2.0, Color32::WHITE));
        assert_eq!((widgets.inactive.bg_fill, widgets.inactive.weak_bg_fill), (Color32::RED, Color32::RED));
        assert_eq!(widgets.hovered.expansion, 7.0);
    }
}
