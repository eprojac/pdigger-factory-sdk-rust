//! The client: base URL, timeouts, retries with backoff, headers, and the entry points to the
//! five layers (`api()`, `streams()`, `session()`, `interpreter()`, sources).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::future::BoxFuture;
use serde::de::DeserializeOwned;

use crate::api::Api;
use crate::error::{Error, ErrorCode, Result};
use crate::interpreter::Interpreter;
use crate::models::Source;
use crate::session::Session;
use crate::streams::Streams;
use crate::transport::{Headers, Method, Request, ReqwestTransport, Response, Transport};

/// The SDK's user agent: `pdigger-factory-sdk-rust/<version>`.
pub const USER_AGENT: &str = concat!("pdigger-factory-sdk-rust/", env!("CARGO_PKG_VERSION"));

/// A source of uniform random numbers in `[0, 1)` for the backoff's jitter. Inject a seeded
/// one ([`SeededRandom::new`]) for deterministic retries.
pub trait RandomSource: Send + 'static {
    /// The next number in `[0, 1)`.
    fn next_f64(&mut self) -> f64;
}

/// SplitMix64: small, fast, deterministic for a seed.
#[derive(Debug, Clone)]
pub struct SeededRandom(u64);

impl SeededRandom {
    /// A generator with this seed.
    pub fn new(seed: u64) -> SeededRandom {
        SeededRandom(seed)
    }

    /// A generator seeded from the system clock.
    pub fn from_time() -> SeededRandom {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        SeededRandom(nanos as u64)
    }
}

impl RandomSource for SeededRandom {
    fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Wall time in epoch seconds. Inject a manual clock (the `testing` module has one) to make the
/// session clock deterministic.
pub trait Clock: Send + Sync + 'static {
    /// Now, in epoch seconds.
    fn now(&self) -> f64;

    /// Waits `duration` on this clock: what [`crate::Session::until_live`] waits with between its
    /// reads. The default sleeps on tokio's timer; a test clock may move itself on instead (the
    /// `testing` module's `ManualClock` does), so that such a wait takes no real time.
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// The system's wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
    }
}

/// Exponential backoff with full jitter: attempt `n` waits a uniform random time in
/// `[0, min(max, base * factor^n))`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backoff {
    /// First delay (250 ms).
    pub base: Duration,
    /// Growth per attempt (2).
    pub factor: f64,
    /// Longest delay (5 s).
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff { base: Duration::from_millis(250), factor: 2.0, max: Duration::from_secs(5) }
    }
}

impl Backoff {
    /// The cap of attempt `attempt` (0-based) before jitter.
    pub fn cap(&self, attempt: u32) -> Duration {
        if self.base.is_zero() {
            return Duration::ZERO; // at every attempt, also where factor^attempt is infinite (0 * inf is NaN)
        }
        // past the cap the growth is infinite, which the cap holds (a slow factor reaches it late)
        let grown = self.base.as_secs_f64() * self.factor.powi(attempt.min(i32::MAX as u32) as i32);
        Duration::from_secs_f64(grown.min(self.max.as_secs_f64()).max(0.0))
    }
}

/// Builds a [`FactoryClient`].
pub struct ClientBuilder {
    base_url: String,
    request_timeout: Duration,
    connect_timeout: Duration,
    stream_idle_timeout: Duration,
    retries: u32,
    backoff: Backoff,
    headers: Vec<(String, String)>,
    token: Option<String>,
    user_agent_suffix: Option<String>,
    transport: Option<Arc<dyn Transport>>,
    observe: bool,
    random: Option<Box<dyn RandomSource>>,
    clock: Option<Arc<dyn Clock>>,
}

impl ClientBuilder {
    /// Timeout of a request (10 s): the whole of an ordinary request; for a stream, the wait for
    /// its reply headers (a stream that does not answer in time counts as broken and is tried
    /// again).
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// The streams' idle timeout (30 s): a connection that has sent something and then nothing
    /// for this long is closed and resumed (a half-open connection). `Duration::ZERO` turns the
    /// watchdog off.
    pub fn stream_idle_timeout(mut self, timeout: Duration) -> Self {
        self.stream_idle_timeout = timeout;
        self
    }

