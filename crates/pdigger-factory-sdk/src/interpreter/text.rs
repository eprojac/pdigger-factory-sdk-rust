//! The text of events, activities and the summary (spec sections 4.1, 4.2, 4.4) and the number
//! formats every SDK shares (4.5). Never the language's float formatting: it rounds ties
//! differently.

use crate::models::{EventKind, Side};

/// `fmt(x, d)`: `n = floor(x * 10^d + 0.5)`, printed with a decimal point `d` places from the
/// right; a negative `x` prints `-` and `fmt(-x, d)`.
///
/// ```
/// use pdigger_factory_sdk::interpreter::text::fmt;
/// assert_eq!(fmt(0.05, 1), "0.1");
/// assert_eq!(fmt(1.234, 2), "1.23");
/// assert_eq!(fmt(-2.5, 0), "-3");
/// ```
pub fn fmt(x: f64, d: u32) -> String {
    if x < 0.0 {
        return format!("-{}", fmt(-x, d));
    }
    let n = (x * scale(d) + 0.5).floor() as i64;
    let digits = n.to_string();
    if d == 0 {
        return digits;
    }
    let d = d as usize;
    let padded = format!("{digits:0>width$}", width = d + 1);
    let (whole, part) = padded.split_at(padded.len() - d);
    format!("{whole}.{part}")
}

fn scale(d: u32) -> f64 {
    let mut s = 1.0;
    for _ in 0..d {
        s *= 10.0;
    }
    s
}

/// `round3(x) = floor(x * 1000 + 0.5) / 1000`, the rounding of the JSON summary.
pub fn round3(x: f64) -> f64 {
    (x * 1000.0 + 0.5).floor() / 1000.0
}

/// The media clock `M:SS.d` below one hour, `H:MM:SS.d` from one hour.
///
/// ```
/// use pdigger_factory_sdk::interpreter::text::clock;
/// assert_eq!(clock(5.2), "0:05.2");
/// assert_eq!(clock(1277.8), "21:17.8");
/// assert_eq!(clock(3723.4), "1:02:03.4");
/// ```
pub fn clock(t: f64) -> String {
    let n = (t * 10.0 + 0.5).floor() as i64;
    let (h, m, s, d) =
        (n.div_euclid(36000), n.div_euclid(600).rem_euclid(60), n.div_euclid(10).rem_euclid(60), n.rem_euclid(10));
    if h == 0 { format!("{m}:{s:02}.{d}") } else { format!("{h}:{m:02}:{s:02}.{d}") }
}

/// `Left hand`, `Right hand`, `Both hands`, or `A hand` without a side.
pub fn hand_word(side: Option<&Side>) -> &'static str {
    match side {
        Some(Side::Left) => "Left hand",
        Some(Side::Right) => "Right hand",
        Some(Side::Both) => "Both hands",
        _ => "A hand",
    }
}

/// The verb agreeing with the hand: the plural for `Both hands`.
pub fn verb(side: Option<&Side>, singular: &'static str) -> &'static str {
    if side != Some(&Side::Both) {
        return singular;
    }
    match singular {
        "pushes" => "push",
        "lifts" => "lift",
        "tips" => "tip",
        "carries" => "carry",
        "places" => "place",
        "drops" => "drop",
        "takes" => "take",
        "moves" => "move",
        "is" => "are",
        other => other,
    }
}

/// A stone id, `?` when missing.
pub fn id_text(id: Option<i64>) -> String {
    id.map_or_else(|| "?".to_string(), |id| id.to_string())
}

/// `3`, `3 and 5`, `3, 5 and 9`.
pub fn join_ids(ids: &[i64]) -> String {
    match ids {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")),
    }
}

/// The text of a delete's `why`.
pub fn delete_reason(why: &str) -> &str {
    match why {
        "absent" => "no longer seen",
        "overlap" => "overlapped another stone",
        "outside" => "outside the board",
        "merged" => "merged with a neighbour",
        "empty" => "its place is empty",
        "hand" => "a hand emptied its place",
        "carried-away" => "carried away",
        other => other,
    }
}

/// The parts of an event its sentence needs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EventText<'a> {
    pub kind: &'a EventKind,
    pub id: Option<i64>,
    pub side: Option<&'a Side>,
    pub why: Option<&'a str>,
    pub into: Option<&'a [i64]>,
    pub source: Option<&'a str>,
}

