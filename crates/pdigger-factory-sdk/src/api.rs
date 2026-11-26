//! The low level: one method per operation of `spec/openapi.yaml` (named after its
//! `operationId`), typed requests and replies, typed errors.
//!
//! Controls (`session_select`, `session_seek`, `session_play`, `session_pause`, `put_focus`,
//! `delete_focus`) move the one shared session **for every viewer**; each takes `dry_run`,
//! which validates and answers without moving anything.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;

use crate::client::{FactoryClient, Query, decode_json, encode_component, header};
use crate::error::{Error, ErrorCode, Result};
use crate::models::*;
use crate::sse::EventStream;
use crate::transport::{Body, Headers, Method};
use crate::wire::WireEnum;

/// A grid frame or the newest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSel {
    /// Grid frame `g`.
    G(i64),
    /// The newest retained frame.
    Latest,
}

impl From<i64> for FrameSel {
    fn from(g: i64) -> FrameSel {
        FrameSel::G(g)
    }
}

impl FrameSel {
    fn segment(self) -> String {
        match self {
            FrameSel::G(g) => g.to_string(),
            FrameSel::Latest => "latest".to_string(),
        }
    }
}

/// Query of `get_world_events`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldEventsQuery {
    /// Only events with a larger `g` (default -1).
    pub after: Option<i64>,
    /// At most this many events, by whole frames (1..1024, default 256).
    pub limit: Option<u32>,
    /// Only these types.
    pub types: Vec<EventKind>,
}

/// Query of `get_frame`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameQuery {
    /// Layers (empty: the server's default `overlay,scene,cards,world`).
    pub layers: Vec<Layer>,
    /// Answer 410 if this is not the current session.
    pub session: Option<i64>,
    /// Number precision.
    pub precision: Option<Precision>,
}

/// Query of `poll_frames`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollQuery {
    /// The last cursor this client has.
    pub cursor: Option<i64>,
    /// The last state revision this client has (0 returns at once).
    pub rev: Option<i64>,
    /// A per-tab id (`^[A-Za-z0-9_-]{8,40}$`); counts this client as a viewer.
    pub cid: Option<String>,
    /// Layers.
    pub layers: Vec<Layer>,
    /// At most this many newest frames (1..16).
    pub limit: Option<u32>,
    /// Longest wait in ms (0..1000).
    pub wait_ms: Option<u32>,
    /// Number precision.
    pub precision: Option<Precision>,
}

/// Query of `stream_frames` and `raw_stream`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamQuery {
    /// Layers (`stream_frames` only).
    pub layers: Vec<Layer>,
    /// SSE (the default) or NDJSON.
    pub format: StreamFormat,
    /// Start after this cursor (default the newest).
    pub after: Option<i64>,
    /// End the stream after this many frames.
    pub limit: Option<u32>,
    /// End the stream after this many seconds.
    pub max_s: Option<u32>,
    /// Do not count as a viewer (`observe=1`).
    pub observe: bool,
    /// Number precision (`stream_frames` only).
    pub precision: Option<Precision>,
    /// Resume after this event id (`Last-Event-ID`).
    pub last_event_id: Option<String>,
}

impl StreamQuery {
    pub(crate) fn query(&self, frames: bool) -> Query {
        let mut query = Query::default();
        if frames && !self.layers.is_empty() {
            query = query.add("layers", join_layers(&self.layers));
        }
        query = query
            .add("format", self.format.as_str())
            .opt("after", self.after)
            .opt("limit", self.limit)
            .opt("max_s", self.max_s)
            .flag("observe", self.observe);
        if frames {
            query = query.opt("precision", self.precision.as_ref());
        }
        query
    }

    pub(crate) fn accept(&self) -> &'static str {
        if self.format == StreamFormat::Ndjson { "application/x-ndjson" } else { "text/event-stream" }
    }

    pub(crate) fn headers(&self) -> Vec<(&'static str, String)> {
        self.last_event_id.iter().map(|id| ("Last-Event-ID", id.clone())).collect()
    }
}

/// When a render is: exactly one of a grid frame and a media time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RenderAt {
    /// A grid frame.
    G(i64),
    /// A media time in seconds (interpolated between packets).
    T(f64),
}

impl Default for RenderAt {
    fn default() -> Self {
        RenderAt::G(0)
    }
}

