//! The shared session (spec 2.6, 3): its state and clock, and the controls.
//!
//! **Every control moves the one shared session for every viewer** of the server: a select or a
//! seek starts a new interpretation run for everybody. Use [`Session::dry_run`] to validate a
//! control without moving anything.
//!
//! A live camera of the plant is watched like a recording, then waited for:
//!
//! ```no_run
//! # async fn run(factory: pdigger_factory_sdk::FactoryClient) -> pdigger_factory_sdk::Result<()> {
//! let camera = factory.live().await?;
//! factory.session().watch(&camera.id).await?; // connecting
//! let state = factory.session().until_live(std::time::Duration::from_secs(30)).await?; // warmed up, live
//! let picture = factory.session().picture(pdigger_factory_sdk::api::FrameSel::Latest).await?;
//! println!("{} is live; frame g {} is a {}-byte JPEG", state.video, picture.g, picture.jpeg.len());
//! # Ok(()) }
//! ```

use std::sync::Arc;
use std::time::Duration;

use crate::api::{FrameSel, Picture};
use crate::client::{Clock, FactoryClient};
use crate::error::{Error, ErrorCode, Result};
use crate::models::{FocusRequest, LivePhase, SeekRequest, SelectRequest, SessionReply, State, VideoEntry};

/// The shared session. Get it with [`FactoryClient::session`].
#[derive(Debug, Clone)]
pub struct Session {
    client: FactoryClient,
    dry_run: bool,
}

/// The id the design gave the live camera before the plant cameras shipped (each camera now has
/// its own id, its serial): a state whose `video` is `live` counts as a camera.
pub const LIVE: &str = "live";

/// [`Session::until_live`] reads the shared state at most this often.
pub const UNTIL_LIVE_POLL: Duration = Duration::from_millis(250);

impl Session {
    pub(crate) fn new(client: FactoryClient) -> Session {
        Session { client, dry_run: false }
    }

    /// The same session handle whose controls only validate (`dry_run=1`): the server answers
    /// with the state it would have, and nothing moves.
    pub fn dry_run(&self) -> Session {
        Session { client: self.client.clone(), dry_run: true }
    }

    /// Whether the controls of this handle only validate.
    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// The shared state (clock, source, focus).
    pub async fn state(&self) -> Result<State> {
        Ok(self.client.api().get_session().await?.state)
    }

    /// The whole `getSession` reply: state, worker, viewers, the Stable sim.
    pub async fn info(&self) -> Result<SessionReply> {
        self.client.api().get_session().await
    }

    /// A [`SessionClock`] from `getSession`: maps media time to server and local wall time.
    /// Each attempt is timed on its own (a retry's wait is not part of the round trip).
    pub async fn clock(&self) -> Result<SessionClock> {
        let mut attempt = 0;
        loop {
            let t_send = self.client.now();
            let error = match self.client.api().get_session_once().await {
                Ok(reply) => {
                    return Ok(SessionClock::new(&reply, t_send, self.client.now(), self.client.clock_source()));
                }
                Err(error) => error,
            };
            if !error.is_retryable() || attempt >= self.client.retries() {
                return Err(error);
            }
            tokio::time::sleep(self.client.retry_delay(attempt, &error)).await;
            attempt += 1;
        }
    }

    /// Shows `source_id` (a recording's id or a camera's serial) to every viewer: plays first if
    /// paused (a select keeps the paused/playing state, and playing first gives the worker its
    /// hold to warm up), then selects it at its default start. Returns the new state: on a camera
    /// it is `connecting`, and [`Session::until_live`] waits out the connection and the warm-up.
    pub async fn watch(&self, source_id: &str) -> Result<State> {
        self.watch_at(source_id, None).await
    }

