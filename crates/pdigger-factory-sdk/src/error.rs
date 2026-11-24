//! The one error type of the SDK. Code is keyed on [`Error::code`], never on the message.

use std::fmt;
use std::sync::Arc;

use serde::Deserialize;

use crate::models::Retained;
use crate::wire::{WireEnum, wire_enum};

wire_enum! {
    /// The API's stable error codes (`ErrorCode` in the spec) and the SDK-only codes
    /// (`transport`, `decode`, `live_unavailable`, `live_timeout`, `stream_closed`).
    ErrorCode {
        /// The body is not JSON.
        InvalidJson = "invalid_json",
        /// The JSON body is not an object.
        NotAnObject = "not_an_object",
        /// A chunked body was refused.
        ChunkedRefused = "chunked_refused",
        /// A body without `Content-Length`.
        LengthRequired = "length_required",
        /// A control body over 512 bytes.
        BodyTooLarge = "body_too_large",
        /// A GET with a body.
        BodyNotAllowed = "body_not_allowed",
        /// A control from a foreign `Origin`.
        CrossOrigin = "cross_origin",
        /// A body that is not `application/json`.
        JsonOnly = "json_only",
        /// Controls are rate limited; wait `retry_ms`.
        TooManyControls = "too_many_controls",
        /// Requests are rate limited; wait `retry_ms`.
        TooManyRequests = "too_many_requests",
        /// Too many long-polls.
        TooManyViewers = "too_many_viewers",
        /// Too many streams.
        TooManyStreams = "too_many_streams",
        /// Not in the catalog.
        UnknownVideo = "unknown_video",
        /// A time outside the video.
        TimeOutOfRange = "time_out_of_range",
        /// A field of the wrong type.
        InvalidType = "invalid_type",
        /// A required field is missing.
        MissingField = "missing_field",
        /// A bad query or path parameter.
        InvalidParam = "invalid_param",
        /// A layer name that does not exist.
        UnknownLayer = "unknown_layer",
        /// The frame or time is outside the retained window (see `retained`).
        NotRetained = "not_retained",
        /// The session asked for has ended.
        SessionGone = "session_gone",
        /// The request was made for an earlier session.
        StaleSession = "stale_session",
        /// The stone is not in this session's world.
        UnknownStone = "unknown_stone",
        /// The Stable sim is not running.
        WorldUnavailable = "world_unavailable",
        /// Debug tensors are off.
        DebugDisabled = "debug_disabled",
        /// The render queue is full.
        Busy = "busy",
        /// Not ready yet (see `waiting`).
        NotReady = "not_ready",
        /// No such resource.
        NotFound = "not_found",
        /// A method this path does not have.
        MethodNotAllowed = "method_not_allowed",
        /// A server error.
        Internal = "internal",
        /// A live camera cannot be seeked or selected at a time.
        NotSeekable = "not_seekable",
        /// SDK only: no HTTP reply (connection refused, reset, timeout, TLS).
        Transport = "transport",
        /// SDK only: a reply that is not the documented JSON.
        Decode = "decode",
        /// SDK only: the server's catalog has no live camera.
        LiveUnavailable = "live_unavailable",
        /// SDK only: the session's camera did not go live in time ([`crate::Session::until_live`]).
        LiveTimeout = "live_timeout",
        /// SDK only: a stream ended where the SDK needed more of it.
        StreamClosed = "stream_closed",
    }
}

/// An error of the API or of the SDK.
///
/// `status` is the HTTP status (`None` when there was no reply or the SDK raised the error
/// itself), `code` the API's stable code, `message` the server's `error` text. Rate-limited and
/// busy replies carry `retry_ms`; `not_retained` carries the [`Error::retained`] window;
/// `not_ready` carries [`Error::waiting`].
#[derive(Clone)]
pub struct Error {
    /// HTTP status of the reply, if there was one.
    pub status: Option<u16>,
    /// The stable error code.
    pub code: ErrorCode,
    /// Human-readable message (the API's `error`).
    pub message: String,
    /// The parameter or body field at fault.
    pub field: Option<String>,
    /// How long to wait before a retry, in milliseconds.
    pub retry_ms: Option<u64>,
    details: Option<Box<Details>>,
    source: Option<Arc<dyn std::error::Error + Send + Sync + 'static>>,
    /// A stream event that does not decode: the stream goes on after it (spec 2.3, rule 9).
    event: bool,
}

