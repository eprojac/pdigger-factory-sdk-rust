//! [`EventFeed`]: the interpreter's sentences, newest first, with hand colours.

use std::collections::VecDeque;

use egui::{Color32, Rect, RichText, Sense, Stroke, Ui, vec2};
use pdigger_factory_sdk::models::Side;
use pdigger_factory_sdk::{Activity, Event, TimeRef, Update};

use crate::craft::{IconButton, IconButtonStyle, Tokens};
use crate::draw::hand_color;

/// One line of the feed.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedLine {
    /// When.
    pub at: TimeRef,
    /// The sentence (`5:01.6 Right hand lifts stone 643`).
    pub text: String,
    /// The hand.
    pub hand: Option<Side>,
    /// An activity (a move or a push burst) rather than an event.
    pub activity: bool,
    /// The stone it is about (the first of a burst).
    pub stone: Option<i64>,
}

impl FeedLine {
    /// The line of an event.
    pub fn event(e: &Event) -> FeedLine {
        FeedLine { at: e.at(), text: e.describe(), hand: e.hand.clone(), activity: false, stone: e.stone_id }
    }

    /// The line of an activity.
    pub fn activity(a: &Activity) -> FeedLine {
        let stone = match a {
            Activity::Move(m) => m.stone_id,
            Activity::Push(p) => p.stones.first().copied(),
        };
        FeedLine { at: a.end(), text: a.describe(), hand: a.hand().cloned(), activity: true, stone }
    }
}

/// The interpreter's events and activities as a scrolling list, newest first, each with its
/// hand's colour. Two toggles choose events, activities or both. Hovering a line names its stone
/// (to highlight it in the other views).
///
/// ```
/// let mut feed = pdigger_factory_viz::EventFeed::new(100);
/// assert!(feed.is_empty());
/// # let ctx = egui::Context::default();
/// # ctx.run_ui(Default::default(), |ui| { feed.show(ui); }).drop_without_applying_deltas();
/// ```
#[derive(Debug, Clone)]
pub struct EventFeed {
    lines: VecDeque<FeedLine>,
    capacity: usize,
    /// Show events.
    pub show_events: bool,
    /// Show activities.
    pub show_activities: bool,
}

impl Default for EventFeed {
    fn default() -> Self {
        EventFeed::new(500)
    }
}

impl EventFeed {
    /// A feed keeping the newest `capacity` lines.
    pub fn new(capacity: usize) -> EventFeed {
        EventFeed { lines: VecDeque::new(), capacity: capacity.max(1), show_events: true, show_activities: true }
    }

    /// Adds a line (the oldest goes when full).
    pub fn push(&mut self, line: FeedLine) {
        if self.lines.len() == self.capacity {
            self.lines.pop_back();
        }
        self.lines.push_front(line);
    }

    /// Adds an event.
    pub fn push_event(&mut self, e: &Event) {
        self.push(FeedLine::event(e));
    }

    /// Adds an activity.
    pub fn push_activity(&mut self, a: &Activity) {
        self.push(FeedLine::activity(a));
    }

    /// Adds an update's events, then its activities.
    pub fn push_update(&mut self, u: &Update) {
        u.events.iter().for_each(|e| self.push_event(e));
        u.activities.iter().for_each(|a| self.push_activity(a));
    }

    /// Empties the feed.
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// The lines shown, newest first.
    pub fn lines(&self) -> impl Iterator<Item = &FeedLine> {
        self.lines.iter().filter(|l| if l.activity { self.show_activities } else { self.show_events })
    }

    /// Lines kept (shown or not).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// No lines.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Paints the toggles and the list; returns the stone of the hovered line.
    pub fn show(&mut self, ui: &mut Ui) -> Option<i64> {
        let t = Tokens::get(ui.ctx());
        let style = IconButtonStyle::from_tokens(&t);
        ui.horizontal(|ui| {
            ui.label(crate::craft::theme::caption("Events"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let size = vec2(22.0, 22.0);
                if IconButton::new("Show activities", size, &style)
                    .selected(self.show_activities)
                    .show(ui, paint_bars)
                    .clicked()
                {
                    self.show_activities = !self.show_activities;
                }
                if IconButton::new("Show events", size, &style).selected(self.show_events).show(ui, paint_dot).clicked()
                {
                    self.show_events = !self.show_events;
                }
            });
        });
        ui.separator();
        let mut hovered = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.lines().next().is_none() {
                ui.label(RichText::new("Nothing yet").color(t.text_dim));
            }
            for line in self.lines() {
                let row = ui.horizontal(|ui| {
                    let (chip, _) = ui.allocate_exact_size(vec2(4.0, 16.0), Sense::hover());
                    ui.painter().rect_filled(chip, 2.0, hand_color(line.hand.as_ref()));
                    let text = RichText::new(&line.text).color(if line.activity { t.text_strong } else { t.text });
                    ui.add(egui::Label::new(if line.activity { text } else { text.small() }).wrap());
                });
                if row.response.contains_pointer() {
                    hovered = line.stone;
                }
            }
        });
        hovered
    }
}

/// The events toggle: a dot.
fn paint_dot(ui: &Ui, rect: Rect, color: Color32) {
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// The activities toggle: three bars.
fn paint_bars(ui: &Ui, rect: Rect, color: Color32) {
    for i in -1..=1 {
        let y = rect.center().y + i as f32 * 4.0;
        ui.painter()
            .line_segment([egui::pos2(rect.min.x + 6.0, y), egui::pos2(rect.max.x - 6.0, y)], Stroke::new(1.5, color));
    }
}
