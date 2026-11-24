//! [`ViewerApp`]: the four views in the Craft split layout, playing a replay or following a
//! server, and the reports of the trays that left.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use egui::{Align2, Color32, FontId, Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use futures::StreamExt;
use pdigger_factory_sdk::interpreter::text::clock;
use pdigger_factory_sdk::models::{Overlay2D, Scene3D};
use pdigger_factory_sdk::replay::RecordingHeader;
use pdigger_factory_sdk::{
    Activity, FactoryClient, Frame, Interpreter, InterpreterCore, InterpreterState, Layer, Replay, ReplayStream,
    Result, TimeRef, Tray, TrayFactKind, TrayReport, Update,
};

use crate::craft::theme::{GUTTER, caption};
use crate::craft::{IconButton, IconButtonStyle, SplitAxis, SplitSize, Tokens, split_rect};
use crate::draw::{overlay_of_frame_in, scene_of_frame};
use crate::{EventFeed, HandCards, OrbitCamera, OverlayView, ReportView, SceneView};

/// What the viewer was asked to show (its command-line argument).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewerInput {
    /// A local recording (`.ndjson` or `.ndjson.gz`).
    File(PathBuf),
    /// A recording on the web (`https://factory.pdigger.com/developers/data/dry1.full.ndjson.gz`).
    RecordingUrl(String),
    /// A server to follow (`http://factory-orin.local:8120`).
    Server(String),
}

impl ViewerInput {
    /// Classifies an argument: an `http(s)` URL ending in `.ndjson`/`.ndjson.gz` is a recording
    /// URL, any other URL a server, anything else a file.
    pub fn parse(arg: &str) -> ViewerInput {
        let recording = arg.ends_with(".ndjson") || arg.ends_with(".ndjson.gz");
        match (arg.starts_with("http://") || arg.starts_with("https://"), recording) {
            (true, true) => ViewerInput::RecordingUrl(arg.to_string()),
            (true, false) => ViewerInput::Server(arg.to_string()),
            (false, _) => ViewerInput::File(PathBuf::from(arg)),
        }
    }
}

/// The interpreter's updates for a viewer, and the future that produces them: spawn it on your
/// runtime. It ends when the stream ends, or at the first update after the receiver is dropped
/// (an idle unit sends none for a long time: [`follow_until`] ends it at once).
pub fn follow(interpreter: Interpreter) -> (Receiver<Result<Update>>, impl Future<Output = ()> + Send + 'static) {
    follow_until(interpreter, std::future::pending())
}

/// [`follow`], ending as soon as `stop` completes too (the stream to the unit closes with it).
pub fn follow_until(
    interpreter: Interpreter,
    stop: impl Future<Output = ()> + Send + 'static,
) -> (Receiver<Result<Update>>, impl Future<Output = ()> + Send + 'static) {
    let (tx, rx): (Sender<Result<Update>>, _) = channel();
    let task = async move {
        let mut updates = interpreter.updates();
        let mut stop = std::pin::pin!(stop);
        loop {
            tokio::select! {
                () = &mut stop => break,
                update = updates.next() => {
                    let Some(update) = update else { break };
                    if tx.send(update).is_err() {
                        break;
                    }
                }
            }
        }
    };
    (rx, task)
}

/// One interpreted frame: the frame, its draw lists, what it added to the feed, the state after.
#[derive(Debug, Clone)]
pub struct Step {
    /// The frame.
    pub frame: Frame,
    /// Its video-space draw list.
    pub overlay: Option<Overlay2D>,
    /// Its 3-D draw list.
    pub scene: Option<Scene3D>,
    /// Its events and the activities it completed.
    pub update: Update,
}

impl Step {
    fn of(update: Update, native_wh: Option<&[i64]>) -> Option<Step> {
        let frame = update.frame.clone()?;
        Some(Step { overlay: overlay_of_frame_in(&frame, native_wh), scene: scene_of_frame(&frame), frame, update })
    }
}

