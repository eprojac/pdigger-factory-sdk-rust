//! [`HandCards`]: what each hand is doing, as two cards.

use egui::{RichText, Sense, Ui, vec2};
use pdigger_factory_sdk::models::Side;
use pdigger_factory_sdk::{InterpreterState, WireEnum};

use crate::craft::Tokens;
use crate::craft::theme::{caption, card};
use crate::draw::hand_color;

/// Two cards, left and right hand: the hand's state (`free`, `touch`, `grasp`, `hinge`,
/// `carry`), the cards layer's title (`Holding a stone`) and the stone it holds.
///
/// ```
/// # let ctx = egui::Context::default();
/// # ctx.run_ui(Default::default(), |ui| {
/// let state = pdigger_factory_sdk::InterpreterState::default();
/// pdigger_factory_viz::HandCards::new(&state).show(ui);
/// # }).drop_without_applying_deltas();
/// ```
#[derive(Clone, Copy)]
pub struct HandCards<'a> {
    state: &'a InterpreterState,
}

impl<'a> HandCards<'a> {
    /// The cards of `state`.
    pub fn new(state: &'a InterpreterState) -> Self {
        HandCards { state }
    }

    /// The lines of one hand's card: state, title, stone.
    pub fn lines(&self, side: &Side) -> [Option<String>; 3] {
        let Some(hand) = self.state.hands.get(side) else { return [None, None, None] };
        [
            hand.state.as_ref().map(|s| s.as_str().to_string()),
            hand.card.clone().filter(|c| !c.is_empty()),
            hand.stone.map(|id| format!("stone {id}")),
        ]
    }

    /// Paints both cards side by side.
    pub fn show(self, ui: &mut Ui) {
        let t = Tokens::get(ui.ctx());
        ui.columns(2, |columns| {
            for (ui, side) in columns.iter_mut().zip([Side::Left, Side::Right]) {
                card(ui, &t, egui::Margin::same(2), egui::Margin::same(8), |ui| {
                    ui.horizontal(|ui| {
                        let (chip, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(chip.center(), 5.0, hand_color(Some(&side)));
                        ui.label(caption(if side == Side::Left { "Left hand" } else { "Right hand" }));
                    });
                    let [state, title, stone] = self.lines(&side);
                    ui.label(
                        RichText::new(state.unwrap_or_else(|| "no world yet".to_string()))
                            .color(t.text_strong)
                            .size(15.0),
                    );
                    ui.label(RichText::new(title.unwrap_or_default()).color(t.text));
                    ui.label(RichText::new(stone.unwrap_or_default()).color(t.text_dim));
                });
            }
        });
    }
}