/// Query of `render_overlay2d` and `render_scene3d`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderQuery {
    /// When.
    pub at: RenderAt,
    /// `stable` (default) or `raw`.
    pub mode: Option<RenderMode>,
    /// `shapes` (default) or `primitives`.
    pub draw: Option<RenderDraw>,
    /// `vertices` (default), `compact` or `none` (`render_scene3d` only).
    pub surface: Option<RenderSurface>,
    /// Answer 410 if this is not the current session.
    pub session: Option<i64>,
}

impl RenderQuery {
    /// A render of grid frame `g` with the defaults.
    pub fn at_g(g: i64) -> RenderQuery {
        RenderQuery { at: RenderAt::G(g), ..RenderQuery::default() }
    }

    /// A render at media time `t` with the defaults.
    pub fn at_t(t: f64) -> RenderQuery {
        RenderQuery { at: RenderAt::T(t), ..RenderQuery::default() }
    }

    fn query(&self, scene: bool) -> Query {
        let query = match self.at {
            RenderAt::G(g) => Query::default().add("g", g),
            RenderAt::T(t) => Query::default().add("t", t),
        };
        let query = query.opt("mode", self.mode.as_ref()).opt("draw", self.draw.as_ref());
        let query = if scene { query.opt("surface", self.surface.as_ref()) } else { query };
        query.opt("session", self.session)
    }
}

/// A byte range of the proxy video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    /// Bytes `start..=end`.
    Span(u64, u64),
    /// From `start` to the end.
    From(u64),
    /// The last `n` bytes.
    Last(u64),
}

impl ByteRange {
    /// The `Range` header value.
    pub fn header(self) -> String {
        match self {
            ByteRange::Span(start, end) => format!("bytes={start}-{end}"),
            ByteRange::From(start) => format!("bytes={start}-"),
            ByteRange::Last(n) => format!("bytes=-{n}"),
        }
    }
}

/// Query of `get_video_media`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaQuery {
    /// One byte range.
    pub range: Option<ByteRange>,
    /// ETag or Last-Modified of the previous reply.
    pub if_range: Option<String>,
    /// ETag of a cached copy (304 when unchanged).
    pub if_none_match: Option<String>,
}

/// `Content-Range: bytes start-end/total` (or `bytes */total`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentRange {
    /// First byte.
    pub start: Option<u64>,
    /// Last byte.
    pub end: Option<u64>,
    /// Size of the whole file.
    pub total: Option<u64>,
}

impl ContentRange {
    /// Parses a `Content-Range` value.
    pub fn parse(text: &str) -> Option<ContentRange> {
        let rest = text.trim().strip_prefix("bytes")?.trim_start();
        let (span, total) = rest.split_once('/')?;
        let total = total.trim().parse().ok();
        let (start, end) = match span.trim().split_once('-') {
            Some((a, b)) => (a.trim().parse().ok(), b.trim().parse().ok()),
            None => (None, None),
        };
        Some(ContentRange { start, end, total })
    }
}

/// A reply of `get_video_media` / `head_video_media`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Media {
    /// 200 (whole), 206 (range) or 304 (unchanged).
    pub status: u16,
    /// The bytes (empty for HEAD and 304).
    pub data: Bytes,
    /// `Content-Type`.
    pub content_type: Option<String>,
    /// `Content-Length`.
    pub content_length: Option<u64>,
    /// `Content-Range` of a 206.
    pub content_range: Option<ContentRange>,
    /// `ETag`.
    pub etag: Option<String>,
    /// `Accept-Ranges`.
    pub accept_ranges: Option<String>,
}

impl Media {
    fn of(status: u16, headers: &Headers, data: Bytes) -> Media {
        Media {
            status,
            data,
            content_type: header(headers, "content-type"),
            content_length: headers.get("content-length").and_then(|v| v.trim().parse().ok()),
            content_range: headers.get("content-range").and_then(ContentRange::parse),
            etag: header(headers, "etag"),
            accept_ranges: header(headers, "accept-ranges"),
        }
    }

    /// A 200 with neither a `Content-Length` nor byte ranges: media that does not end.
    fn is_endless(&self) -> bool {
        self.status == 200 && self.content_length.is_none() && self.accept_ranges.as_deref() != Some("bytes")
    }
}

