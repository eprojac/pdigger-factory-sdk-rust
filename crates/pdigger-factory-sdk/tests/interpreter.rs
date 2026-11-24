//! The high level over a stream: the interpreter following a fake that serves each fixture
//! recording gives exactly the golden events, activities and summary; `wait_for`, state and
//! options.

mod common;

use std::time::Duration;

use futures::StreamExt;
use pdigger_factory_sdk::models::{EventKind, HandStateKind, Layer, Side};
use pdigger_factory_sdk::testing::{FakeFactory, Fault};
use pdigger_factory_sdk::{Activity, ErrorCode, Event, InterpreterCore, MoveOutcome, TrayFact, describe_speckle_txt};

use common::{complete, fixtures, normalise, recording_fake, recordings};

fn golden(name: &str, ext: &str) -> String {
    std::fs::read_to_string(fixtures().join("golden").join(format!("{name}.{ext}"))).unwrap()
}

#[tokio::test(start_paused = true)]
async fn following_a_stream_reproduces_the_golden_files() {
    for (name, path) in recordings() {
        let fake = FakeFactory::from_recording(&path).unwrap();
        let interpreter = fake.client().interpreter().resume(false);
        let updates: Vec<_> = interpreter.updates().map(Result::unwrap).collect().await;
        // the server ended the stream (resume off): the final update closes what is open, as a replay's
        let last = updates.last().unwrap();
        assert!(last.is_final && last.frame.is_none(), "{name}: the stream ends with the final update");
        assert!(updates.iter().all(|u| !u.is_duplicate), "{name}: duplicates are skipped");
        let activities: Vec<Activity> = updates.iter().flat_map(|u| u.activities.clone()).collect();
        let facts: Vec<TrayFact> = updates.iter().flat_map(|u| u.tray_facts.clone()).collect();
        let speckle: Vec<_> = updates.iter().flat_map(|u| u.speckle.clone()).collect();
        assert_eq!(describe_speckle_txt(&speckle), golden(&name, "speckle.txt"), "{name} speckle answers over SSE");
        assert_eq!(interpreter.speckle(), speckle, "{name}: the interpreter keeps the answers");
        assert!(interpreter.end().activities.is_empty(), "{name}: nothing is left open");
        let events: Vec<Event> = updates.iter().flat_map(|u| u.events.clone()).collect();
        let text = |lines: Vec<String>| lines.into_iter().map(|l| l + "\n").collect::<String>();
        assert_eq!(
            text(events.iter().map(Event::describe).collect()),
            golden(&name, "events.txt"),
            "{name} events over SSE"
        );
        assert_eq!(
            text(activities.iter().map(Activity::describe).collect()),
            golden(&name, "activities.txt"),
            "{name} activities over SSE"
        );
        assert_eq!(
            text(facts.iter().map(TrayFact::describe).collect()),
            golden(&name, "trays.txt"),
            "{name} tray facts over SSE"
        );
        let summary = interpreter.summary();
        let expected: serde_json::Value = serde_json::from_str(&golden(&name, "summary.json")).unwrap();
        assert_eq!(normalise(summary.to_json()), normalise(expected), "{name} summary over SSE");
        assert!(events.iter().all(|e| e.wall.is_some()), "{name}: wall times from the clock");
    }
}

#[tokio::test(start_paused = true)]
async fn state_after_the_carry_of_stone_643() {
    let fake = recording_fake("dry1-0296-0306-full");
    let interpreter = fake.client().interpreter().resume(false);
    let mut updates = interpreter.updates();
    let mut lifted = None;
    while let Some(update) = updates.next().await {
        let update = update.unwrap();
        if update.events.iter().any(|e| e.kind == EventKind::Lift) {
            lifted = Some(update);
            break;
        }
    }
    let update = lifted.unwrap();
    let lift = update.events.iter().find(|e| e.kind == EventKind::Lift).unwrap();
    assert_eq!(lift.describe(), "5:01.6 Right hand lifts stone 643");
    assert_eq!(lift.to_string(), lift.describe());
    assert_eq!(lift.stone.as_ref().unwrap().id, 643);
    assert_eq!(lift.at().g, lift.g);
    let state = &update.state;
    assert_eq!(state.g, Some(update.frame.as_ref().unwrap().g));
    assert!(state.stones.contains_key(&643));
    assert_eq!(state.open_moves.len(), 1);
    assert_eq!(state.open_moves[0].stone_id, Some(643));
    assert!(state.board.is_some() && state.stats.is_some() && state.clock.is_some() && state.wall.is_some());
    assert!(state.hands.right.state.is_some());
    assert!(state.hands.left.card.is_some() || state.hands.right.card.is_some());
    assert!(state.hands.get(&Side::Both).is_none());
    assert_eq!(state.hands.get(&Side::Left), Some(&state.hands.left));
    assert_eq!(interpreter.state().open_moves.len(), 1);
    drop(updates);
    let interrupted = interpreter.finish();
    assert!(interrupted.iter().any(|a| matches!(a, Activity::Move(m) if m.outcome == MoveOutcome::Interrupted)));
}