    /// [`Session::watch`] at media time `at`. A live camera has no time: `at` for an id the
    /// catalog lists as a camera is refused locally (`not_seekable`, field `time`) without the
    /// select.
    pub async fn watch_at(&self, source_id: &str, at: Option<f64>) -> Result<State> {
        if at.is_some() && self.is_camera(source_id).await? {
            let message = format!("{source_id} is a live camera: it has no time; leave `at` out");
            return Err(Error::new(ErrorCode::NotSeekable, message).with_field("time"));
        }
        let api = self.client.api();
        if !api.get_session().await?.state.playing {
            api.session_play(self.dry_run).await?;
        }
        let request = SelectRequest { video: source_id.to_string(), time: at };
        Ok(api.session_select(&request, self.dry_run).await?.state)
    }

    /// Whether the catalog lists `source_id` as a live camera ([`LIVE`] always is one).
    async fn is_camera(&self, source_id: &str) -> Result<bool> {
        if source_id == LIVE {
            return Ok(true);
        }
        Ok(self.client.api().list_videos().await?.get(source_id).is_some_and(VideoEntry::is_live))
    }

    /// Moves the shared playhead to media time `t` for every viewer (a new interpretation
    /// run). A live camera cannot be seeked (pause and play instead): the SDK reads the shared
    /// state first (another viewer may have switched it) and, when it shows a camera, refuses the
    /// seek locally (`not_seekable`, field `video`) without sending it. The unit refuses a seek
    /// that races another viewer's switch to a camera (409 `not_seekable`) too.
    pub async fn seek(&self, t: f64) -> Result<State> {
        let api = self.client.api();
        if api.get_session().await?.state.is_live() {
            return Err(Error::new(ErrorCode::NotSeekable, "a live camera cannot be seeked; pause and play instead")
                .with_field("video"));
        }
        Ok(api.session_seek(&SeekRequest { video: None, time: t }, self.dry_run).await?.state)
    }

    /// Pauses the shared playhead for every viewer (the paused frame's world keeps being
    /// refined and revised; on a live camera: freezes the shared view and stops the interpreter).
    /// Idempotent.
    pub async fn pause(&self) -> Result<State> {
        Ok(self.client.api().session_pause(self.dry_run).await?.state)
    }

    /// Resumes the shared playhead for every viewer (on a live camera: at the live edge, after a
    /// new warm-up; `g` jumps over the pause). Idempotent.
    pub async fn play(&self) -> Result<State> {
        Ok(self.client.api().session_play(self.dry_run).await?.state)
    }

    /// Waits until the session's camera shows live frames (`state.live.phase` is `live`) and
    /// returns that state.
    ///
    /// After [`Session::watch`] of a camera the session is `connecting` until the camera's first
    /// frame, then `warming` while the interpreter runs on its first `hold` seconds (3 on the plant
    /// cameras; nothing is released meanwhile), then `live`; after [`Session::play`], and when a
    /// playing camera sent no frame for 5 s, it is `connecting` again until a new warm-up ends.
    /// This reads the shared state at most every [`UNTIL_LIVE_POLL`], waiting on the client's clock
    /// ([`Clock::sleep`]), and sends no control; on a recording, and on a paused camera that was
    /// live (a pause keeps the phase), it returns the state at once. The unit interprets only while
    /// someone watches: an open stream (the interpreter's), the page, or for 15 s the address that
    /// sent a control that is not a dry run (its worker idles 20 s after the last). A watch that is
    /// not a dry run counts, so `watch` then `until_live` goes live on a unit nobody else watches,
    /// while a camera session nobody watches stays `connecting` (the worker's reason:
    /// `nobody is watching`). After `timeout` it fails with `live_timeout`, with the phase it saw
    /// last in the message (the worker's reason in [`Session::info`] says why: nobody watching, or
    /// a camera that does not answer).
    pub async fn until_live(&self, timeout: Duration) -> Result<State> {
        let clock = self.client.clock_source();
        let deadline = clock.now() + timeout.as_secs_f64();
        loop {
            let state = self.state().await?;
            let phase = match state.live_phase() {
                None | Some(LivePhase::Live) => return Ok(state),
                Some(phase) => phase,
            };
            let left = deadline - clock.now();
            if left <= 0.0 {
                let message = format!("{} is still {phase} after {} s", state.video, timeout.as_secs_f64());
                return Err(Error::new(ErrorCode::LiveTimeout, message));
            }
            clock.sleep(UNTIL_LIVE_POLL.min(Duration::try_from_secs_f64(left).unwrap_or(UNTIL_LIVE_POLL))).await;
        }
    }