/// Media read as it arrives ([`Api::open_video_media`]): the reply's headers in
/// [`MediaStream::media`], then the body as a [`Stream`] of chunks. Dropping the stream closes
/// the connection.
#[derive(Debug)]
pub struct MediaStream {
    /// The status and headers of the reply (`data` is empty).
    pub media: Media,
    body: Body,
}

impl Stream for MediaStream {
    type Item = Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().body).poll_next(cx)
    }
}

/// `not_seekable` (field `id`, the reply's status): media that never ends cannot be read into
/// memory.
fn endless_media(id: &str, status: u16) -> Error {
    Error::new(ErrorCode::NotSeekable, format!("the media of {id} does not end: read it with open_video_media"))
        .with_field("id")
        .with_status(status)
}

/// The camera picture a live frame was interpreted from ([`Api::get_frame_picture`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Picture {
    /// The frame's grid index (the reply's `X-Interp-G`: the frame `latest` was).
    pub g: i64,
    /// The JPEG (640 × 360 on the plant rigs; the layers' native pixels are this picture
    /// scaled to fit `native_wh` and centred, the rest black: a picture of another shape is
    /// letterboxed, from worker `claude-20261125-dspad1` on; earlier workers stretched it).
    pub jpeg: Bytes,
}

/// A conditional reply: `value` is `None` when the server answered 304 (unchanged).
#[derive(Debug, Clone, PartialEq)]
pub struct Cached<T> {
    /// The document, or `None` when unchanged.
    pub value: Option<T>,
    /// Its `ETag`.
    pub etag: Option<String>,
}

/// The low-level API: one method per operation. Get it with [`FactoryClient::api`].
#[derive(Debug, Clone)]
pub struct Api {
    client: FactoryClient,
}

fn json_body<T: serde::Serialize>(value: &T) -> Option<Bytes> {
    Some(Bytes::from(serde_json::to_vec(value).unwrap_or_default()))
}

fn media_headers(query: &MediaQuery) -> Vec<(&'static str, String)> {
    let mut extra = Vec::new();
    if let Some(range) = query.range {
        extra.push(("Range", range.header()));
    }
    if let Some(tag) = &query.if_range {
        extra.push(("If-Range", tag.clone()));
    }
    if let Some(tag) = &query.if_none_match {
        extra.push(("If-None-Match", tag.clone()));
    }
    extra
}

fn video_path(id: &str) -> String {
    format!("/api/v2/videos/{}", encode_component(id, false))
}

fn frame_query(query: &FrameQuery) -> Query {
    let q = if query.layers.is_empty() {
        Query::default()
    } else {
        Query::default().add("layers", join_layers(&query.layers))
    };
    q.opt("session", query.session).opt("precision", query.precision.as_ref())
}

impl Api {
    pub(crate) fn new(client: FactoryClient) -> Api {
        Api { client }
    }

    /// The client this API belongs to.
    pub fn client(&self) -> &FactoryClient {
        &self.client
    }

    // ---- Interpreter ------------------------------------------------------------------------

    /// `getWorld`: the newest world (at its newest revision). 503 `world_unavailable` while the
    /// Stable sim is off.
    pub async fn get_world(&self, precision: Option<Precision>) -> Result<WorldLayer> {
        self.client.get_json("/api/v2/session/world", &Query::default().opt("precision", precision.as_ref())).await
    }

    /// `getWorldEvents`: the event log of this session, oldest first.
    pub async fn get_world_events(&self, query: &WorldEventsQuery) -> Result<EventLog> {
        let types =
            (!query.types.is_empty()).then(|| query.types.iter().map(WireEnum::as_str).collect::<Vec<_>>().join(","));
        let q = Query::default().opt("after", query.after).opt("limit", query.limit).opt("type", types);
        self.client.get_json("/api/v2/session/world/events", &q).await
    }

    /// `getStone`: one stone with its path since the session began.
    pub async fn get_stone(&self, id: i64) -> Result<StoneTrack> {
        self.client.get_json(&format!("/api/v2/session/world/stones/{id}"), &Query::default()).await
    }

    /// `getFocus`: the stone highlighted for every viewer, and its path.
    pub async fn get_focus(&self) -> Result<FocusReply> {
        self.client.get_json("/api/v2/session/focus", &Query::default()).await
    }