/// The layers a viewer following a unit streams: everything it draws.
pub const VIEWER_LAYERS: [Layer; 4] = [Layer::World, Layer::Cards, Layer::Scene, Layer::Overlay];

/// The video size a recording's header names (its source's `native_wh`).
fn native_of(header: Option<&RecordingHeader>) -> Option<Vec<i64>> {
    header.and_then(|h| h.source.as_ref()).map(|s| s.native_wh.clone())
}

/// Playback speeds offered.
pub const SPEEDS: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];

/// A tray point the viewer jumps to ([`ViewerApp::next_tray_point`], keys `]` and Page Down;
/// [`ViewerApp::previous_tray_point`], keys `[` and Page Up): where a tray had arrived and
/// settled, or where it was about to move out (spec 4.6 of the SDK design).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrayMark {
    /// The tray's number in its session.
    pub tray: i64,
    /// The arrival (`true`) or the leaving point (`false`).
    pub arrived: bool,
    /// The point: grid frame, media time, wall time.
    pub at: TimeRef,
    /// The held frame that shows it.
    pub index: usize,
}

/// The viewer: a top bar (time, play, step, scrub, speed) over the video-space overlay, the 3-D
/// view, the hand cards and the event feed, laid out as Craft cards. It plays a replay (all
/// frames in memory, scrubbable) or follows a server's updates (the newest
/// [`ViewerApp::LIVE_KEEP`] frames). The top bar's Reports button shows the reports instead: the
/// trays that left, and the [`ReportView`] of the one chosen. [`ViewerApp::ui`] draws it into any
/// `Ui`; the `pdigger-factory-viewer` binary puts it in a window.
pub struct ViewerApp {
    title: String,
    steps: Vec<Step>,
    index: usize,
    playing: bool,
    speed: f32,
    media: f64,
    live: Option<Receiver<Result<Update>>>,
    native_wh: Option<Vec<i64>>,
    follow_newest: bool,
    camera: Option<OrbitCamera>,
    feed: EventFeed,
    fed: usize,
    hovered: Option<i64>,
    status: Option<String>,
    /// The trays that left, oldest first, and their reports.
    trays: Vec<Tray>,
    reports: Vec<TrayReport>,
    /// The tray whose report is shown (the four views when `None`).
    report: Option<usize>,
    /// The unit followed ([`ViewerApp::open`]): its trays, as they leave.
    following: Option<Interpreter>,
    /// Held only to be dropped with the app: that stops the thread that follows a unit
    /// ([`ViewerApp::open`]).
    _stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ViewerApp {
    /// Frames a live viewer keeps.
    pub const LIVE_KEEP: usize = 3000;

    fn empty(title: &str) -> ViewerApp {
        ViewerApp {
            title: title.to_string(),
            steps: Vec::new(),
            index: 0,
            playing: false,
            speed: 1.0,
            media: 0.0,
            live: None,
            native_wh: None,
            follow_newest: true,
            camera: None,
            feed: EventFeed::default(),
            fed: 0,
            hovered: None,
            status: None,
            trays: Vec::new(),
            reports: Vec::new(),
            report: None,
            following: None,
            _stop: None,
        }
    }

    /// A viewer of these updates (a replay's): the final update's activities join the last frame.
    pub fn from_updates(title: &str, updates: impl IntoIterator<Item = Update>) -> ViewerApp {
        ViewerApp::from_updates_of(title, updates, None)
    }

    /// [`ViewerApp::from_updates`] for a source of this video size (its `native_wh`), which
    /// places the stones of frames without an overlay layer (a `world` recording).
    pub fn from_updates_of(
        title: &str,
        updates: impl IntoIterator<Item = Update>,
        native_wh: Option<Vec<i64>>,
    ) -> ViewerApp {
        let mut app = ViewerApp { native_wh, ..ViewerApp::empty(title) };
        for update in updates {
            match Step::of(update.clone(), app.native_wh.as_deref()) {
                Some(step) => app.steps.push(step),
                None => {
                    if let Some(last) = app.steps.last_mut() {
                        last.update.activities.extend(update.activities);
                        last.update.tray_facts.extend(update.tray_facts);
                    }
                }
            }
        }
        app.seek(0);
        app
    }

    /// A viewer of a replay (reads it all), sized by its header's source, with the trays that left
    /// (the newest [`TRAYS_KEPT`](pdigger_factory_sdk::interpreter::TRAYS_KEPT)) for its reports.
    pub fn from_replay(title: &str, replay: Replay) -> Result<ViewerApp> {
        let native = native_of(replay.header());
        let mut updates = replay.updates();
        let all = updates.by_ref().collect::<Result<Vec<_>>>()?;
        let mut app = ViewerApp::from_updates_of(title, all, native);
        app.set_trays(updates.core().trays());
        Ok(app)
    }

    /// A viewer following live updates (see [`follow`]).
    pub fn live(title: &str, updates: Receiver<Result<Update>>) -> ViewerApp {
        ViewerApp { live: Some(updates), ..ViewerApp::empty(title) }
    }

    /// Opens what `input` names, as `pdigger-factory-viewer` does, and returns a title and the
    /// app: a file is replayed; a recording URL is downloaded (streamed), then replayed; a unit is
    /// followed live ([`VIEWER_LAYERS`]) by a thread of its own, which ends when the app is
    /// dropped, and its trays are listed for their reports as they leave. Blocks while a recording
    /// downloads, so call it outside an async runtime.
    pub fn open(input: &ViewerInput) -> Result<(String, ViewerApp)> {
        match input {
            ViewerInput::File(path) => {
                let title =
                    path.file_name().map_or_else(|| "recording".to_string(), |n| n.to_string_lossy().into_owned());
                let app = ViewerApp::from_replay(&title, Replay::open(path)?)?;
                Ok((title, app))
            }
            ViewerInput::RecordingUrl(url) => {
                let title = url.rsplit('/').next().unwrap_or(url).to_string();
                let (native, updates, trays) = runtime().block_on(download(url))?;
                let mut app = ViewerApp::from_updates_of(&title, updates, native);
                app.set_trays(trays);
                Ok((title, app))
            }
            ViewerInput::Server(url) => {
                let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
                let interpreter = FactoryClient::new(url)?.interpreter().layers(VIEWER_LAYERS);
                let following = Some(interpreter.clone()); // the same interpreter: its trays
                let (updates, task) = follow_until(interpreter, async move {
                    let _ = stopped.await; // the app was dropped
                });
                std::thread::spawn(move || runtime().block_on(task));
                Ok((url.clone(), ViewerApp { _stop: Some(stop), following, ..ViewerApp::live(url, updates) }))
            }
        }
    }

    /// Frames held.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// No frames yet.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The frame shown (index into the held frames).
    pub fn index(&self) -> usize {
        self.index
    }

    /// The step shown.
    pub fn current(&self) -> Option<&Step> {
        self.steps.get(self.index)
    }

    /// The interpreter's state at the frame shown.
    pub fn state(&self) -> InterpreterState {
        self.current().map(|s| s.update.state.clone()).unwrap_or_default()
    }

    /// The feed as of the frame shown.
    pub fn feed(&self) -> &EventFeed {
        &self.feed
    }

    /// Playing.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Plays or pauses.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing && self.live.is_none();
        self.media = self.current().map_or(0.0, |s| s.frame.t);
    }