#[tokio::test(start_paused = true)]
async fn wait_for_an_event() {
    let fake = recording_fake("dry1-0296-0306-full");
    let factory = fake.client();
    let lift = factory.interpreter().wait_for(EventKind::Lift, Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(lift.stone_id, Some(643));
    let place = factory
        .interpreter()
        .resume(false)
        .wait_for_match(|e| e.kind == EventKind::Place && e.hand == Some(Side::Right), Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(place.unwrap().stone_id, Some(643));
    let never =
        factory.interpreter().resume(false).wait_for(EventKind::Split, Duration::from_secs(60)).await.unwrap_err();
    assert_eq!(never.code, ErrorCode::StreamClosed);
    fake.set_delay(|_, _| Some(Duration::from_secs(10)));
    let late = factory.interpreter().wait_for(EventKind::Lift, Duration::from_secs(5)).await.unwrap();
    assert!(late.is_none(), "timed out");
}

#[tokio::test(start_paused = true)]
async fn wait_for_waits_on_past_an_event_that_does_not_decode() {
    // spec 2.3 rule 9: the stream goes on past an event that does not decode, and so does wait_for
    let world = |g: i64, events: serde_json::Value| {
        complete(serde_json::json!({"cursor": g, "g": g,
            "world": {"stones": [{"id": 7, "solid": {"pos": [0.0, 0.0, 0.0]}}], "events": events}}))
    };
    let fake = FakeFactory::from_values(vec![
        world(1, serde_json::json!([{"type": "new", "id": 7}])),
        world(2, serde_json::json!([{"type": "lift", "id": 7, "side": "left"}])),
        world(3, serde_json::json!([{"type": "confirm", "id": 7}])),
    ]);
    let bad = || serde_json::json!({"g": "x"});
    fake.inject("streamFrames", "frame", bad(), 2);
    let interpreter = fake.client().interpreter().resume(false);
    let confirm = interpreter.wait_for(EventKind::Confirm, Duration::from_secs(5)).await.unwrap().unwrap();
    assert_eq!(confirm.g, 3);
    // a later wait_for goes on where the last one stopped: it reads that event again (spec 2.4), and past it
    fake.inject("streamFrames", "frame", bad(), 2);
    fake.inject("streamFrames", "frame", bad(), 0);
    let interpreter = fake.client().interpreter().resume(false);
    assert_eq!(interpreter.wait_for(EventKind::Lift, Duration::from_secs(5)).await.unwrap().unwrap().g, 2);
    assert_eq!(interpreter.wait_for(EventKind::Confirm, Duration::from_secs(5)).await.unwrap().unwrap().g, 3);
    // an error that ends the stream ends the wait
    fake.fail("streamFrames", Fault::status(400, "invalid_param"));
    let interpreter = fake.client().interpreter().resume(false);
    let error = interpreter.wait_for(EventKind::Confirm, Duration::from_secs(5)).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidParam);
}

#[tokio::test(start_paused = true)]
async fn events_and_activities_streams() {
    let fake = recording_fake("dry1-0296-0306-full");
    let factory = fake.client();
    let events: Vec<_> = factory.interpreter().resume(false).events().map(Result::unwrap).collect().await;
    assert_eq!(events.len(), golden("dry1-0296-0306-full", "events.txt").lines().count());
    let activities: Vec<_> = factory.interpreter().resume(false).activities().map(Result::unwrap).collect().await;
    let a = activities.iter().find(|a| a.kind() == pdigger_factory_sdk::ActivityKind::Move).unwrap();
    assert_eq!(a.describe(), "5:01.6 Right hand moves stone 643 1.09 units in 1.2 s");
    assert_eq!(
        (a.kind().to_string(), a.hand(), a.session(), a.video()),
        ("move".to_string(), Some(&Side::Right), events[0].session, "dry1")
    );
    assert!(a.duration() > 1.1 && a.end().t > a.start().t);
    assert_eq!(a.to_string(), a.describe());
    let bad = factory.interpreter().layers([Layer::Other("foo".into())]);
    let errors: Vec<_> = bad.events().collect().await;
    assert_eq!(errors[0].as_ref().unwrap_err().code, ErrorCode::UnknownLayer);
    let errors: Vec<_> = bad.activities().collect().await;
    assert!(errors[0].is_err());
}

#[tokio::test(start_paused = true)]
async fn options_reach_the_stream() {
    let fake = recording_fake("dry1-0296-0306-full");
    fake.fail("getSession", Fault::status(400, "invalid_param"));
    let interpreter =
        fake.client().interpreter().layers([Layer::World]).observe(true).backfill(false).after(1553).resume(false);
    assert_eq!(interpreter.options().layers, vec![Layer::World]);
    let updates: Vec<_> = interpreter.updates().map(Result::unwrap).collect().await;
    assert_eq!(updates[0].frame.as_ref().unwrap().g, 1501);
    assert!(updates.iter().all(|u| u.state.hands.left.card.is_none()), "no cards layer");
    let request = fake.requests().into_iter().find(|r| r.url.contains("/frames/stream")).unwrap();
    assert!(
        request.url.contains("layers=world") && request.url.contains("after=1553") && request.url.contains("observe=1")
    );
    assert!(
        updates[0].state.clock.is_none() && updates[0].state.wall.is_none(),
        "no clock: getSession failed and the stream started after the clock line"
    );
}

#[test]
fn the_core_handles_revisions_session_changes_and_partial_moves() {
    let mut core = InterpreterCore::new();
    let frame = |session: i64, g: i64, rev: i64, events: serde_json::Value| -> pdigger_factory_sdk::Frame {
        serde_json::from_value(serde_json::json!({"cursor": g, "session": session, "video": "v", "g": g, "t": g as f64 * 0.2,
            "world": {"rev": rev, "events": events, "stones": [{"id": 1, "solid": {"pos": [0.0, 0.0, 0.0]}}, {"id": 2, "solid": {"pos": [3.0, 4.0, 0.0]}}],
                      "hands": {"left": {"state": "grasp"}, "right": {"state": "wave"}}, "stats": {"confirmed": 2}},
            "cards": {"left": {"title": "Holding a stone"}, "right": {"title": ""}}}))
        .unwrap()
    };
    core.set_clock(
        serde_json::from_value(
            serde_json::json!({"session": 1, "rate": 0.5, "anchor_wall": 100.0, "anchor_media": 0.0}),
        )
        .unwrap(),
    );
    assert_eq!(core.clock().unwrap().rate, 0.5);
    assert_eq!(core.wall(1, 1.0), Some(102.0));
    assert_eq!(core.wall(2, 1.0), None);
    let mut held = InterpreterCore::new();
    let state = |session: i64, revision: i64, playing: bool, anchor_wall: f64| -> pdigger_factory_sdk::models::State {
        serde_json::from_value(serde_json::json!({"session": session, "revision": revision, "playing": playing,
            "rate": 0.5, "anchor_wall": anchor_wall, "anchor_media": 0.0, "hold_until": anchor_wall + 1.5}))
        .unwrap()
    };
    held.set_clock(state(1, 1, true, 100.0));
    assert_eq!(held.wall(1, 1.0), Some(103.5), "playing: from the end of the hold, as SessionClock::wall_of");
    held.set_clock(state(1, 2, false, 100.0));
    assert_eq!(held.wall(1, 1.0), Some(102.0), "paused: mapped back from the pause");
    held.set_clock(state(1, 1, true, 500.0));
    assert_eq!(held.wall(1, 1.0), Some(102.0), "an older state of the session is ignored");
    held.set_clock(state(2, 3, true, 200.0));
    assert_eq!((held.wall(1, 1.0), held.wall(2, 1.0)), (Some(102.0), Some(203.5)), "each session its own state");
    assert_eq!(held.clock().unwrap().session, 2);
    for session in 3..10 {
        held.set_clock(state(session, 1, true, 300.0));
    }
    assert_eq!((held.wall(1, 1.0), held.wall(2, 1.0).is_some()), (None, true), "the last 8 sessions are kept");
    let u = core.feed(frame(1, 10, 0, serde_json::json!([{"type": "lift", "id": 1, "side": "left"}, {"type": "hinge", "id": 1, "side": "right"}, {"type": "carry", "id": 1}])));
    assert_eq!(u.events.len(), 3);
    assert_eq!(u.state.hands.left.card.as_deref(), Some("Holding a stone"));
    assert_eq!(u.state.hands.right.card, None);
    assert_eq!(u.state.hands.left.state, Some(HandStateKind::Grasp));
    let dup = core.feed(frame(1, 10, 0, serde_json::json!([{"type": "place", "id": 1, "side": "left"}])));
    assert!(dup.events.is_empty() && !dup.is_revision && dup.is_duplicate);
    let rev = core.feed(frame(1, 10, 2, serde_json::json!([{"type": "lift", "id": 1, "side": "left"}, {"type": "place", "id": 1, "side": "left"}, {"type": "delete", "id": 1}])));
    assert!(rev.is_revision);
    assert_eq!(rev.events.len(), 2, "only the new events of a revision");
    let Activity::Move(m) = &rev.activities[0] else { panic!("a move") };
    assert_eq!(
        (m.outcome.clone(), m.hand.clone(), m.tipped, m.carried, m.distance),
        (MoveOutcome::Placed, Some(Side::Both), true, true, Some(0.0))
    );
    assert_eq!(m.start.wall, Some(104.0));
    let gap = core.feed(frame(1, 13, 0, serde_json::json!([{"type": "drop", "id": 2, "side": "right"}, {"type": "gone", "id": 3}, {"type": "delete", "id": 4, "why": "absent"}, {"type": "split", "id": 2, "into": [2, 5]}, {"type": "melt", "id": 2}])));
    assert_eq!(gap.activities.len(), 2, "a drop and a gone without a lift are partial moves");
    assert!(gap.activities.iter().all(|a| matches!(a, Activity::Move(m) if m.partial)));
    assert_eq!(gap.activities[0].describe(), "0:02.6 Right hand drops stone 2 after 0.0 s (joined mid-move)");
    assert_eq!(gap.activities[1].describe(), "0:02.6 A hand takes stone 3 away (0.0 s) (joined mid-move)");
    let older = core.feed(frame(1, 12, 5, serde_json::json!([])));
    assert!(older.events.is_empty() && !older.is_revision, "a frame never seen is not a revision");
    let pushes = core.feed(frame(1, 14, 0, serde_json::json!([{"type": "push", "id": 2, "side": "right"}, {"type": "push", "id": 1, "side": "right"}, {"type": "push", "id": 1, "side": "x"}, {"type": "push", "id": 1, "side": "left", "why": "course"}, {"type": "push", "side": "both"}])));
    assert!(pushes.activities.is_empty());
    let late = core.feed(frame(
        1,
        30,
        0,
        serde_json::json!([{"type": "push", "id": 2, "side": "right"}, {"type": "lift", "id": 2, "side": "left"}]),
    ));
    let bursts: Vec<String> = late.activities.iter().map(Activity::describe).collect();
    assert_eq!(
        bursts,
        ["0:02.8 Both hands push stone ? (1 push)", "0:02.8 Right hand pushes stones 1 and 2 (2 pushes in 0.0 s)"]
    );
    let other = core.feed(frame(2, 1, 0, serde_json::json!([])));
    let closed: Vec<String> = other.activities.iter().map(Activity::describe).collect();
    assert_eq!(
        closed,
        ["0:06.0 Left hand is still holding stone 2 (0.0 s so far)", "0:06.0 Right hand pushes stone 2 (1 push)"]
    );
    core.feed(frame(2, 2, 0, serde_json::json!([])));
    let no_world: pdigger_factory_sdk::Frame =
        serde_json::from_value(serde_json::json!({"cursor": 3, "session": 2, "g": 3, "t": 0.6})).unwrap();
    let u = core.feed(no_world);
    assert!(u.events.is_empty());
    let finished = core.end();
    assert!(finished.is_final && finished.frame.is_none() && finished.activities.is_empty());
    let summary = core.summary();
    assert_eq!(
        (summary.sessions, summary.frames, summary.revisions, summary.missing_world, summary.gaps),
        (2, 7, 1, 1, 2)
    );
    assert_eq!(summary.events.other, 1);
    assert_eq!(summary.hands.left.grasp, 1.2);
    assert_eq!(summary.stones.confirmed_last, Some(2));
    assert_eq!(summary.moves.partial, 2);
    assert_eq!(summary.moves.mean_distance, Some(0.0));
    assert_eq!(summary.push_bursts.both, 1);
}

#[test]
fn activity_texts() {
    use pdigger_factory_sdk::interpreter::{PushBurst, TimeRef};
    use pdigger_factory_sdk::{Move, MoveOutcome};
    let at = |g: i64, t: f64| TimeRef { g, t, wall: None };
    let m = |outcome: MoveOutcome, carried: bool, tipped: bool, distance: Option<f64>, hand: Option<Side>| {
        Activity::Move(Move {
            stone_id: Some(72),
            hand,
            start: at(1, 1277.8),
            end: at(9, 1281.9),
            duration: 4.1,
            from: None,
            to: None,
            distance,
            carried,
            tipped,
            partial: false,
            outcome,
            session: 1,
            video: "aug28".into(),
        })
    };
    let r = Some(Side::Right);
    assert_eq!(
        m(MoveOutcome::Placed, true, false, Some(0.384), r.clone()).describe(),
        "21:17.8 Right hand moves stone 72 0.38 units in 4.1 s"
    );
    assert_eq!(
        m(MoveOutcome::Placed, true, false, None, Some(Side::Both)).describe(),
        "21:17.8 Both hands move stone 72 in 4.1 s"
    );
    assert_eq!(
        m(MoveOutcome::Placed, false, true, None, r.clone()).describe(),
        "21:17.8 Right hand tips stone 72 and sets it down (4.1 s)"
    );
    assert_eq!(
        m(MoveOutcome::Placed, false, false, None, r.clone()).describe(),
        "21:17.8 Right hand lifts stone 72 and sets it down (4.1 s)"
    );
    assert_eq!(
        m(MoveOutcome::CarriedAway, true, false, None, None).describe(),
        "21:17.8 A hand takes stone 72 away (4.1 s)"
    );
    assert_eq!(
        m(MoveOutcome::Removed, false, false, None, r.clone()).describe(),
        "21:17.8 Right hand lifts stone 72; it is removed after 4.1 s"
    );
    assert_eq!(
        m(MoveOutcome::Interrupted, false, false, None, Some(Side::Both)).describe(),
        "21:17.8 Both hands are still holding stone 72 (4.1 s so far)"
    );
    let burst = Activity::Push(PushBurst {
        hand: r,
        start: at(1, 20.5),
        end: at(5, 21.7),
        duration: 1.2,
        stones: (1..=10).collect(),
        pushes: 11,
        session: 1,
        video: "dry1".into(),
    });
    assert_eq!(burst.describe(), "0:20.5 Right hand pushes stones 1, 2, 3, 4, 5, 6 and 4 more (11 pushes in 1.2 s)");
    assert_eq!(burst.sort_key(), (5, 1, 1, Some(1), "right"));
    let json = serde_json::to_value(&burst).unwrap();
    assert_eq!(json["kind"], "push");
}

#[test]
fn the_core_edge_cases() {
    use pdigger_factory_sdk::interpreter::HandsView;
    let mut empty = InterpreterCore::new();
    assert!(empty.finish().is_empty(), "nothing was fed");
    assert_eq!(empty.summary().media_seconds, 0.0);
    let frame = |g: i64, t: f64, rev: i64, events: serde_json::Value| -> pdigger_factory_sdk::Frame {
        serde_json::from_value(serde_json::json!({"cursor": g, "session": 3, "video": "v", "g": g, "t": t,
            "world": {"rev": rev, "events": events, "stones": [{"id": 8, "solid": {"pos": [1.0, 1.0, 0.0]}}]}}))
        .unwrap()
    };
    let mut core = InterpreterCore::new();
    let u = core.feed(frame(
        1,
        0.0,
        0,
        serde_json::json!([{"type": "carry", "id": 8, "side": "left"}, {"type": "push", "id": 8, "side": "left"}]),
    ));
    assert!(u.activities.is_empty(), "a carry without a lift opens nothing");
    core.feed(frame(2, 0.5, 0, serde_json::json!([{"type": "lift", "id": 8}])));
    core.feed(frame(3, 0.6, 0, serde_json::json!([{"type": "carry", "id": 8, "side": "right"}])));
    let removed = core.feed(frame(4, 0.9, 0, serde_json::json!([{"type": "delete", "id": 8, "why": "absent"}])));
    assert_eq!(core.summary().media_seconds, 0.9, "the open session counts up to its newest frame");
    let Activity::Move(m) = &removed.activities[0] else { panic!("a move") };
    assert_eq!(
        (m.outcome.clone(), m.hand.clone(), m.carried),
        (MoveOutcome::Removed, Some(Side::Right), true),
        "the carry named the hand"
    );
    let late = core.feed(frame(2, 5.0, 3, serde_json::json!([{"type": "push", "id": 8, "side": "left"}])));
    assert!(late.is_revision);
    let Activity::Push(burst) = &late.activities[0] else { panic!("the stale burst") };
    assert_eq!((burst.pushes, burst.start.g, burst.end.g), (1, 1, 1), "a revision far after a burst closes it");
    let push = &late.activities[0];
    assert_eq!(
        (push.hand(), push.start().g, push.end().g, push.duration(), push.session(), push.video()),
        (Some(&Side::Left), 1, 1, 0.0, 3, "v")
    );
    let rest = core.finish();
    assert_eq!(rest.len(), 1, "the burst the revision opened");
    let summary = core.summary();
    assert_eq!((summary.moves.removed, summary.push_bursts.left, summary.media_seconds), (1, 2, 0.9));
    let hands = HandsView::default();
    assert!(hands.get(&Side::Left).is_some() && hands.get(&Side::Right).is_some() && hands.get(&Side::Both).is_none());
}

#[tokio::test(start_paused = true)]
async fn a_later_loop_goes_on_where_the_last_one_stopped() {
    let fake = recording_fake("dry1-0296-0306-full");
    let interpreter = fake.client().interpreter().resume(false);
    let lift = interpreter.wait_for(EventKind::Lift, Duration::from_secs(60)).await.unwrap().unwrap();
    assert_eq!(lift.describe(), "5:01.6 Right hand lifts stone 643");
    let rest: Vec<String> = interpreter.events().map(|e| e.unwrap().describe()).collect().await;
    let golden = golden("dry1-0296-0306-full", "events.txt");
    let all: Vec<&str> = golden.lines().collect();
    let at = all.iter().position(|l| *l == lift.describe()).unwrap();
    assert_eq!(rest, all[at + 1..], "the rest of the lift's frame first, then the frames after it");
    let track = interpreter.track(643);
    assert!(track.len() > 1 && track.windows(2).all(|w| w[0].g < w[1].g && w[0].pos != w[1].pos));
    assert!(interpreter.track(-1).is_empty());
    // an events loop that stops after a frame's last event leaves nothing of it; one that stops inside a frame (after
    // the first of the two events at 4:56.0) leaves the rest of it to the next loop
    let fake = recording_fake("dry1-0296-0306-full");
    let interpreter = fake.client().interpreter().resume(false);
    let one: Vec<String> = interpreter.events().take(1).map(|e| e.unwrap().describe()).collect().await;
    let two: Vec<String> = interpreter.events().take(2).map(|e| e.unwrap().describe()).collect().await;
    let rest: Vec<String> = interpreter.events().map(|e| e.unwrap().describe()).collect().await;
    assert_eq!(one, ["4:55.6 stone 663 gets its final shape"]);
    assert_eq!(two, ["4:55.8 stone 683 appears", "4:56.0 stone 683 is confirmed"]);
    assert_eq!(rest[0], "4:56.0 stone 664 gets its final shape", "the rest of the 4:56.0 frame first");
    assert_eq!([one, two, rest].concat(), all, "nothing lost, nothing twice");
    // an activities loop that stops inside a frame leaves the rest of it too
    let fake = recording_fake("dry1-0296-0306-full");
    let interpreter = fake.client().interpreter().resume(false);
    let first = interpreter.activities().next().await.unwrap().unwrap();
    let mut later: Vec<String> = interpreter.activities().map(|a| a.unwrap().describe()).collect().await;
    later.insert(0, first.describe());
    let fresh = recording_fake("dry1-0296-0306-full").client().interpreter().resume(false);
    let whole: Vec<String> = fresh.activities().map(|a| a.unwrap().describe()).collect().await;
    assert_eq!(later, whole);
}

fn world_frame(g: i64, stones: Vec<serde_json::Value>) -> pdigger_factory_sdk::Frame {
    serde_json::from_value(serde_json::json!({"cursor": g, "session": 1, "video": "v", "g": g, "t": g as f64 * 0.2,
        "world": {"rev": 0, "stones": stones}}))
    .unwrap()
}

#[test]
fn tracks_and_per_frame_bookkeeping_stay_bounded() {
    let mut core = InterpreterCore::new();
    // stone 1 moves on every frame: its track keeps the newest 512 points
    for g in 0..1100 {
        core.feed(world_frame(g, vec![serde_json::json!({"id": 1, "solid": {"pos": [g as f64, 0.0, 0.0]}})]));
    }
    let track = core.track(1);
    assert_eq!((track.len(), track[0].g, track[511].g), (512, 588, 1099));
    assert_eq!(track[511].pos, [1099.0, 0.0, 0.0]);
    // a frame where it stays put adds nothing; a stone without a solid has no track
    core.feed(world_frame(
        1100,
        vec![serde_json::json!({"id": 1, "solid": {"pos": [1099.0, 0.0, 0.0]}}), serde_json::json!({"id": 2})],
    ));
    assert_eq!(core.track(1).last().unwrap().g, 1099);
    assert!(core.track(2).is_empty());
    // 4096 more stones: the least recently moved one (stone 1) loses its track
    let many: Vec<_> =
        (3..4099).map(|id| serde_json::json!({"id": id, "solid": {"pos": [0.0, 0.0, id as f64]}})).collect();
    core.feed(world_frame(1101, many));
    assert!(core.track(1).is_empty());
    assert_eq!(core.track(4098).len(), 1);
    // a new session starts with no tracks
    let mut other = world_frame(1, vec![]);
    other.session = 2;
    core.feed(other);
    assert!(core.track(4098).is_empty());
    // revisions of grid frames far behind the newest are no longer taken
    let mut core = InterpreterCore::new();
    for g in 0..8200 {
        core.feed(world_frame(g, vec![]));
    }
    let revised = |g: i64| -> pdigger_factory_sdk::Frame {
        serde_json::from_value(
            serde_json::json!({"cursor": 9000 + g, "session": 1, "video": "v", "g": g, "t": g as f64 * 0.2,
            "world": {"rev": 5, "events": [{"type": "lift", "id": 9, "side": "left"}], "stones": [{"id": 9}]}}),
        )
        .unwrap()
    };
    assert!(core.feed(revised(1)).events.is_empty(), "too far behind to be revised");
    assert!(core.feed(revised(1)).is_duplicate);
    assert_eq!(core.feed(revised(8190)).events.len(), 1);
    // the window keeps g - 4096 at the prune (at g 8192), as in the other languages
    assert_eq!(core.feed(revised(4096)).events.len(), 1);
}

#[test]
fn a_new_session_clears_the_cards_and_a_revision_keeps_them() {
    // spec 2.4: a new session resets the hands and their cards (its own cards then come with its frames); a
    // revision does not change the cards
    let frame = |session: i64, g: i64, rev: i64, card: Option<&str>| -> pdigger_factory_sdk::Frame {
        let mut f = serde_json::json!({"cursor": g, "session": session, "video": "v", "g": g, "t": g as f64 * 0.2,
            "world": {"rev": rev, "stones": []}});
        if let Some(title) = card {
            f["cards"] = serde_json::json!({"left": {"title": title}});
        }
        serde_json::from_value(f).unwrap()
    };
    let mut core = InterpreterCore::new();
    let card = |u: &pdigger_factory_sdk::Update| u.state.hands.left.card.clone();
    assert_eq!(card(&core.feed(frame(1, 1, 0, Some("Holding a stone")))).as_deref(), Some("Holding a stone"));
    let revision = core.feed(frame(1, 1, 2, Some("Pushing a stone")));
    assert!(revision.is_revision);
    assert_eq!(card(&revision).as_deref(), Some("Holding a stone"), "a revision keeps the cards");
    assert_eq!(card(&core.feed(frame(1, 2, 0, None))).as_deref(), Some("Holding a stone"), "no cards layer: kept");
    assert_eq!(card(&core.feed(frame(2, 0, 0, None))), None, "a new session clears them");
    let own = core.feed(frame(2, 1, 0, Some("Pushing a stone")));
    assert_eq!(card(&own).as_deref(), Some("Pushing a stone"), "the new session's own cards");
}

#[tokio::test(start_paused = true)]
async fn the_live_updates_skip_what_the_core_drops_and_keep_the_final_after_a_bad_event() {
    let frame = |cursor: i64, g: i64| {
        complete(serde_json::json!({"cursor": cursor, "session": 1, "video": "v", "g": g, "t": g as f64 * 0.2,
            "world": {"rev": 0, "stones": []}}))
    };
    // g 3 comes after g 4: new to the stream, so it gets past the stream's duplicate check, but older than the
    // newest frame the core has seen, so the core drops it (spec 2.4) and the updates skip it
    let fake = FakeFactory::from_values(vec![frame(1, 1), frame(2, 2), frame(3, 4), frame(4, 3), frame(5, 5)]);
    // the stream's last event does not decode: it is an Err item and the stream goes on to its end (spec 2.3, rule 9)
    fake.inject("streamFrames", "frame", serde_json::json!({"g": "x"}), usize::MAX);
    let interpreter = fake.client().interpreter().resume(false).backfill(false);
    let updates: Vec<_> = interpreter.updates().collect().await;
    let seen: Vec<String> = updates
        .iter()
        .map(|u| match u {
            Ok(u) if u.is_final => "final".to_string(),
            Ok(u) => u.frame.as_ref().unwrap().g.to_string(),
            Err(e) => format!("{:?}", e.code),
        })
        .collect();
    assert_eq!(seen, ["1", "2", "4", "5", "Decode", "final"]);
    // a loop after one whose stream ended gets nothing of it: the final update's interrupted move,
    // which the events loop did not yield, is not left behind (as in the other languages)
    let lift = complete(serde_json::json!({"cursor": 2, "session": 1, "video": "v", "g": 2, "t": 0.4,
        "world": {"rev": 0, "stones": [{"id": 7, "solid": {"pos": [0.0, 0.0, 0.0]}}],
                  "events": [{"type": "lift", "id": 7, "side": "left"}]}}));
    let fake = FakeFactory::from_values(vec![frame(1, 1), lift]);
    let interpreter = fake.client().interpreter().resume(false).backfill(false);
    let events: Vec<_> = interpreter.events().map(Result::unwrap).collect().await;
    assert_eq!(events.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(), [EventKind::Lift]);
    assert!(interpreter.activities().collect::<Vec<_>>().await.is_empty(), "nothing of the ended stream");
}

#[tokio::test(start_paused = true)]
async fn a_stream_that_fails_has_no_final_update() {
    let fake = recording_fake("dry1-0296-0306-full");
    fake.fail("streamFrames", Fault::status(400, "invalid_param"));
    let updates: Vec<_> = fake.client().interpreter().resume(false).updates().collect().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].as_ref().unwrap_err().code, ErrorCode::InvalidParam);
}
