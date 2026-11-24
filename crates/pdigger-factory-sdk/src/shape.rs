//! What every SDK checks before it takes a stream event or a recording line (spec 2.3 rule 9,
//! section 6): the data is a JSON object, a field that is null counts as missing, and each type
//! has the fields of its table, of their kinds. Further inside (a stone, an event, a hand card, the
//! other layers) the models read the data as they do.

use std::collections::HashMap;

use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::value::RawValue;

use crate::error::{Error, Result};

/// The kind of a field.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    /// A whole number, written without a fraction or an exponent (Rust's models read no `2.0`
    /// as a whole number).
    Int,
    /// A number.
    Num,
    /// A string.
    Str,
    /// A boolean.
    Bool,
    /// An object.
    Obj,
    /// An array.
    Arr,
    /// An array of objects.
    Objs,
    /// An object with the fields of another table.
    Of(Table),
}

/// Field name and kind; a name that ends in `?` may be missing.
pub(crate) type Table = &'static [(&'static str, Kind)];

use Kind::{Arr, Bool, Int, Num, Obj, Objs, Of, Str};

/// The world layer.
pub(crate) const WORLD: Table = &[
    ("schema", Str),
    ("session", Int),
    ("video", Str),
    ("g", Int),
    ("t", Num),
    ("motion", Str),
    ("logF", Num),
    ("stones", Objs),
    ("hands", Obj),
    ("events", Objs),
    ("stats", Obj),
    ("draw?", Obj),
    ("focus?", Obj),
    ("rev?", Int),
];
/// The cards layer.
pub(crate) const CARDS: Table = &[("schema", Str), ("left", Obj), ("right", Obj), ("numbers", Obj), ("stage?", Obj)];
/// A sip1 packet (the raw layer, and an event of `raw_packets`).
pub(crate) const PACKET: Table = &[
    ("schema", Str),
    ("maps", Obj),
    ("depth", Obj),
    ("boxes", Obj),
    ("person", Obj),
    ("stones", Obj),
    ("tray", Obj),
    ("hands", Obj),
    ("both", Num),
    ("session", Int),
    ("video", Str),
    ("g", Int),
    ("i", Int),
    ("t", Num),
    ("past_valid", Bool),
    ("dt_past", Num),
    ("ms", Obj),
    ("cursor?", Int),
];
/// A frame (a stream event's data or a recording line).
pub(crate) const FRAME: Table = &[
    ("cursor", Int),
    ("session", Int),
    ("video", Str),
    ("g", Int),
    ("i", Int),
    ("t", Num),
    ("past_valid", Bool),
    ("dt_past", Num),
    ("missing?", Arr),
    ("raw?", Of(PACKET)),
    ("overlay?", Obj),
    ("scene?", Obj),
    ("cards?", Of(CARDS)),
    ("world?", Of(WORLD)),
];
/// A clock event: the shared state and `server_now`.
pub(crate) const CLOCK: Table = &[
    ("session", Int),
    ("revision", Int),
    ("video", Str),
    ("playing", Bool),
    ("rate", Num),
    ("anchor_media", Num),
    ("anchor_wall", Num),
    ("hold_until", Num),
    ("changed_by", Str),
    ("media_time", Num),
    ("server_now", Num),
    ("duration?", Num),
    ("focus?", Obj),
];
/// A worker event.
pub(crate) const WORKER: Table = &[
    ("state", Str),
    ("reason", Str),
    ("fps?", Num),
    ("frame_ms?", Num),
    ("cycle_ms?", Num),
    ("gpu?", Num),
    ("session?", Int),
    ("g?", Int),
    ("errors?", Int),
    ("stage_ms?", Obj),
    ("engines?", Obj),
    ("model?", Str),
    ("video?", Str),
];
/// A gap event.
pub(crate) const GAP: Table = &[("gap", Int)];
/// A reset event.
pub(crate) const RESET: Table = &[("boot", Str), ("cursor", Int)];
/// A recording's header line (spec 6: a member the header model cannot hold is left out of it, a
/// `source` that is no catalog entry by [`SOURCE`] too, so neither is checked here).
pub(crate) const HEADER: Table = &[
    ("recording", Str),
    ("name?", Str),
    ("video?", Str),
    ("build?", Str),
    ("api?", Str),
    ("model?", Str),
    ("backend?", Str),
    ("recorded_from?", Str),
    ("recorded_at?", Str),
    ("notes?", Str),
    ("frames?", Int),
    ("g_first?", Int),
    ("g_last?", Int),
    ("layers?", Arr),
    ("redacted?", Arr),
    ("sessions?", Arr),
    ("complete?", Obj),
];