    /// Timeout of a connection (5 s), for the default transport.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Retries of a request (3): GET/HEAD on connection errors, 429, 502, 503, 504; other
    /// methods on 429 only.
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// The backoff between retries and stream reconnects.
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// An extra header on every request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sends `Authorization: Bearer <token>` (20.20 ignores it; a deployment may require it).
    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Appended to the user agent: `pdigger-factory-sdk-rust/0.1.0 <suffix>`.
    pub fn user_agent_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.user_agent_suffix = Some(suffix.into());
        self
    }

    /// Sends every request through `transport` instead of the default `reqwest` one.
    pub fn transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// The default of `observe` for streams that do not say (false: a stream counts as a
    /// viewer and keeps the interpreter running).
    pub fn observe(mut self, observe: bool) -> Self {
        self.observe = observe;
        self
    }

    /// The random source of the backoff's jitter.
    pub fn random(mut self, random: impl RandomSource) -> Self {
        self.random = Some(Box::new(random));
        self
    }

    /// The wall clock of the session clock.
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// The client.
    pub fn build(self) -> Result<FactoryClient> {
        let base_url = self.base_url.trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(Error::new(
                ErrorCode::InvalidParam,
                format!("base URL must start with http:// or https://: {base_url:?}"),
            ));
        }
        let transport: Arc<dyn Transport> = match self.transport {
            Some(transport) => transport,
            None => Arc::new(ReqwestTransport::new(self.connect_timeout)?),
        };
        let user_agent = match self.user_agent_suffix {
            Some(suffix) => format!("{USER_AGENT} {suffix}"),
            None => USER_AGENT.to_string(),
        };
        let mut headers = vec![("User-Agent".to_string(), user_agent)];
        if let Some(token) = self.token {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }
        headers.extend(self.headers);
        Ok(FactoryClient {
            inner: Arc::new(Inner {
                base_url,
                transport,
                request_timeout: self.request_timeout,
                stream_idle_timeout: self.stream_idle_timeout,
                retries: self.retries,
                backoff: self.backoff,
                headers,
                observe: self.observe,
                random: Mutex::new(self.random.unwrap_or_else(|| Box::new(SeededRandom::from_time()))),
                clock: self.clock.unwrap_or_else(|| Arc::new(SystemClock)),
            }),
        })
    }
}

struct Inner {
    base_url: String,
    transport: Arc<dyn Transport>,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
    retries: u32,
    backoff: Backoff,
    headers: Vec<(String, String)>,
    observe: bool,
    random: Mutex<Box<dyn RandomSource>>,
    clock: Arc<dyn Clock>,
}

/// The client of one pDigger Factory server. Cheap to clone (shared state).
///
/// ```no_run
/// # async fn demo() -> pdigger_factory_sdk::Result<()> {
/// use pdigger_factory_sdk::FactoryClient;
/// let factory = FactoryClient::new("http://factory-orin.local:8120")?;
/// for source in factory.sources().await? {
///     println!("{} {}", source.id, source.name());
/// }
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct FactoryClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for FactoryClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FactoryClient").field("base_url", &self.inner.base_url).finish_non_exhaustive()
    }
}

/// Query parameters of a request.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Query(pub(crate) Vec<(&'static str, String)>);

impl Query {
    pub(crate) fn add(mut self, key: &'static str, value: impl ToString) -> Self {
        self.0.push((key, value.to_string()));
        self
    }

    pub(crate) fn opt(self, key: &'static str, value: Option<impl ToString>) -> Self {
        match value {
            Some(value) => self.add(key, value),
            None => self,
        }
    }

    /// `key=1` when `on`.
    pub(crate) fn flag(self, key: &'static str, on: bool) -> Self {
        if on { self.add(key, 1) } else { self }
    }

    fn encode(&self) -> String {
        self.0.iter().map(|(k, v)| format!("{k}={}", encode_component(v, true))).collect::<Vec<_>>().join("&")
    }
}