    /// The playback speed.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Sets the playback speed (0.05 to 16).
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.05, 16.0);
    }

    /// The last error of a live stream.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Shows frame `index` (clamped); the feed then holds what happened up to it.
    pub fn seek(&mut self, index: usize) {
        self.index = index.min(self.steps.len().saturating_sub(1));
        self.feed.clear();
        self.fed = 0;
        self.catch_up();
        self.media = self.current().map_or(0.0, |s| s.frame.t);
    }

    /// Shows the first frame at or after media time `t`.
    pub fn seek_time(&mut self, t: f64) {
        let index = self.steps.iter().position(|s| s.frame.t >= t).unwrap_or(self.steps.len());
        self.seek(index);
    }

    /// Feeds the lines of the frames up to the one shown.
    fn catch_up(&mut self) {
        let end = if self.steps.is_empty() { 0 } else { self.index + 1 };
        for step in &self.steps[self.fed.min(end)..end] {
            self.feed.push_update(&step.update);
        }
        self.fed = end;
    }

    /// Moves playback on by `dt` seconds of wall time (at the speed); stops at the end.
    pub fn advance(&mut self, dt: f64) {
        if !self.playing {
            return;
        }
        self.media += dt * f64::from(self.speed);
        while self.steps.get(self.index + 1).is_some_and(|s| s.frame.t <= self.media) {
            self.index += 1;
        }
        self.catch_up();
        if self.index + 1 >= self.steps.len() {
            self.playing = false;
        }
    }

    /// Takes what a live stream sent; keeps the newest [`ViewerApp::LIVE_KEEP`] frames.
    pub fn poll_live(&mut self) {
        let Some(rx) = &self.live else { return };
        let mut fresh = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(Ok(update)) => fresh.push(update),
                Ok(Err(error)) => self.status = Some(error.to_string()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.status.get_or_insert_with(|| "the stream ended".to_string());
                    break;
                }
            }
        }
        let left = fresh.iter().any(|u| u.tray_facts.iter().any(|f| f.kind == TrayFactKind::Leaving));
        for update in fresh {
            match Step::of(update.clone(), self.native_wh.as_deref()) {
                Some(step) => self.steps.push(step),
                // the final update (no frame: the stream ended): its activities and tray facts go to
                // the newest frame, as in a replay; to the feed now if that frame is already shown
                None => {
                    if self.fed >= self.steps.len() {
                        self.feed.push_update(&update);
                    }
                    if let Some(last) = self.steps.last_mut() {
                        last.update.activities.extend(update.activities);
                        last.update.tray_facts.extend(update.tray_facts);
                    }
                }
            }
        }
        if self.steps.len() > Self::LIVE_KEEP {
            let drop = self.steps.len() - Self::LIVE_KEEP;
            self.steps.drain(..drop);
            self.index = self.index.saturating_sub(drop);
            self.fed = self.fed.saturating_sub(drop);
        }
        if self.follow_newest && !self.steps.is_empty() {
            self.index = self.steps.len() - 1;
        }
        self.catch_up();
        // a tray left: the unit's trays, for the reports
        if let Some(interpreter) = self.following.as_ref().filter(|_| left) {
            let trays = interpreter.trays();
            self.set_trays(trays);
        }
    }

    /// The trays that left, oldest first, each with its report (spec 4.7): a replay's, those of the
    /// unit followed so far, or the ones given to [`ViewerApp::set_trays`].
    pub fn trays(&self) -> &[Tray] {
        &self.trays
    }

    /// Lists these trays in the report panel, those that left (with a report); for a viewer built
    /// from updates, `Replay::trays()` or `Interpreter::trays()` of the same frames.
    pub fn set_trays(&mut self, trays: Vec<Tray>) {
        (self.trays, self.reports) = trays.into_iter().filter_map(|t| t.report().map(|r| (t, r))).unzip();
        self.report = self.report.map(|i| i.min(self.trays.len().saturating_sub(1)));
    }

    /// Shows the report of tray `index` of [`ViewerApp::trays`] (the last one when past it) in
    /// place of the four views, or the four views again (`None`), as the top bar's Reports button
    /// and the panel's list do.
    pub fn show_report(&mut self, index: Option<usize>) {
        self.report = index.map(|i| i.min(self.trays.len().saturating_sub(1)));
    }

    /// The index of the tray whose report is shown (`None`: the four views are).
    pub fn report_index(&self) -> Option<usize> {
        self.report
    }

    /// The view of the report shown: its tray, with the worlds at its arrival and leaving points
    /// from the frames held. The leaving account still draws its stones after those frames expire. `None` when no
    /// report is shown, or no tray has left.
    pub fn report_view(&self) -> Option<ReportView> {
        let tray = self.trays.get(self.report?)?;
        let empty = Scene3D::default();
        let scene = |point: Option<TimeRef>| {
            let shows = |s: &&Step| point.is_some_and(|p| s.frame.session == tray.session && s.frame.g == p.g);
            self.steps.iter().find(shows).and_then(|s| s.scene.as_ref()).unwrap_or(&empty)
        };
        ReportView::new(tray, scene(tray.arrived), scene(tray.leaving))
    }

    /// The Reports button: the report panel, on the tray of the frame shown (the last that had
    /// arrived by it), or the four views again.
    fn toggle_reports(&mut self) {
        if self.report.take().is_some() {
            return;
        }
        let at = self.current().map(|s| (s.frame.session, s.frame.g));
        let arrived =
            |t: &Tray| at.is_some_and(|(session, g)| t.session == session && t.arrived.is_some_and(|a| a.g <= g));
        self.show_report(Some(self.trays.iter().rposition(arrived).unwrap_or(0)));
    }

    /// The tray points of the frames held, in frame order: each tray's arrival and leaving point
    /// (the leaving fact comes later than its point, when the tray has gone; a point whose frame
    /// is no longer held is left out).
    pub fn tray_marks(&self) -> Vec<TrayMark> {
        let mut marks = Vec::new();
        for fact in self.steps.iter().flat_map(|s| &s.update.tray_facts) {
            let arrived = fact.kind == TrayFactKind::Arrived;
            if !arrived && fact.kind != TrayFactKind::Leaving {
                continue;
            }
            let shows = |s: &Step| s.frame.session == fact.session && s.frame.g == fact.g;
            if let Some(index) = self.steps.iter().position(shows) {
                let at = TimeRef { g: fact.g, t: fact.t, wall: fact.wall };
                marks.push(TrayMark { tray: fact.tray, arrived, at, index });
            }
        }
        marks.sort_by_key(|m| m.index);
        marks
    }

    /// Shows the first tray point after the frame shown, and returns it (stops following a live
    /// stream); `None` (nothing moves) when there is none.
    pub fn next_tray_point(&mut self) -> Option<TrayMark> {
        let mark = self.tray_marks().into_iter().find(|m| m.index > self.index)?;
        self.follow_newest = false;
        self.seek(mark.index);
        Some(mark)
    }

    /// Shows the last tray point before the frame shown, and returns it.
    pub fn previous_tray_point(&mut self) -> Option<TrayMark> {
        let mark = self.tray_marks().into_iter().rev().find(|m| m.index < self.index)?;
        self.follow_newest = false;
        self.seek(mark.index);
        Some(mark)
    }

    /// The stone to highlight: the hovered one, else the newest activity's of the frame shown.
    pub fn highlight(&self) -> Option<i64> {
        self.hovered.or_else(|| {
            self.current().and_then(|s| {
                s.update.activities.iter().rev().find_map(|a| match a {
                    Activity::Move(m) => m.stone_id,
                    Activity::Push(p) => p.stones.first().copied(),
                })
            })
        })
    }

    /// Draws the viewer into `ui` (and moves playback on by the frame's `dt`). Keys: `[` or Page
    /// Up shows the previous tray point, `]` or Page Down the next.
    pub fn ui(&mut self, ui: &mut Ui) {
        let dt = ui.input(|i| f64::from(i.stable_dt));
        let (back, on) = ui.input(|i| {
            let pressed = |keys: [egui::Key; 2]| keys.iter().any(|k| i.key_pressed(*k));
            (
                pressed([egui::Key::OpenBracket, egui::Key::PageUp]),
                pressed([egui::Key::CloseBracket, egui::Key::PageDown]),
            )
        });
        self.poll_live();
        if back {
            self.previous_tray_point();
        }
        if on {
            self.next_tray_point();
        }
        self.advance(dt);
        if self.playing || self.live.is_some() {
            ui.ctx().request_repaint();
        }
        let t = Tokens::get(ui.ctx());
        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0.0, t.chrome);
        let Some(areas) = Areas::of(full) else { return };
        self.top_bar(ui, areas.top, &t);
        if let Some(index) = self.report {
            self.reports_ui(ui, &areas, &t, index);
            return;
        }
        let highlight = self.highlight();
        let mut hovered = None;
        panel(ui, areas.video, &t, "Video space", &mut |ui| {
            if let Some(overlay) = self.current().and_then(|s| s.overlay.as_ref()) {
                hovered =
                    OverlayView::new(overlay).labels(true).highlight(highlight).background(t.viewport).show(ui).hovered;
            } else {
                ui.label(RichText::new("No world or overlay layer yet").color(t.text_dim));
            }
        });
        let scene = self.current().and_then(|s| s.scene.clone());
        panel(ui, areas.scene, &t, "3-D", &mut |ui| match &scene {
            Some(scene) => {
                let camera = self.camera.get_or_insert_with(|| OrbitCamera::framing(scene));
                SceneView::new(scene, camera).highlight(highlight).show(ui);
            }
            None => {
                ui.label(RichText::new("No scene yet").color(t.text_dim));
            }
        });
        let state = self.state();
        panel(ui, areas.hands, &t, "Hands", &mut |ui| HandCards::new(&state).show(ui));
        let mut feed_hover = None;
        panel(ui, areas.feed, &t, "", &mut |ui| feed_hover = self.feed.show(ui));
        self.hovered = hovered.or(feed_hover);
    }

    /// The report panel under the top bar: the trays that left on the left, the report of tray
    /// `index` on the right.
    fn reports_ui(&mut self, ui: &mut Ui, areas: &Areas, t: &Tokens, index: usize) {
        let mut chosen = None;
        panel(ui, areas.trays, t, "Trays that left", &mut |ui| {
            egui::ScrollArea::vertical().id_salt("trays-that-left").auto_shrink([false, false]).show(ui, |ui| {
                if self.reports.is_empty() {
                    ui.label(RichText::new("No tray has left yet").color(t.text_dim));
                }
                for (i, report) in self.reports.iter().enumerate() {
                    if tray_row(ui, t, report, i == index).clicked() {
                        chosen = Some(i);
                    }
                }
            });
        });
        let view = self.report_view();
        let title =
            view.as_ref().map_or_else(|| "Report".to_string(), |v| format!("Report of tray {}", v.report().tray));
        panel(ui, areas.report, t, &title, &mut |ui| match &view {
            Some(view) => {
                view.show(ui);
            }
            None => {
                ui.label(RichText::new("A tray's report shows here once it has left").color(t.text_dim));
            }
        });
        if chosen.is_some() {
            self.show_report(chosen);
        }
    }

    /// The previous frame (stops following a live stream).
    pub fn step_back(&mut self) {
        self.follow_newest = false;
        self.seek(self.index.saturating_sub(1));
    }

    /// The next frame.
    pub fn step_forward(&mut self) {
        self.seek(self.index + 1);
    }

    /// Plays or pauses a replay (from the start when at the end); follows a live stream again.
    pub fn play_pause(&mut self) {
        if self.live.is_some() {
            self.follow_newest = true;
            return;
        }
        if !self.playing && self.index + 1 >= self.steps.len() {
            self.seek(0);
        }
        self.set_playing(!self.playing);
    }

    /// The next of [`SPEEDS`].
    pub fn cycle_speed(&mut self) {
        let next = SPEEDS.iter().position(|s| *s == self.speed).map_or(2, |i| (i + 1) % SPEEDS.len());
        self.set_speed(SPEEDS[next]);
    }

    /// The top bar: back, play, next and speed at fixed places (x 8, 40, 72 and 104), the scrub
    /// bar, the time, and the title on the right.
    fn top_bar(&mut self, ui: &mut Ui, rect: Rect, t: &Tokens) {
        ui.painter().rect_filled(rect, t.cr(), t.panel);
        let style = IconButtonStyle::from_tokens(t);
        let mut bar = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(8.0, 6.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        bar.spacing_mut().item_spacing.x = 6.0;
        let size = vec2(26.0, 26.0);
        if IconButton::new("Previous frame", size, &style).show(&mut bar, paint_back).clicked() {
            self.step_back();
        }
        let playing = self.playing || (self.live.is_some() && self.follow_newest);
        if IconButton::new(if playing { "Pause" } else { "Play" }, size, &style)
            .selected(playing)
            .show(&mut bar, move |ui, r, c| paint_play(ui, r, c, playing))
            .clicked()
        {
            self.play_pause();
        }
        if IconButton::new("Next frame", size, &style).show(&mut bar, paint_forward).clicked() {
            self.step_forward();
        }
        if self.live.is_some() {
            bar.add_sized(
                vec2(44.0, 26.0),
                egui::Label::new(RichText::new(if self.follow_newest { "LIVE" } else { "live" }).color(t.accent)),
            );
        } else if bar.add_sized(vec2(44.0, 26.0), egui::Button::new(format!("{}x", self.speed))).clicked() {
            self.cycle_speed();
        }
        let mut index = self.index;
        let last = self.steps.len().saturating_sub(1);
        bar.spacing_mut().slider_width = (rect.width() * 0.3).max(80.0);
        let slider = egui::Slider::new(&mut index, 0..=last.max(1)).show_value(false);
        if bar.add_enabled(last > 0, slider).changed() {
            self.follow_newest = false;
            self.seek(index);
        }
        let when = self
            .current()
            .map(|s| format!("{} · g {}", clock(s.frame.t), s.frame.g))
            .unwrap_or_else(|| "waiting for frames".to_string());
        bar.label(RichText::new(when).monospace().color(t.text_strong));
        bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |bar| {
            let reports = egui::Button::selectable(self.report.is_some(), "Reports");
            if bar.add_sized(vec2(REPORTS_W, 26.0), reports).clicked() {
                self.toggle_reports();
            }
            bar.label(caption(&self.title));
            if let Some(status) = &self.status {
                bar.label(RichText::new(status).color(t.warning));
            }
        });
    }
}