/// The sentence of an event (spec 4.1), without the clock.
pub(crate) fn event_sentence(e: EventText<'_>) -> String {
    let i = id_text(e.id);
    let side = e.side.filter(|s| !crate::wire::WireEnum::as_str(*s).is_empty());
    let hand = hand_word(side);
    let v = |word| verb(side, word);
    match e.kind {
        EventKind::New => {
            format!("stone {i} appears{}", if e.source == Some("image") { " (found in the image)" } else { "" })
        }
        EventKind::Confirm => format!("stone {i} is confirmed"),
        EventKind::Freeze => format!("stone {i} gets its final shape"),
        EventKind::Push => match e.why {
            Some("course") => format!("stone {i} is pushed along its row"),
            Some("reseat") => format!("stone {i} is reseated"),
            _ if side.is_some() => format!("{hand} {} stone {i}", v("pushes")),
            Some(why) if !why.is_empty() => format!("stone {i} is pushed ({why})"),
            _ => format!("stone {i} is pushed"),
        },
        EventKind::Lift => format!("{hand} {} stone {i}", v("lifts")),
        EventKind::Hinge => format!("{hand} {} stone {i} up on its edge", v("tips")),
        EventKind::Carry => format!("{hand} {} stone {i}", v("carries")),
        EventKind::Place => format!("{hand} {} stone {i}", v("places")),
        EventKind::Drop => format!("{hand} {} stone {i}", v("drops")),
        EventKind::Gone => format!("{hand} {} stone {i} out of view", v("takes")),
        EventKind::Delete => match e.why {
            None => format!("stone {i} is removed"),
            Some(why) => format!("stone {i} is removed ({})", delete_reason(why)),
        },
        EventKind::Split => match e.into {
            None | Some([]) => format!("stone {i} splits"),
            Some([_]) => format!("stone {i} is trimmed"),
            Some(into) => format!("stone {i} splits into {}", join_ids(into)),
        },
        EventKind::Other(kind) => format!("stone {i}: {kind}"),
    }
}

