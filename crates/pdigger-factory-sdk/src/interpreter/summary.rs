//! The summary of a replay or of a live interpreter so far (spec 4.4).

use serde::{Deserialize, Serialize};

use super::text::{clock, fmt};
use super::trays::TrayCounts;
use crate::models::{EventKind, HandStateKind};

/// Event counts by type (`other` for types this SDK does not know).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct EventCounts {
    pub new: u64,
    pub confirm: u64,
    pub freeze: u64,
    pub push: u64,
    pub lift: u64,
    pub hinge: u64,
    pub carry: u64,
    pub place: u64,
    pub drop: u64,
    pub gone: u64,
    pub delete: u64,
    pub split: u64,
    #[serde(default, skip_serializing_if = "zero_count")]
    pub sheet: u64,
    pub other: u64,
}

fn zero_count(value: &u64) -> bool {
    *value == 0
}

impl EventCounts {
    /// The counter of `kind`.
    pub fn get_mut(&mut self, kind: &EventKind) -> &mut u64 {
        match kind {
            EventKind::New => &mut self.new,
            EventKind::Confirm => &mut self.confirm,
            EventKind::Freeze => &mut self.freeze,
            EventKind::Push => &mut self.push,
            EventKind::Lift => &mut self.lift,
            EventKind::Hinge => &mut self.hinge,
            EventKind::Carry => &mut self.carry,
            EventKind::Place => &mut self.place,
            EventKind::Drop => &mut self.drop,
            EventKind::Gone => &mut self.gone,
            EventKind::Delete => &mut self.delete,
            EventKind::Sheet => &mut self.sheet,
            EventKind::Split => &mut self.split,
            EventKind::Other(_) => &mut self.other,
        }
    }

    fn line(&self) -> String {
        let pairs = [
            ("new", self.new),
            ("confirm", self.confirm),
            ("freeze", self.freeze),
            ("push", self.push),
            ("lift", self.lift),
            ("hinge", self.hinge),
            ("carry", self.carry),
            ("place", self.place),
            ("drop", self.drop),
            ("gone", self.gone),
            ("delete", self.delete),
            ("split", self.split),
            ("sheet", self.sheet),
            ("other", self.other),
        ];
        pairs
            .iter()
            .filter(|(k, v)| *k != "sheet" || *v > 0)
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Move counts.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MoveCounts {
    /// All moves.
    pub total: u64,
    /// Put down on the tray.
    pub placed: u64,
    /// Dropped.
    pub dropped: u64,
    /// Taken out of view.
    pub carried_away: u64,
    /// Removed by the world.
    pub removed: u64,
    /// Still open at a session change or the end.
    pub interrupted: u64,
    /// Joined mid-move.
    pub partial: u64,
    /// Carried.
    pub carried: u64,
    /// Mean duration of closed moves that are neither partial nor interrupted, seconds.
    pub mean_duration: Option<f64>,
    /// Mean distance of moves with a distance, template units.
    pub mean_distance: Option<f64>,
}

/// Push burst counts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PushCounts {
    /// Bursts.
    pub total: u64,
    /// Push events in them.
    pub pushes: u64,
    /// Bursts of the left hand.
    pub left: u64,
    /// Bursts of the right hand.
    pub right: u64,
    /// Bursts of both hands.
    pub both: u64,
}

/// Stone counts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StoneCounts {
    /// Stones in the last world.
    pub last: Option<u64>,
    /// The most stones in a world.
    pub max: u64,
    /// `stats.confirmed` of the last world.
    pub confirmed_last: Option<i64>,
}

/// Seconds a hand spent in each state.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct HandTimes {
    pub free: f64,
    pub touch: f64,
    pub grasp: f64,
    pub hinge: f64,
    pub carry: f64,
}

impl HandTimes {
    /// The times of the milliseconds summed per state (spec 4.5: each frame with a world adds its step).
    pub(crate) fn from_ms(ms: &[u64; 5]) -> HandTimes {
        let s = |i: usize| super::text::round3(ms[i] as f64 / 1000.0);
        HandTimes { free: s(0), touch: s(1), grasp: s(2), hinge: s(3), carry: s(4) }
    }

    /// The index of a known state in `free, touch, grasp, hinge, carry`.
    pub(crate) fn index(state: &HandStateKind) -> Option<usize> {
        match state {
            HandStateKind::Free => Some(0),
            HandStateKind::Touch => Some(1),
            HandStateKind::Grasp => Some(2),
            HandStateKind::Hinge => Some(3),
            HandStateKind::Carry => Some(4),
            HandStateKind::Other(_) => None,
        }
    }

    fn line(&self, side: &str) -> String {
        let pairs = [
            ("free", self.free),
            ("touch", self.touch),
            ("grasp", self.grasp),
            ("hinge", self.hinge),
            ("carry", self.carry),
        ];
        format!(
            "{side} hand: {}",
            pairs.iter().map(|(k, v)| format!("{k} {}", clock(*v))).collect::<Vec<_>>().join(", ")
        )
    }
}

/// Hand times of both hands.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HandsTimes {
    /// Left hand.
    pub left: HandTimes,
    /// Right hand.
    pub right: HandTimes,
}