/// A header's `source`, a catalog entry (`VideoEntry`): one that does not have these fields is
/// left out of the header (spec 6).
pub(crate) const SOURCE: Table = &[
    ("id", Str),
    ("fps", Num),
    ("grid_step", Int),
    ("proxy", Str),
    ("proxy_wh", Arr),
    ("native_wh", Arr),
    ("title?", Str),
    ("subtitle?", Str),
    ("duration?", Num),
    ("frames?", Int),
    ("default_start?", Num),
    ("hold?", Num),
    ("source_wh?", Arr),
    ("crop?", Arr),
    ("profile?", Str),
    ("camera?", Obj),
    ("held_out?", Arr),
    ("stone_model?", Obj),
    ("kind?", Str),
    ("live?", Bool),
    ("seekable?", Bool),
    ("station?", Str),
];

/// The table of a side event's data; `None` for an event this SDK does not know (not checked).
pub(crate) fn side(event: &str) -> Option<Table> {
    match event {
        "clock" => Some(CLOCK),
        "worker" => Some(WORKER),
        "gap" => Some(GAP),
        "reset" => Some(RESET),
        _ => None,
    }
}

/// The fields of a JSON object, each as its text.
pub(crate) type Fields<'a> = HashMap<String, &'a RawValue>;

/// The fields of `text` when it is a JSON object, else `None`.
pub(crate) fn fields(text: &str) -> Option<Fields<'_>> {
    serde_json::from_str(text).ok()
}

/// The field `name`, unless it is missing or null (which counts as missing).
fn field<'a>(fields: &Fields<'a>, name: &str) -> Option<&'a RawValue> {
    fields.get(name).copied().filter(|value| value.get() != "null")
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Int => "a whole number",
        Num => "a number",
        Str => "a string",
        Bool => "a boolean",
        Obj | Of(_) => "an object",
        Arr => "an array",
        Objs => "an array of objects",
    }
}

/// `None` when `value` is of `kind`, else what is wrong with it.
fn misfit(value: &RawValue, kind: Kind) -> Option<String> {
    let text = value.get();
    let first = text.bytes().next().unwrap_or(b' ');
    let number = first == b'-' || first.is_ascii_digit();
    let fits = match kind {
        Int => number && !text.contains(['.', 'e', 'E']),
        Num => number,
        Str => first == b'"',
        Bool => first == b't' || first == b'f',
        Obj => first == b'{',
        Arr => first == b'[',
        Objs => serde_json::from_str::<Vec<&RawValue>>(text)
            .is_ok_and(|items| items.iter().all(|item| item.get().starts_with('{'))),
        Of(table) => return problem(text, table),
    };
    (!fits).then(|| kind_name(kind).to_string())
}

/// `None` when `fields` has those of `table`, each of its kind; else the first one that does not.
pub(crate) fn problem_of(fields: &Fields<'_>, table: Table) -> Option<String> {
    for &(key, kind) in table {
        let (name, optional) = key.strip_suffix('?').map_or((key, false), |name| (name, true));
        let Some(value) = field(fields, name) else {
            if optional {
                continue;
            }
            return Some(format!("no {name}"));
        };
        if let Some(wrong) = misfit(value, kind) {
            return Some(match kind {
                Of(_) => format!("{name}: {wrong}"),
                _ => format!("{name} is not {wrong}"),
            });
        }
    }
    None
}

/// `None` when `text` is a JSON object with the fields of `table`, each of its kind; else what is
/// wrong.
pub(crate) fn problem(text: &str, table: Table) -> Option<String> {
    match fields(text) {
        Some(fields) => problem_of(&fields, table),
        None => Some("not an object".to_string()),
    }
}

/// `None` when a recording's side line decodes: `sse` a string, `recv` a number when it is there,
/// and `data` of its event's shape (an event this SDK does not know is not checked); else why not.
pub(crate) fn side_line_problem(fields: &Fields<'_>) -> Option<String> {
    let Some(event) = field(fields, "sse").and_then(|sse| serde_json::from_str::<String>(sse.get()).ok()) else {
        return Some("sse is not a string".to_string());
    };
    if field(fields, "recv").is_some_and(|recv| misfit(recv, Num).is_some()) {
        return Some("recv is not a number".to_string());
    }
    let table = side(&event)?;
    match field(fields, "data") {
        Some(data) => problem(data.get(), table),
        None => Some("no data".to_string()),
    }
}