    /// `putFocus`: highlight a stone for **every viewer**.
    pub async fn put_focus(&self, request: &FocusRequest, dry_run: bool) -> Result<ControlReply> {
        self.client.control(Method::Put, "/api/v2/session/focus", json_body(request), dry_run).await
    }

    /// `deleteFocus`: clear the highlight for **every viewer**.
    pub async fn delete_focus(&self, dry_run: bool) -> Result<ControlReply> {
        self.client.control(Method::Delete, "/api/v2/session/focus", None, dry_run).await
    }

    // ---- Sources ----------------------------------------------------------------------------

    /// `listVideos`: the public catalog.
    pub async fn list_videos(&self) -> Result<VideoList> {
        Ok(self.list_videos_if_changed(None).await?.value.unwrap_or_default())
    }

    /// `listVideos` with `If-None-Match`: `value` is `None` when the catalog is unchanged
    /// (304).
    pub async fn list_videos_if_changed(&self, etag: Option<&str>) -> Result<Cached<VideoList>> {
        let extra: Vec<(&str, String)> = etag.iter().map(|e| ("If-None-Match", e.to_string())).collect();
        let reply = self.client.send(Method::Get, "/api/v2/videos", &Query::default(), &extra, None).await?;
        let etag = header(&reply.headers, "etag");
        if reply.status == 304 {
            return Ok(Cached { value: None, etag });
        }
        Ok(Cached { value: Some(decode_json(reply).await?), etag })
    }

    /// `getVideo`: one catalog entry (404 `unknown_video`).
    pub async fn get_video(&self, id: &str) -> Result<VideoEntry> {
        self.client.get_json(&video_path(id), &Query::default()).await
    }

    /// `getVideoMedia`: the proxy video, whole or one byte range, read into memory. A range
    /// the file does not have is `invalid_param` (field `range`, HTTP 416). A live camera has no
    /// media (404 `not_found`): its pictures come with its frames ([`Api::get_frame_picture`]). A
    /// reply that has neither a length nor byte ranges would never end: it gets `not_seekable`
    /// (field `id`) and its connection is closed unread; read such media with
    /// [`Api::open_video_media`].
    pub async fn get_video_media(&self, id: &str, query: &MediaQuery) -> Result<Media> {
        self.media(Method::Get, id, query).await
    }

    /// The `HEAD` variant of `getVideoMedia`: the same headers, no body.
    pub async fn head_video_media(&self, id: &str) -> Result<Media> {
        self.media(Method::Head, id, &MediaQuery::default()).await
    }

    /// `getVideoMedia` read as it arrives: one request with no request timeout, whose
    /// [`MediaStream`] gives the headers at once and then the body chunk by chunk (also media
    /// that never ends); dropping the stream closes the connection. `query` is that of
    /// [`Api::get_video_media`].
    pub async fn open_video_media(&self, id: &str, query: &MediaQuery) -> Result<MediaStream> {
        let reply = self
            .client
            .open_stream(&format!("{}/media", video_path(id)), &Query::default(), "video/mp4", &media_headers(query))
            .await?;
        Ok(MediaStream { media: Media::of(reply.status, &reply.headers, Bytes::new()), body: reply.body })
    }

    async fn media(&self, method: Method, id: &str, query: &MediaQuery) -> Result<Media> {
        let reply = self
            .client
            .send(method, &format!("{}/media", video_path(id)), &Query::default(), &media_headers(query), None)
            .await?;
        let media = Media::of(reply.status, &reply.headers, Bytes::new());
        if method == Method::Head {
            return Ok(media);
        }
        if media.is_endless() {
            drop(reply.body); // closes the connection unread
            return Err(endless_media(id, media.status));
        }
        Ok(Media { data: reply.body.bytes().await?, ..media })
    }

    // ---- Session ----------------------------------------------------------------------------

    /// `getSession`: the shared clock, the worker and the Stable sim.
    pub async fn get_session(&self) -> Result<SessionReply> {
        self.client.get_json("/api/v2/session", &Query::default()).await
    }

    /// [`Api::get_session`] as one request, without retries (a clock sample, the state read
    /// after a connect).
    pub(crate) async fn get_session_once(&self) -> Result<SessionReply> {
        self.client.get_json_once("/api/v2/session", &Query::default()).await
    }