/// The rarer parts of an error reply, boxed to keep [`Result`] small.
#[derive(Debug, Clone, PartialEq, Default)]
struct Details {
    retained: Option<Retained>,
    waiting: Vec<String>,
    retry_after_s: Option<f64>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "{} ({}, HTTP {status})", self.message, self.code),
            None => write!(f, "{} ({})", self.message, self.code),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.source {
            Some(source) => Some(source.as_ref()),
            None => None,
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("status", &self.status)
            .field("code", &self.code.as_str())
            .field("message", &self.message)
            .field("field", &self.field)
            .field("retry_ms", &self.retry_ms)
            .finish_non_exhaustive()
    }
}

/// The API's error body: `{"error", "code", "retry_ms"?, "field"?, "retained"?, "waiting"?}`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ErrorBody {
    /// Human-readable message.
    pub error: String,
    /// Stable code (`None` when the body has none: then it is not an API error body).
    pub code: Option<ErrorCode>,
    /// Wait before a retry, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_ms: Option<u64>,
    /// The parameter or field at fault.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The retained window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retained: Option<Retained>,
    /// What readiness waits for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting: Option<Vec<String>>,
}

/// The SDK's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// An error with this code and message and nothing else.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Error {
        Error {
            status: None,
            code,
            message: message.into(),
            field: None,
            retry_ms: None,
            details: None,
            source: None,
            event: false,
        }
    }

    /// A `transport` error (no HTTP reply) caused by `source`.
    pub fn transport(source: impl std::error::Error + Send + Sync + 'static) -> Error {
        let message = format!("no reply: {source}");
        Error::new(ErrorCode::Transport, message).with_source(source)
    }

    /// A `decode` error: the reply (HTTP `status`, if any) is not the documented format.
    pub fn decode(status: Option<u16>, message: impl Into<String>) -> Error {
        Error { status, ..Error::new(ErrorCode::Decode, message) }
    }

    /// The `decode` error of one stream event, which the stream yields and goes on after.
    pub(crate) fn event_decode(message: impl Into<String>) -> Error {
        Error { event: true, ..Error::decode(None, message) }
    }

    /// The error of one stream event that does not decode (the stream went on after it), not
    /// one that ended the stream.
    pub(crate) fn is_event(&self) -> bool {
        self.event
    }

    /// The code of an error reply that does not carry the API's JSON error body (the empty 416
    /// of `getVideoMedia`, a proxy's page), the same in every SDK: 400 and 416 `invalid_param`,
    /// 403 `cross_origin`, 404 `not_found`, 405 `method_not_allowed`, 413 `body_too_large`,
    /// 415 `json_only`, 429 `too_many_requests`, 503 `not_ready`, any other 5xx `internal`,
    /// anything else `decode`.
    pub fn code_of_status(status: u16) -> ErrorCode {
        match status {
            400 | 416 => ErrorCode::InvalidParam,
            403 => ErrorCode::CrossOrigin,
            404 => ErrorCode::NotFound,
            405 => ErrorCode::MethodNotAllowed,
            413 => ErrorCode::BodyTooLarge,
            415 => ErrorCode::JsonOnly,
            429 => ErrorCode::TooManyRequests,
            503 => ErrorCode::NotReady,
            500..=599 => ErrorCode::Internal,
            _ => ErrorCode::Decode,
        }
    }

    /// The error of an HTTP reply with `status` and `body`: the API's JSON error body when it
    /// is one, else an error whose code comes from the status ([`Error::code_of_status`]) and
    /// whose message names the status and quotes the start of the body. A 416 (a byte range the
    /// file does not have) names the field `range`.
    pub fn from_reply(status: u16, body: &[u8], retry_after_s: Option<f64>) -> Error {
        let (error, retained, waiting) = match serde_json::from_slice::<ErrorBody>(body) {
            Ok(ErrorBody { code: Some(code), error, field, retry_ms, retained, waiting }) => {
                let error = Error {
                    status: Some(status),
                    code,
                    message: error,
                    field,
                    retry_ms,
                    ..Error::new(ErrorCode::Decode, "")
                };
                (error, retained, waiting.unwrap_or_default())
            }
            _ => {
                let text = String::from_utf8_lossy(&body[..body.len().min(200)]).trim().to_string();
                let message = if status == 416 {
                    "range not satisfiable".to_string()
                } else {
                    format!("HTTP {status} without an API error body: {text:?}")
                };
                let error = Error { status: Some(status), ..Error::new(Error::code_of_status(status), message) };
                (if status == 416 { error.with_field("range") } else { error }, None, Vec::new())
            }
        };
        Error { details: Some(Box::new(Details { retained, waiting, retry_after_s })), ..error }
    }

    /// The retained frame window of a `not_retained` error.
    pub fn retained(&self) -> Option<&Retained> {
        self.details.as_ref().and_then(|d| d.retained.as_ref())
    }

    /// What the server is still waiting for, of a `not_ready` error.
    pub fn waiting(&self) -> &[String] {
        self.details.as_ref().map_or(&[], |d| &d.waiting)
    }

    /// The reply's `Retry-After`, in seconds.
    pub fn retry_after_s(&self) -> Option<f64> {
        self.details.as_ref().and_then(|d| d.retry_after_s)
    }

    /// Adds the field at fault.
    pub fn with_field(self, field: impl Into<String>) -> Error {
        Error { field: Some(field.into()), ..self }
    }

    /// Adds the HTTP status.
    pub fn with_status(self, status: u16) -> Error {
        Error { status: Some(status), ..self }
    }

    /// Adds the underlying error.
    pub fn with_source(self, source: impl std::error::Error + Send + Sync + 'static) -> Error {
        Error { source: Some(Arc::new(source)), ..self }
    }

    /// The underlying error (of a `transport` error, a file read).
    pub fn cause(&self) -> Option<&(dyn std::error::Error + Send + Sync + 'static)> {
        self.source.as_deref()
    }

    /// Worth retrying: no reply at all, or HTTP 429, 502, 503, 504.
    pub fn is_retryable(&self) -> bool {
        match self.status {
            None => self.code == ErrorCode::Transport,
            Some(status) => matches!(status, 429 | 502 | 503 | 504),
        }
    }

    /// The frame or time asked for is outside the retained window (`retained` says which).
    pub fn is_not_retained(&self) -> bool {
        self.code == ErrorCode::NotRetained
    }

    /// The session asked for has ended (another viewer selected or seeked).
    pub fn is_session_gone(&self) -> bool {
        self.code == ErrorCode::SessionGone
    }

    /// The server's catalog has no live camera.
    pub fn is_live_unavailable(&self) -> bool {
        self.code == ErrorCode::LiveUnavailable
    }

    /// How long the server asked to wait: `retry_ms`, else `Retry-After`.
    pub fn retry_delay(&self) -> Option<std::time::Duration> {
        match (self.retry_ms, self.retry_after_s()) {
            (Some(ms), _) => Some(std::time::Duration::from_millis(ms)),
            (None, Some(s)) if s.is_finite() && s >= 0.0 => Some(std::time::Duration::from_secs_f64(s.min(3600.0))),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Error {
        Error::decode(None, format!("not the documented JSON: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        crate::wire::tests::round_trip::<ErrorCode>();
        assert_eq!(ErrorCode::KNOWN.len(), 35);
    }

    #[test]
    fn api_error_bodies_parse_with_their_fields() {
        let body = br#"{"error":"frame 0 is not retained","code":"not_retained","field":"g","retained":{"session":5,"g0":1,"g1":9,"t0":0.2,"t1":1.8,"n":9}}"#;
        let e = Error::from_reply(404, body, None);
        assert_eq!(e.status, Some(404));
        assert!(e.is_not_retained());
        assert!(!e.is_session_gone());
        assert_eq!(e.field.as_deref(), Some("g"));
        assert_eq!(e.retained().unwrap().g1, Some(9));
        assert!(e.waiting().is_empty() && e.retry_after_s().is_none());
        assert!(
            Error::new(ErrorCode::Decode, "x").retained().is_none()
                && Error::new(ErrorCode::Decode, "x").waiting().is_empty()
        );
        assert_eq!(e.to_string(), "frame 0 is not retained (not_retained, HTTP 404)");
        assert!(!e.is_retryable());
        assert!(format!("{e:?}").contains("not_retained"));
        let ready = Error::from_reply(
            503,
            br#"{"error":"still starting","code":"not_ready","retry_ms":500,"waiting":["derive"]}"#,
            Some(2.0),
        );
        assert!(ready.is_retryable());
        assert_eq!(ready.waiting(), ["derive"]);
        assert_eq!(ready.retry_after_s(), Some(2.0));
        assert_eq!(ready.retry_delay(), Some(std::time::Duration::from_millis(500)));
        let gone =
            Error::from_reply(410, br#"{"error":"session 1 has ended","code":"session_gone","field":"session"}"#, None);
        assert!(gone.is_session_gone());
    }

    #[test]
    fn replies_without_an_error_body_get_their_code_from_the_status() {
        let e = Error::from_reply(416, b"", Some(1.5));
        assert_eq!((e.code.clone(), e.field.as_deref()), (ErrorCode::InvalidParam, Some("range")));
        assert_eq!(e.status, Some(416));
        assert_eq!(e.message, "range not satisfiable");
        assert_eq!(e.retry_delay(), Some(std::time::Duration::from_millis(1500)));
        let html = Error::from_reply(502, b"<html>bad gateway</html>", Some(f64::NAN));
        assert!(html.is_retryable());
        assert_eq!(html.code, ErrorCode::Internal);
        assert!(html.message.contains("bad gateway"));
        assert_eq!(html.retry_delay(), None);
        let no_code = Error::from_reply(500, br#"{"error":"x"}"#, None);
        assert_eq!(no_code.code, ErrorCode::Internal);
        for (status, code) in [
            (400, ErrorCode::InvalidParam),
            (403, ErrorCode::CrossOrigin),
            (404, ErrorCode::NotFound),
            (405, ErrorCode::MethodNotAllowed),
            (413, ErrorCode::BodyTooLarge),
            (415, ErrorCode::JsonOnly),
            (418, ErrorCode::Decode),
            (429, ErrorCode::TooManyRequests),
            (503, ErrorCode::NotReady),
            (504, ErrorCode::Internal),
        ] {
            let e = Error::from_reply(status, b"<html>proxy</html>", None);
            assert_eq!((e.code, e.status, e.field), (code, Some(status), None), "HTTP {status}");
        }
        let unknown = Error::from_reply(400, br#"{"error":"x","code":"brand_new"}"#, None);
        assert_eq!(unknown.code, ErrorCode::Other("brand_new".into()));
    }

    #[test]
    fn sdk_errors() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let e = Error::transport(io);
        assert!(e.is_retryable());
        assert_eq!(e.status, None);
        assert!(e.cause().unwrap().to_string().contains("refused"));
        assert!(std::error::Error::source(&e).is_some());
        assert_eq!(e.to_string(), "no reply: refused (transport)");
        let live = Error::new(ErrorCode::LiveUnavailable, "no live camera").with_field("video").with_status(404);
        assert!(live.is_live_unavailable());
        assert_eq!(live.field.as_deref(), Some("video"));
        assert!(!Error::new(ErrorCode::Decode, "x").is_retryable());
        let json: Error = serde_json::from_str::<u8>("x").unwrap_err().into();
        assert_eq!(json.code, ErrorCode::Decode);
        assert!(json.cause().is_none());
    }
}