/// A runtime of its own for [`ViewerApp::open`] (two worker threads).
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a tokio runtime")
}

/// Every update of the recording at `url` and its trays: its lines fed to an interpreter core as a
/// replay feeds them (`InterpreterCore::feed_line`: the frames, duplicates skipped, the clock lines
/// for wall times, and the `gap` and `reset` lines, whose dropped frames end the trays' presence
/// questions), then the final update.
async fn download(url: &str) -> Result<(Option<Vec<i64>>, Vec<Update>, Vec<Tray>)> {
    let replay = ReplayStream::open_url(url).await?;
    let native = native_of(replay.header());
    let (mut lines, mut core, mut updates) = (replay.lines(), InterpreterCore::new(), Vec::new());
    while let Some(line) = lines.next().await {
        updates.extend(core.feed_line(line?));
    }
    updates.push(core.end());
    Ok((native, updates, core.trays()))
}

/// The width of the top bar's Reports button, and of the report panel's list of trays.
const REPORTS_W: f32 = 70.0;
const TRAY_LIST_W: f32 = 250.0;

/// The viewer's layout: the top bar, the video view on the left, the 3-D view, the hand cards
/// and the feed on the right; in their place, the report panel: the trays that left and the
/// report.
struct Areas {
    top: Rect,
    trays: Rect,
    report: Rect,
    video: Rect,
    scene: Rect,
    hands: Rect,
    feed: Rect,
}