/// Percent-encodes everything but the unreserved characters (and `,` when `commas`).
pub(crate) fn encode_component(text: &str, commas: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) || (commas && byte == b',') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl FactoryClient {
    /// A client of the server at `base_url` (`http://factory-orin.local:8120`) with the default
    /// options.
    pub fn new(base_url: &str) -> Result<FactoryClient> {
        FactoryClient::builder(base_url).build()
    }

    /// A builder for a client of the server at `base_url`.
    pub fn builder(base_url: &str) -> ClientBuilder {
        ClientBuilder {
            base_url: base_url.to_string(),
            request_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(5),
            stream_idle_timeout: Duration::from_secs(30),
            retries: 3,
            backoff: Backoff::default(),
            headers: Vec::new(),
            token: None,
            user_agent_suffix: None,
            transport: None,
            observe: false,
            random: None,
            clock: None,
        }
    }

    /// The base URL, without a trailing `/`.
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// The low level: one method per HTTP operation.
    pub fn api(&self) -> Api {
        Api::new(self.clone())
    }

    /// The mid level: frames over SSE with resume, the long-poll loop, the raw packet stream.
    pub fn streams(&self) -> Streams {
        Streams::new(self.clone())
    }

    /// The shared session: state, clock and the controls (which move it for every viewer).
    pub fn session(&self) -> Session {
        Session::new(self.clone())
    }

    /// The high level: an [`Interpreter`] that follows the shared session (layers `world` and
    /// `cards`, `observe` false, backfill on).
    pub fn interpreter(&self) -> Interpreter {
        Interpreter::new(self.clone())
    }

    /// The sources the shared session can show (the recordings and the live cameras).
    pub async fn sources(&self) -> Result<Vec<Source>> {
        Ok(self.api().list_videos().await?.videos)
    }

    /// The live cameras of the catalog (builds before `claude-20261120-live2` list none).
    pub async fn cameras(&self) -> Result<Vec<Source>> {
        Ok(self.api().list_videos().await?.cameras().into_iter().cloned().collect())
    }

    /// One source by id (`unknown_video` when it is not in the catalog).
    pub async fn source(&self, id: &str) -> Result<Source> {
        self.api().get_video(id).await
    }

    /// The first live camera of the catalog, or the `live_unavailable` error when it lists none
    /// (builds before `claude-20261120-live2`). Pick another with [`FactoryClient::cameras`].
    pub async fn live(&self) -> Result<Source> {
        let list = self.api().list_videos().await?;
        match list.live() {
            Some(live) => Ok(live.clone()),
            None => Err(Error::new(ErrorCode::LiveUnavailable, "the server's catalog has no live camera")),
        }
    }

    /// The default of `observe` for streams.
    pub fn observe_default(&self) -> bool {
        self.inner.observe
    }

    /// Now on the client's wall clock (epoch seconds).
    pub fn now(&self) -> f64 {
        self.inner.clock.now()
    }

    /// The wall clock the client was built with.
    pub(crate) fn clock_source(&self) -> Arc<dyn Clock> {
        self.inner.clock.clone()
    }

    pub(crate) fn backoff(&self) -> Backoff {
        self.inner.backoff
    }

    /// Retries of a request.
    pub(crate) fn retries(&self) -> u32 {
        self.inner.retries
    }

    /// The streams' idle timeout (`None`: off).
    pub(crate) fn stream_idle_timeout(&self) -> Option<Duration> {
        Some(self.inner.stream_idle_timeout).filter(|t| !t.is_zero())
    }

    /// A per-client id for the long-poll (`^[A-Za-z0-9_-]{8,40}$`), from the backoff's random
    /// source.
    pub(crate) fn new_cid(&self) -> String {
        const ALPHABET: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
        let mut random = self.inner.random.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let tail: String = (0..16).map(|_| char::from(ALPHABET[(random.next_f64() * 64.0) as usize % 64])).collect();
        format!("rs-{tail}")
    }

    pub(crate) fn url(&self, path: &str, query: &Query) -> String {
        if query.0.is_empty() {
            format!("{}{path}", self.inner.base_url)
        } else {
            format!("{}{path}?{}", self.inner.base_url, query.encode())
        }
    }

    /// How long to wait before retry `attempt` (0-based) after `error`: the server's
    /// `retry_ms` or `Retry-After`, else the jittered backoff.
    pub(crate) fn retry_delay(&self, attempt: u32, error: &Error) -> Duration {
        if let Some(delay) = error.retry_delay() {
            return delay;
        }
        let cap = self.inner.backoff.cap(attempt);
        let jitter = self.inner.random.lock().unwrap_or_else(std::sync::PoisonError::into_inner).next_f64();
        cap.mul_f64(jitter)
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        query: &Query,
        accept: &str,
        body: Option<Bytes>,
        timeout: Option<Duration>,
    ) -> Request {
        let mut headers = self.inner.headers.clone();
        headers.push(("Accept".to_string(), accept.to_string()));
        if body.is_some() {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        Request { method, url: self.url(path, query), headers, body, timeout }
    }

    /// Sends a request with the retry policy; a reply of status 400 and above is an error.
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        query: &Query,
        extra: &[(&str, String)],
        body: Option<Bytes>,
    ) -> Result<Response> {
        self.send_with_retries(method, path, query, extra, body, self.inner.retries).await
    }

    /// [`FactoryClient::send`] with at most `retries` retries (0: one attempt).
    pub(crate) async fn send_with_retries(
        &self,
        method: Method,
        path: &str,
        query: &Query,
        extra: &[(&str, String)],
        body: Option<Bytes>,
        retries: u32,
    ) -> Result<Response> {
        let accept =
            extra.iter().find(|(k, _)| k.eq_ignore_ascii_case("accept")).map_or("application/json", |(_, v)| v);
        let mut request = self.request(method, path, query, accept, body, Some(self.inner.request_timeout));
        let rest = extra.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("accept"));
        request.headers.extend(rest.map(|(k, v)| (k.to_string(), v.clone())));
        let mut attempt = 0;
        loop {
            let error = match self.inner.transport.send(request.clone()).await {
                Ok(reply) if reply.status < 400 => return Ok(reply),
                Ok(reply) => reply_error(reply).await,
                Err(error) => error,
            };
            let retry = if method.is_read() { error.is_retryable() } else { error.status == Some(429) };
            if !retry || attempt >= retries {
                return Err(error);
            }
            tokio::time::sleep(self.retry_delay(attempt, &error)).await;
            attempt += 1;
        }
    }

    /// Opens a stream: one attempt; the wait for the reply headers is bounded by the request
    /// timeout (a `transport` error), the body is not; a reply of status 400 and above is an
    /// error.
    pub(crate) async fn open_stream(
        &self,
        path: &str,
        query: &Query,
        accept: &str,
        extra: &[(&str, String)],
    ) -> Result<Response> {
        let mut request = self.request(Method::Get, path, query, accept, None, None);
        request.headers.extend(extra.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let reply = tokio::time::timeout(self.inner.request_timeout, self.inner.transport.send(request))
            .await
            .map_err(|_| {
                Error::transport(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the stream sent no reply headers within the request timeout",
                ))
            })??;
        if reply.status >= 400 {
            return Err(reply_error(reply).await);
        }
        Ok(reply)
    }

    /// GET a JSON document.
    pub(crate) async fn get_json<T: DeserializeOwned>(&self, path: &str, query: &Query) -> Result<T> {
        let reply = self.send(Method::Get, path, query, &[], None).await?;
        decode_json(reply).await
    }

    /// GET a JSON document with one request (no retries: the caller has a schedule of its own).
    pub(crate) async fn get_json_once<T: DeserializeOwned>(&self, path: &str, query: &Query) -> Result<T> {
        let reply = self.send_with_retries(Method::Get, path, query, &[], None, 0).await?;
        decode_json(reply).await
    }

    /// A control (POST/PUT/DELETE) with a JSON body and `dry_run`.
    pub(crate) async fn control<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Bytes>,
        dry_run: bool,
    ) -> Result<T> {
        let reply = self.send(method, path, &Query::default().flag("dry_run", dry_run), &[], body).await?;
        decode_json(reply).await
    }
}

/// The error of a reply with status 400 and above.
pub(crate) async fn reply_error(reply: Response) -> Error {
    let status = reply.status;
    let retry_after = reply.headers.retry_after_s();
    match reply.body.bytes().await {
        Ok(body) => Error::from_reply(status, &body, retry_after),
        Err(error) => error.with_status(status),
    }
}

/// The JSON body of a reply.
pub(crate) async fn decode_json<T: DeserializeOwned>(reply: Response) -> Result<T> {
    let status = reply.status;
    let body = reply.body.bytes().await?;
    serde_json::from_slice(&body).map_err(|e| Error::decode(Some(status), format!("not the documented JSON: {e}")))
}

/// The headers of a reply, for replies that carry useful ones.
pub(crate) fn header(headers: &Headers, name: &str) -> Option<String> {
    headers.get(name).map(str::to_string)
}