    /// The camera picture live frame `g` ([`FrameSel::Latest`]: the newest) was interpreted
    /// from. Draw a frame's layers over its own picture: `session.picture(frame.g)`. The newest
    /// 64 of a camera session are kept (older and warm-up frames: `not_retained`); a recording has
    /// none (`not_found`: its picture is the proxy video).
    pub async fn picture(&self, g: impl Into<FrameSel>) -> Result<Picture> {
        self.client.api().get_frame_picture(g, None).await
    }

    /// [`Session::picture`] of frame `g` of session `session`: `session_gone` once another
    /// viewer has switched the session, rather than the picture of the new session's frame `g`.
    pub async fn picture_in(&self, g: impl Into<FrameSel>, session: i64) -> Result<Picture> {
        self.client.api().get_frame_picture(g, Some(session)).await
    }

    /// Highlights stone `stone_id` for every viewer.
    pub async fn focus(&self, stone_id: i64) -> Result<State> {
        let request = FocusRequest { stone_id, g: None, session: None };
        Ok(self.client.api().put_focus(&request, self.dry_run).await?.state)
    }

    /// Clears the highlight for every viewer.
    pub async fn clear_focus(&self) -> Result<State> {
        Ok(self.client.api().delete_focus(self.dry_run).await?.state)
    }
}

/// The shared clock: media time from server wall time, both ways, and the offset between the
/// server's wall clock and this machine's (`offset = server_recv - (t_send + t_recv) / 2`, the
/// sampler the portal uses).
#[derive(Clone)]
pub struct SessionClock {
    /// The state the clock follows.
    pub state: State,
    /// Server wall time minus local wall time, seconds.
    pub offset: f64,
    /// The round trip of the sample, seconds.
    pub rtt: f64,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for SessionClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionClock")
            .field("state", &self.state)
            .field("offset", &self.offset)
            .field("rtt", &self.rtt)
            .finish_non_exhaustive()
    }
}

impl SessionClock {
    /// A clock from a `getSession` reply sent at local `t_send` and received at `t_recv`.
    pub fn new(reply: &SessionReply, t_send: f64, t_recv: f64, clock: Arc<dyn Clock>) -> SessionClock {
        SessionClock {
            state: reply.state.clone(),
            offset: reply.server_recv - (t_send + t_recv) / 2.0,
            rtt: t_recv - t_send,
            clock,
        }
    }

    /// Follows a newer state (a stream's `clock` event).
    pub fn update(&mut self, state: State) {
        self.state = state;
    }

    /// The server's wall time now.
    pub fn server_now(&self) -> f64 {
        self.clock.now() + self.offset
    }

    /// The media time at server wall time `server_wall`: the anchor plus the time played
    /// since the anchor (the hold of a start counts as not played), within the duration.
    pub fn media_time_at(&self, server_wall: f64) -> f64 {
        let s = &self.state;
        let mut t = s.anchor_media;
        if s.playing {
            t += (server_wall - s.anchor_wall.max(s.hold_until)).max(0.0) * s.rate;
        }
        match s.duration {
            Some(d) => t.min(d),
            None => t,
        }
    }

    /// The media time now.
    pub fn media_time_now(&self) -> f64 {
        self.media_time_at(self.server_now())
    }

    /// The server wall time at which the playhead shows (or showed) media time `t`, at the
    /// current rate: from the end of the hold (`max(anchor_wall, hold_until)`) while playing,
    /// mapped back from the pause (`anchor_wall`) while paused; `None` when the rate is not
    /// above 0.
    pub fn wall_of(&self, t: f64) -> Option<f64> {
        self.state.wall_of(t)
    }

    /// The local wall time of media time `t`.
    pub fn local_wall_of(&self, t: f64) -> Option<f64> {
        self.wall_of(t).map(|w| w - self.offset)
    }

    /// Whether the clock follows a live camera.
    pub fn is_live(&self) -> bool {
        self.state.is_live()
    }
}