/// The data of a stream event (`what`: "a frame event", "a clock event", ...), checked against
/// `table` and then read by the model; one that does not decode is the event's own `decode` error,
/// which the stream yields and goes on after (spec 2.3 rule 9).
pub(crate) fn decode_event<T: DeserializeOwned>(data: &str, what: &str, table: Table) -> Result<T> {
    let fail = |why: String| Error::event_decode(format!("{what} that does not decode: {why}"));
    let Some(fields) = fields(data) else {
        return Err(fail(match serde_json::from_str::<IgnoredAny>(data) {
            Ok(_) => "not an object".to_string(),
            Err(e) => format!("not JSON: {e}"),
        }));
    };
    if let Some(why) = problem_of(&fields, table) {
        return Err(fail(why));
    }
    serde_json::from_str(data).map_err(|e| fail(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_each_kind_and_names_the_first_field_that_is_wrong() {
        const T: Table =
            &[("a", Int), ("b?", Num), ("c?", Str), ("d?", Bool), ("e?", Arr), ("f?", Objs), ("g?", Of(GAP))];
        let cases = [
            (r#"{"a": 1}"#, None),
            (r#"{"a": -1, "b": 2, "c": "x", "d": false, "e": [], "f": [{}], "g": {"gap": 3}}"#, None),
            (r#"{"a": null}"#, Some("no a")),
            (r#"{"a": 1.5}"#, Some("a is not a whole number")),
            (r#"{"a": 2e0}"#, Some("a is not a whole number")),
            (r#"{"a": 1, "b": "2"}"#, Some("b is not a number")),
            (r#"{"a": 1, "b": null, "c": 5}"#, Some("c is not a string")),
            (r#"{"a": 1, "d": 0}"#, Some("d is not a boolean")),
            (r#"{"a": 1, "e": {}}"#, Some("e is not an array")),
            (r#"{"a": 1, "f": [{}, 5]}"#, Some("f is not an array of objects")),
            (r#"{"a": 1, "f": {}}"#, Some("f is not an array of objects")),
            (r#"{"a": 1, "g": 5}"#, Some("g: not an object")),
            (r#"{"a": 1, "g": {"gap": "x"}}"#, Some("g: gap is not a whole number")),
            ("[1]", Some("not an object")),
            ("5", Some("not an object")),
        ];
        for (text, want) in cases {
            assert_eq!(problem(text, T).as_deref(), want, "{text}");
        }
        assert_eq!(problem(r#"{"gap": 1}"#, side("gap").unwrap()), None);
        assert!(side("hello").is_none());
        for event in ["clock", "worker", "reset"] {
            assert_eq!(problem("{}", side(event).unwrap()).as_deref().map(|p| p.starts_with("no ")), Some(true));
        }
    }

    #[test]
    fn checks_side_lines() {
        let check = |text: &str| side_line_problem(&fields(text).unwrap());
        assert_eq!(check(r#"{"sse": "gap", "data": {"gap": 1}, "recv": 5}"#), None);
        assert_eq!(check(r#"{"sse": "gap", "data": {"gap": 1}, "recv": null}"#), None);
        assert_eq!(check(r#"{"sse": "hello", "data": 5}"#), None, "an event this SDK does not know");
        assert_eq!(check(r#"{"sse": 5, "data": {"gap": 1}}"#).as_deref(), Some("sse is not a string"));
        assert_eq!(check(r#"{"sse": null}"#).as_deref(), Some("sse is not a string"));
        assert_eq!(
            check(r#"{"sse": "gap", "data": {"gap": 1}, "recv": "x"}"#).as_deref(),
            Some("recv is not a number")
        );
        assert_eq!(check(r#"{"sse": "gap"}"#).as_deref(), Some("no data"));
        assert_eq!(check(r#"{"sse": "gap", "data": null}"#).as_deref(), Some("no data"));
        assert_eq!(check(r#"{"sse": "gap", "data": 5}"#).as_deref(), Some("not an object"));
    }

    #[test]
    fn decodes_events_or_says_why_not() {
        let gap: Result<crate::models::GapEvent> = decode_event(r#"{"gap": 4}"#, "a gap event", GAP);
        assert_eq!(gap.unwrap().gap, 4);
        let why = |data: &str| decode_event::<crate::models::GapEvent>(data, "a gap event", GAP).unwrap_err();
        assert_eq!(why("[]").message, "a gap event that does not decode: not an object");
        assert!(why("{nope").message.starts_with("a gap event that does not decode: not JSON: "));
        assert_eq!(why(r#"{"gap": "x"}"#).message, "a gap event that does not decode: gap is not a whole number");
        let deeper = why(r#"{"gap": 99999999999999999999}"#); // a whole number the model cannot hold
        assert!(deeper.message.starts_with("a gap event that does not decode: "), "{}", deeper.message);
        assert!(deeper.is_event());
    }
}