    /// `sessionSelect`: switch the shared session to another source, for **every viewer**
    /// (a new interpretation run). Leave `time` out for a live camera (400 `not_seekable`, field
    /// `time`).
    pub async fn session_select(&self, request: &SelectRequest, dry_run: bool) -> Result<ControlReply> {
        self.client.control(Method::Post, "/api/v2/session/select", json_body(request), dry_run).await
    }

    /// `sessionSeek`: move the shared playhead for **every viewer** (a new interpretation run).
    /// 409 `not_seekable` (field `video`) in a live camera.
    pub async fn session_seek(&self, request: &SeekRequest, dry_run: bool) -> Result<ControlReply> {
        self.client.control(Method::Post, "/api/v2/session/seek", json_body(request), dry_run).await
    }

    /// `sessionPlay`: resume the shared playhead for **every viewer**. Idempotent.
    pub async fn session_play(&self, dry_run: bool) -> Result<ControlReply> {
        self.client.control(Method::Post, "/api/v2/session/play", json_body(&EmptyRequest {}), dry_run).await
    }

    /// `sessionPause`: pause the shared playhead for **every viewer**. Idempotent.
    pub async fn session_pause(&self, dry_run: bool) -> Result<ControlReply> {
        self.client.control(Method::Post, "/api/v2/session/pause", json_body(&EmptyRequest {}), dry_run).await
    }

    // ---- Streams ----------------------------------------------------------------------------

    /// `streamFrames`: one connection of the frame stream (SSE or NDJSON), as raw events.
    /// The mid level ([`crate::Streams::frames`]) resumes, backfills and deduplicates.
    pub async fn stream_frames(&self, query: &StreamQuery) -> Result<EventStream> {
        let reply = self
            .client
            .open_stream("/api/v2/session/frames/stream", &query.query(true), query.accept(), &query.headers())
            .await?;
        Ok(EventStream::new(reply.body, query.format.clone()))
    }

    /// `pollFrames`: one long-poll call.
    pub async fn poll_frames(&self, query: &PollQuery) -> Result<LiveReply> {
        let mut q = Query::default().opt("cursor", query.cursor).opt("rev", query.rev).opt("cid", query.cid.as_ref());
        if !query.layers.is_empty() {
            q = q.add("layers", join_layers(&query.layers));
        }
        let q = q.opt("limit", query.limit).opt("wait", query.wait_ms).opt("precision", query.precision.as_ref());
        self.client.get_json("/api/v2/session/frames", &q).await
    }

    /// `rawStream`: one connection of the raw model codec stream, as raw events.
    pub async fn raw_stream(&self, query: &StreamQuery) -> Result<EventStream> {
        let reply = self
            .client
            .open_stream("/api/v2/raw/stream", &query.query(false), query.accept(), &query.headers())
            .await?;
        Ok(EventStream::new(reply.body, query.format.clone()))
    }

    // ---- Render-ready -----------------------------------------------------------------------

    /// `renderOverlay2d`: everything the 2-D overlay draws, final, in native px.
    pub async fn render_overlay2d(&self, query: &RenderQuery) -> Result<Overlay2D> {
        self.client.get_json("/api/v2/session/render/overlay2d", &query.query(false)).await
    }

    /// `renderScene3d`: everything the 3-D view draws, final, in the scene world.
    pub async fn render_scene3d(&self, query: &RenderQuery) -> Result<Scene3D> {
        self.client.get_json("/api/v2/session/render/scene3d", &query.query(true)).await
    }

    // ---- Frames and codec -------------------------------------------------------------------

    /// `getFrame`: random access to a retained frame (404 `not_retained`, 410 `session_gone`).
    pub async fn get_frame(&self, g: impl Into<FrameSel>, query: &FrameQuery) -> Result<Frame> {
        self.client.get_json(&format!("/api/v2/session/frames/{}", g.into().segment()), &frame_query(query)).await
    }

    /// [`Api::get_frame`] as one request, without retries (the backfill's reads, which have a
    /// schedule of their own).
    pub(crate) async fn get_frame_once(&self, g: i64, query: &FrameQuery) -> Result<Frame> {
        self.client.get_json_once(&format!("/api/v2/session/frames/{g}"), &frame_query(query)).await
    }