impl Areas {
    /// The areas of `full`; `None` when it is too small for them.
    fn of(full: Rect) -> Option<Areas> {
        let rows = split_rect(full, SplitAxis::Vertical, SplitSize::FixedFirst(40.0), GUTTER, 30.0)?;
        let main = split_rect(rows.second, SplitAxis::Horizontal, SplitSize::Ratio(0.58), GUTTER, 200.0)?;
        let right = split_rect(main.second, SplitAxis::Vertical, SplitSize::Ratio(0.52), GUTTER, 120.0)?;
        let lower = split_rect(right.second, SplitAxis::Vertical, SplitSize::FixedFirst(136.0), GUTTER, 60.0)?;
        let reports =
            split_rect(rows.second, SplitAxis::Horizontal, SplitSize::FixedFirst(TRAY_LIST_W), GUTTER, 200.0)?;
        Some(Areas {
            top: rows.first,
            trays: reports.first,
            report: reports.second,
            video: main.first,
            scene: right.first,
            hands: lower.first,
            feed: lower.second,
        })
    }
}

/// A Craft card at `rect` with a caption (none when `title` is empty).
fn panel(ui: &mut Ui, rect: Rect, t: &Tokens, title: &str, body: &mut dyn FnMut(&mut Ui)) {
    ui.painter().rect_filled(rect, t.cr(), t.panel);
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(rect.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(rect);
    if !title.is_empty() {
        child.label(caption(title));
    }
    body(&mut child);
}

/// A row of the report panel's list: the tray's number and window, its counts, and whether the
/// report is complete (as `TrayReport::describe` says it), tinted when it is the one shown
/// (hovered: lighter); a click shows its report.
fn tray_row(ui: &mut Ui, t: &Tokens, report: &TrayReport, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
    let fill = if selected {
        t.row_selected
    } else if response.hovered() {
        t.row_hover
    } else {
        t.row
    };
    ui.painter().rect_filled(rect, t.cr(), fill);
    let head = format!("tray {} · {} to {}", report.tray, clock(report.arrived.t), clock(report.left.t));
    let (removed, added, moved) = (report.removed.len(), report.added.len(), report.moved.len());
    let counts = format!("{removed} removed, {added} added, {moved} moved");
    let open: Vec<String> = [(report.unclear, "unclear"), (report.unsure, "not sure")]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
    let account = if report.complete { "complete".to_string() } else { format!("not complete: {}", open.join(", ")) };
    let painter = ui.painter_at(rect);
    let small = FontId::proportional(11.0);
    painter.text(rect.left_top() + vec2(8.0, 5.0), Align2::LEFT_TOP, head, FontId::proportional(13.0), t.text_strong);
    painter.text(rect.left_top() + vec2(8.0, 24.0), Align2::LEFT_TOP, counts, small.clone(), t.text_dim);
    painter.text(rect.left_top() + vec2(8.0, 39.0), Align2::LEFT_TOP, account, small, t.text_dim);
    response
}

fn paint_play(ui: &Ui, rect: Rect, color: Color32, playing: bool) {
    let c = rect.center();
    if playing {
        for dx in [-3.5, 3.5] {
            ui.painter().line_segment([pos2(c.x + dx, c.y - 6.0), pos2(c.x + dx, c.y + 6.0)], Stroke::new(3.0, color));
        }
    } else {
        ui.painter().add(egui::Shape::convex_polygon(
            vec![pos2(c.x - 4.0, c.y - 6.0), pos2(c.x + 6.0, c.y), pos2(c.x - 4.0, c.y + 6.0)],
            color,
            Stroke::NONE,
        ));
    }
}

fn paint_back(ui: &Ui, rect: Rect, color: Color32) {
    let c = rect.center();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(c.x + 4.0, c.y - 5.0), pos2(c.x + 4.0, c.y + 5.0), pos2(c.x - 4.0, c.y)],
        color,
        Stroke::NONE,
    ));
}

fn paint_forward(ui: &Ui, rect: Rect, color: Color32) {
    let c = rect.center();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(c.x - 4.0, c.y - 5.0), pos2(c.x + 4.0, c.y), pos2(c.x - 4.0, c.y + 5.0)],
        color,
        Stroke::NONE,
    ));
}
