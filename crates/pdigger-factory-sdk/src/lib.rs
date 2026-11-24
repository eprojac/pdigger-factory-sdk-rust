//! Rust SDK of the pDigger Factory interpreter API.
//!
//! A pDigger Factory server interprets one shared session (a recording, or one of the plant's
//! live cameras) and says what happens on the tray: stones appear, hands lift, carry, place and
//! push them. This crate turns that into one report per tray (the stones taken off, put on and
//! moved, when and where, and whether that is the tray's whole account) and, underneath, typed events,
//! joined activities and plain sentences:
//!
//! ```no_run
//! use pdigger_factory_sdk::Replay;
//!
//! # fn demo() -> pdigger_factory_sdk::Result<()> {
//! for report in Replay::open("dry1.world.ndjson.gz")?.tray_reports()? {
//!     println!("{}", report.describe()); // tray 1 (2026-11-22T02:38:47.0Z), 0:22.5 to 2:24.0: ...
//! }
//! # Ok(()) }
//! ```
//!
//! The detail underneath, live from a unit:
//!
//! ```no_run
//! use futures::StreamExt;
//! use pdigger_factory_sdk::FactoryClient;
//!
//! # async fn demo() -> pdigger_factory_sdk::Result<()> {
//! let factory = FactoryClient::new("http://factory-orin.local:8120")?;
//! let mut events = factory.interpreter().events();
//! while let Some(event) = events.next().await {
//!     println!("{}", event?.describe()); // 5:01.6 Right hand lifts stone 643
//! }
//! # Ok(()) }
//! ```
//!
//! The same over a published recording, offline and synchronous:
//!
//! ```no_run
//! use pdigger_factory_sdk::Replay;
//!
//! # fn demo() -> pdigger_factory_sdk::Result<()> {
//! for activity in Replay::open("dry1.world.ndjson.gz")?.activities() {
//!     println!("{}", activity?.describe()); // 5:01.6 Right hand moves stone 643 1.09 units in 1.2 s
//! }
//! # Ok(()) }
//! ```
//!
//! # Levels
//!
//! | Level | What | Where |
//! |---|---|---|
//! | High | follow the shared session: events, activities, trays with stone ids that hold over time, state, summary, text, `wait_for` | [`Interpreter`], [`FactoryClient::session`], [`FactoryClient::sources`] |
//! | High | the same rules over a recording file or URL | [`Replay`], [`ReplayStream`] |
//! | Mid | frames over SSE with resume, backfill of late layers, revisions; long-poll; raw packets | [`Streams`] |
//! | Low | one method per HTTP operation, typed models and errors; decoders of maps, depth, buffers | [`Api`], [`models`], [`codec`] |
//! | Tooling | a fake server for your own unit tests (feature `testing`) | `testing` |
//!
//! Everything that talks to the server is `async` (tokio); streams are [`futures::Stream`]s and
//! are cancelled by dropping them, which closes their connection. The interpreter's rules live in
//! the synchronous [`InterpreterCore`], so they run and test without I/O.
//!
//! The controls ([`Session::watch`], [`Session::seek`], [`Session::pause`], ...) move the one
//! shared session **for every viewer**; [`Session::dry_run`] validates them without moving
//! anything.
//!
//! API reference of the server: <https://factory.pdigger.com/developers/api/>; published
//! recordings: <https://factory.pdigger.com/developers/data/>.

pub mod api;
pub mod client;
pub mod codec;
pub mod error;
pub mod interpreter;
pub mod models;
pub mod replay;
pub mod session;
mod shape;
pub mod sse;
pub mod streams;
#[cfg(feature = "testing")]
pub mod testing;
pub mod transport;
mod wire;

pub use api::Api;
pub use client::{Backoff, ClientBuilder, Clock, FactoryClient, RandomSource, SeededRandom, SystemClock, USER_AGENT};
pub use error::{Error, ErrorCode, Result};
pub use interpreter::{
    Activity, ActivityKind, Event, FrameParts, Interpreter, InterpreterCore, InterpreterState, Move, MoveOutcome,
    PushBurst, Summary, TimeRef, TrackPoint, Tray, TrayCandidate, TrayChange, TrayChangeKind, TrayEvidence, TrayFact,
    TrayFactKind, TrayOpen, TrayPoint, TrayQuestion, TrayReport, TrayStone, TrayStoneStatus, Update, describe_open_txt,
    look_pairing, look_permille, look_score, usable_look,
};
pub use interpreter::{SpeckleSide, StoneSpeckle, TraySpeckle, describe_speckle_txt};
pub use models::{Frame, Layer, Side, Source};
pub use replay::{RecordingHeader, Replay, ReplayStream};
pub use session::{Session, SessionClock};
pub use streams::{FrameItem, FramesOptions, PollOptions, StreamItem, Streams};
pub use transport::{ReqwestTransport, Transport};
pub use wire::WireEnum;