    /// `getFramePicture`: the camera picture live frame `g` was interpreted from, as a JPEG. The
    /// newest 64 of a camera session are kept: an older frame, or a warm-up frame (never
    /// released), is 404 `not_retained`; with `session`, a frame of an ended session is 410
    /// `session_gone`. A recording has none (404 `not_found`): its picture is the proxy video.
    pub async fn get_frame_picture(&self, g: impl Into<FrameSel>, session: Option<i64>) -> Result<Picture> {
        let path = format!("/api/v2/session/frames/{}/image.jpg", g.into().segment());
        let accept = [("Accept", "image/jpeg".to_string())];
        let query = Query::default().opt("session", session);
        let reply = self.client.send(Method::Get, &path, &query, &accept, None).await?;
        let status = reply.status;
        let shown = reply.headers.get("x-interp-g").and_then(|g| g.trim().parse::<i64>().ok());
        let jpeg = reply.body.bytes().await?;
        let g = shown.ok_or_else(|| Error::decode(Some(status), "the picture reply has no X-Interp-G header"))?;
        Ok(Picture { g, jpeg })
    }

    /// `getFrameRaster`: a retained frame's map as PNG bytes (64 × 40 times `scale`, 1..8).
    pub async fn get_frame_raster(&self, g: i64, raster: &Raster, scale: Option<u8>) -> Result<Bytes> {
        let path = format!("/api/v2/session/frames/{g}/{}.png", raster.as_str());
        let reply = self.client.send(Method::Get, &path, &Query::default().opt("scale", scale), &[], None).await?;
        reply.body.bytes().await
    }

    /// `rawPacket`: one raw packet of the retained window.
    pub async fn raw_packet(&self, g: impl Into<FrameSel>, session: Option<i64>) -> Result<Sip1Packet> {
        self.client
            .get_json(&format!("/api/v2/raw/packets/{}", g.into().segment()), &Query::default().opt("session", session))
            .await
    }

    /// `rawTensors`: the debug tensors of one frame (`.npz` bytes; 404 `debug_disabled` when off).
    pub async fn raw_tensors(&self, g: i64) -> Result<Bytes> {
        let reply =
            self.client.send(Method::Get, &format!("/api/v2/raw/tensors/{g}"), &Query::default(), &[], None).await?;
        reply.body.bytes().await
    }

    // ---- Meta and ops -----------------------------------------------------------------------

    /// `getApiRoot`: links, layer schema ids, limits and the build.
    pub async fn get_api_root(&self) -> Result<ApiRoot> {
        self.client.get_json("/api/v2", &Query::default()).await
    }

    /// `getOpenApi`: the OpenAPI document (YAML text).
    pub async fn get_openapi(&self) -> Result<String> {
        self.text("/api/v2/openapi.yaml").await
    }

    /// `getModel`: the running model.
    pub async fn get_model(&self) -> Result<ModelInfo> {
        self.client.get_json("/api/v2/model", &Query::default()).await
    }

    /// `getTopology`: skeleton topology, class names, palette.
    pub async fn get_topology(&self) -> Result<Topology> {
        self.client.get_json("/api/v2/topology", &Query::default()).await
    }

    /// `getHealth`: liveness.
    pub async fn get_health(&self) -> Result<Health> {
        self.client.get_json("/api/v2/health", &Query::default()).await
    }

    /// `getReady`: readiness (503 `not_ready` with `waiting` until ready).
    pub async fn get_ready(&self) -> Result<Ready> {
        self.client.get_json("/api/v2/ready", &Query::default()).await
    }

    /// `getStatus`: everything an operator wants on one page.
    pub async fn get_status(&self) -> Result<Status> {
        self.client.get_json("/api/v2/status", &Query::default()).await
    }

    /// `getMetrics`: the Prometheus text and its parsed samples.
    pub async fn get_metrics(&self) -> Result<Metrics> {
        Ok(Metrics::parse(&self.text("/metrics").await?))
    }

    async fn text(&self, path: &str) -> Result<String> {
        let reply = self.client.send(Method::Get, path, &Query::default(), &[], None).await?;
        let status = reply.status;
        let body = reply.body.bytes().await?;
        String::from_utf8(body.to_vec())
            .map_err(|_| Error::new(ErrorCode::Decode, "the reply is not UTF-8 text").with_status(status))
    }
}