/// The summary: the JSON form has exactly the keys of spec 4.4; [`Summary::describe`] is its
/// text.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Summary {
    /// Frames fed (revisions are counted in `revisions`).
    pub frames: u64,
    /// Frames with a world.
    pub frames_with_world: u64,
    /// Frames whose world was missing.
    pub missing_world: u64,
    /// World revisions.
    pub revisions: u64,
    /// Sessions.
    pub sessions: u64,
    /// The first grid frame.
    pub first_g: Option<i64>,
    /// The last grid frame.
    pub last_g: Option<i64>,
    /// The first media time.
    pub first_t: Option<f64>,
    /// The last media time.
    pub last_t: Option<f64>,
    /// Sum over sessions of last t − first t.
    pub media_seconds: f64,
    /// Jumps of `g` by more than 1 within a session.
    pub gaps: u64,
    /// Events by type.
    pub events: EventCounts,
    /// Moves.
    pub moves: MoveCounts,
    /// Push bursts.
    pub push_bursts: PushCounts,
    /// Stones.
    pub stones: StoneCounts,
    /// Trays and tray stones (spec 4.6); zeros in a summary made before trays were counted.
    #[serde(default)]
    pub trays: TrayCounts,
    /// Hand times.
    pub hands: HandsTimes,
}

fn or_dash<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "-".to_string(), |v| v.to_string())
}

impl Summary {
    /// The nine lines of spec 4.4, each ending in `\n`.
    pub fn describe(&self) -> String {
        let m = &self.moves;
        let p = &self.push_bursts;
        let mean_d = or_dash(m.mean_duration.map(|x| fmt(x, 1)));
        let mean_u = or_dash(m.mean_distance.map(|x| fmt(x, 2)));
        let paper = if self.trays.sheets > 0 || self.trays.under_sheet > 0 {
            format!(", sheets laid {}, under a sheet {}", self.trays.sheets, self.trays.under_sheet)
        } else {
            String::new()
        };
        let lines = [
            format!(
                "frames: {} (with world {}, without {}, revisions {})",
                self.frames, self.frames_with_world, self.missing_world, self.revisions
            ),
            format!("media: {}, sessions {}, gaps {}", clock(self.media_seconds), self.sessions, self.gaps),
            format!(
                "stones: last {}, max {}, confirmed {}",
                or_dash(self.stones.last),
                self.stones.max,
                or_dash(self.stones.confirmed_last)
            ),
            format!(
                "moves: {} (placed {}, dropped {}, carried away {}, removed {}, interrupted {}, partial {}, carried {}), mean {mean_d} s, {mean_u} units",
                m.total, m.placed, m.dropped, m.carried_away, m.removed, m.interrupted, m.partial, m.carried
            ),
            format!(
                "push bursts: {} (left {}, right {}, both {}), pushes {}",
                p.total, p.left, p.right, p.both, p.pushes
            ),
            format!("events: {}", self.events.line()),
            format!(
                "trays: {} (arrived {}), stones at arrival {}, added {} (+{} unclear), removed {} (+{} unclear), not sure {}{paper}",
                self.trays.total,
                self.trays.arrived,
                self.trays.stones,
                self.trays.added,
                self.trays.added_unclear,
                self.trays.removed,
                self.trays.removed_unclear,
                self.trays.unsure
            ),
            self.hands.left.line("left"),
            self.hands.right.line("right"),
        ];
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    /// The JSON form (keys of spec 4.4).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::WireEnum;

    #[test]
    fn empty_summary_text_and_json() {
        let s = Summary::default();
        let text = s.describe();
        assert_eq!(text.lines().count(), 9);
        assert!(text.contains(
            "trays: 0 (arrived 0), stones at arrival 0, added 0 (+0 unclear), removed 0 (+0 unclear), not sure 0\n"
        ));
        assert!(text.contains("stones: last -, max 0, confirmed -"));
        assert!(text.contains("mean - s, - units"));
        assert!(text.ends_with("carry 0:00.0\n"));
        assert_eq!(s.to_string(), text);
        let json = s.to_json();
        assert_eq!(json["moves"]["mean_duration"], serde_json::Value::Null);
        assert_eq!(json["hands"]["left"]["free"], serde_json::json!(0.0));
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 17);
        assert_eq!(json["trays"]["removed_unclear"], serde_json::json!(0));
        let mut old = json.clone();
        old.as_object_mut().unwrap().remove("trays"); // a summary made before trays were counted
        assert_eq!(serde_json::from_value::<Summary>(old).unwrap(), s);
    }

    #[test]
    fn counters() {
        let mut e = EventCounts::default();
        for kind in EventKind::known() {
            *e.get_mut(kind) += 1;
        }
        *e.get_mut(&EventKind::Other("melt".into())) += 2;
        assert_eq!(e.other, 2);
        assert!(e.line().starts_with("new 1, confirm 1"));
        let idx: Vec<Option<usize>> = HandStateKind::known().iter().map(HandTimes::index).collect();
        assert_eq!(idx, vec![Some(0), Some(1), Some(2), Some(3), Some(4)]);
        assert_eq!(HandTimes::index(&HandStateKind::Other("wave".into())), None);
        let t = HandTimes::from_ms(&[200, 400, 600, 800, 1000]);
        assert_eq!(t.carry, 1.0);
        let s = Summary {
            moves: MoveCounts { mean_duration: Some(4.05), mean_distance: Some(0.385), ..MoveCounts::default() },
            ..Summary::default()
        };
        assert!(s.describe().contains("mean 4.1 s, 0.39 units"));
    }
}