/// `stone 3`, `stones 3 and 5`, `stones 3, 5, 9, 11, 12, 14 and 4 more`.
pub(crate) fn stones_text(ids: &[i64]) -> String {
    match ids.len() {
        0 => "stone ?".to_string(),
        1 => format!("stone {}", ids[0]),
        2..=6 => format!("stones {}", join_ids(ids)),
        n => {
            format!("stones {} and {} more", ids[..6].iter().map(i64::to_string).collect::<Vec<_>>().join(", "), n - 6)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(fmt(0.0, 1), "0.0");
        assert_eq!(fmt(0.05, 2), "0.05");
        assert_eq!(fmt(5.0 / 10.0, 1), "0.5");
        assert_eq!(fmt(1.23, 2), "1.23");
        assert_eq!(fmt(0.25, 1), "0.3");
        assert_eq!(fmt(-0.25, 1), "-0.3");
        assert_eq!(fmt(12.0, 0), "12");
        assert_eq!(fmt(-0.0, 1), "0.0");
        assert_eq!(round3(0.0005), 0.001);
        assert_eq!(round3(1.2344), 1.234);
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(59.96), "1:00.0");
        assert_eq!(clock(3600.0), "1:00:00.0");
        assert_eq!(clock(-0.3), "-1:59:59.7");
    }

    #[test]
    fn words() {
        assert_eq!(hand_word(Some(&Side::Left)), "Left hand");
        assert_eq!(hand_word(Some(&Side::Right)), "Right hand");
        assert_eq!(hand_word(Some(&Side::Both)), "Both hands");
        assert_eq!(hand_word(Some(&Side::Other("x".into()))), "A hand");
        assert_eq!(hand_word(None), "A hand");
        let both = Some(&Side::Both);
        let verbs = ["pushes", "lifts", "tips", "carries", "places", "drops", "takes", "moves", "is", "sees"];
        assert_eq!(
            verbs.map(|w| verb(both, w)),
            ["push", "lift", "tip", "carry", "place", "drop", "take", "move", "are", "sees"]
        );
        assert_eq!(verb(Some(&Side::Left), "is"), "is");
        assert_eq!(id_text(None), "?");
        assert_eq!(join_ids(&[]), "");
        assert_eq!(join_ids(&[3, 5, 9]), "3, 5 and 9");
        assert_eq!(join_ids(&[4]), "4");
        let reasons = ["absent", "overlap", "outside", "merged", "empty", "hand", "carried-away", "zz"];
        let texts = reasons.map(delete_reason);
        assert_eq!(texts[6], "carried away");
        assert_eq!(texts[7], "zz");
        assert_eq!(stones_text(&[]), "stone ?");
        assert_eq!(stones_text(&[3, 5, 9, 11, 12, 14, 15, 16, 17, 18]), "stones 3, 5, 9, 11, 12, 14 and 4 more");
    }

    fn says(
        kind: EventKind,
        side: Option<Side>,
        why: Option<&str>,
        into: Option<&[i64]>,
        source: Option<&str>,
    ) -> String {
        event_sentence(EventText { kind: &kind, id: Some(7), side: side.as_ref(), why, into, source })
    }

    #[test]
    fn sentences() {
        let r = Some(Side::Right);
        assert_eq!(says(EventKind::New, None, None, None, Some("image")), "stone 7 appears (found in the image)");
        assert_eq!(says(EventKind::New, None, None, None, None), "stone 7 appears");
        assert_eq!(says(EventKind::Confirm, None, None, None, None), "stone 7 is confirmed");
        assert_eq!(says(EventKind::Freeze, None, None, None, None), "stone 7 gets its final shape");
        assert_eq!(says(EventKind::Push, r.clone(), Some("course"), None, None), "stone 7 is pushed along its row");
        assert_eq!(says(EventKind::Push, None, Some("reseat"), None, None), "stone 7 is reseated");
        assert_eq!(says(EventKind::Push, Some(Side::Both), None, None, None), "Both hands push stone 7");
        assert_eq!(says(EventKind::Push, None, Some("nudge"), None, None), "stone 7 is pushed (nudge)");
        assert_eq!(says(EventKind::Push, None, Some(""), None, None), "stone 7 is pushed");
        assert_eq!(says(EventKind::Push, Some(Side::Other(String::new())), None, None, None), "stone 7 is pushed");
        assert_eq!(says(EventKind::Lift, r.clone(), None, None, None), "Right hand lifts stone 7");
        assert_eq!(says(EventKind::Hinge, None, None, None, None), "A hand tips stone 7 up on its edge");
        assert_eq!(says(EventKind::Carry, r.clone(), None, None, None), "Right hand carries stone 7");
        assert_eq!(says(EventKind::Place, r.clone(), None, None, None), "Right hand places stone 7");
        assert_eq!(says(EventKind::Drop, r.clone(), None, None, None), "Right hand drops stone 7");
        assert_eq!(says(EventKind::Gone, Some(Side::Both), None, None, None), "Both hands take stone 7 out of view");
        assert_eq!(says(EventKind::Delete, None, None, None, None), "stone 7 is removed");
        assert_eq!(
            says(EventKind::Delete, None, Some("merged"), None, None),
            "stone 7 is removed (merged with a neighbour)"
        );
        assert_eq!(says(EventKind::Split, None, None, None, None), "stone 7 splits");
        assert_eq!(says(EventKind::Split, None, None, Some(&[]), None), "stone 7 splits");
        assert_eq!(says(EventKind::Split, None, None, Some(&[7]), None), "stone 7 is trimmed");
        assert_eq!(says(EventKind::Split, None, None, Some(&[7, 9, 10]), None), "stone 7 splits into 7, 9 and 10");
        assert_eq!(says(EventKind::Other("melt".into()), None, None, None, None), "stone 7: melt");
    }
}
