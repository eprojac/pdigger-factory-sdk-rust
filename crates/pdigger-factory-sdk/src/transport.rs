//! HTTP transport: the trait the client sends every request through, and the default
//! implementation on `reqwest` (rustls).
//!
//! Tests and users' own unit tests replace the transport (see the `testing` module), which is
//! how everything runs without a network.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures::future::BoxFuture;
use futures::{Stream, StreamExt};

use crate::error::{Error, Result};

/// An HTTP method the API uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// GET.
    Get,
    /// HEAD.
    Head,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// DELETE.
    Delete,
}

impl Method {
    /// The method name.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }

    /// GET and HEAD: safe to retry on connection errors and 5xx.
    pub fn is_read(self) -> bool {
        matches!(self, Method::Get | Method::Head)
    }
}

/// One HTTP request.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Method.
    pub method: Method,
    /// Absolute URL.
    pub url: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: Option<Bytes>,
    /// Timeout of the whole exchange; `None` for streams.
    pub timeout: Option<Duration>,
}

impl Request {
    /// The value of a request header (case-insensitive name).
    pub fn header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Response headers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Headers(pub Vec<(String, String)>);

impl Headers {
    /// The first value of a header (case-insensitive name).
    pub fn get(&self, name: &str) -> Option<&str> {
        find_header(&self.0, name)
    }

    /// `Retry-After` in seconds (the delta-seconds form; an HTTP date gives `None`).
    pub fn retry_after_s(&self) -> Option<f64> {
        self.get("retry-after")?.trim().parse::<f64>().ok()
    }
}

type ChunkStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

/// A response body: a stream of chunks. Dropping it closes the connection.
pub struct Body(ChunkStream);

impl Body {
    /// A body of these bytes.
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Body {
        let bytes = bytes.into();
        Body(Box::pin(futures::stream::iter((!bytes.is_empty()).then(|| Ok(bytes)))))
    }

    /// A body streamed from `chunks`.
    pub fn from_stream(chunks: impl Stream<Item = Result<Bytes>> + Send + 'static) -> Body {
        Body(Box::pin(chunks))
    }

    /// The whole body.
    pub async fn bytes(mut self) -> Result<Bytes> {
        let mut all = BytesMut::new();
        while let Some(chunk) = self.0.next().await {
            all.extend_from_slice(&chunk?);
        }
        Ok(all.freeze())
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Body")
    }
}

impl Stream for Body {
    type Item = Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.as_mut().poll_next(cx)
    }
}

/// One HTTP response.
#[derive(Debug)]
pub struct Response {
    /// Status code.
    pub status: u16,
    /// Headers.
    pub headers: Headers,
    /// Body.
    pub body: Body,
}

/// Sends requests. Implemented by [`ReqwestTransport`] (the default) and by the `testing`
/// module's fakes; implement it to put the SDK on another HTTP stack.
pub trait Transport: Send + Sync + 'static {
    /// Sends one request. A reply of any status is `Ok`; `Err` means there was no reply
    /// (code `transport`).
    fn send(&self, request: Request) -> BoxFuture<'static, Result<Response>>;
}

/// The default transport: `reqwest` with rustls, HTTP/1.1; it honours `HTTP_PROXY`,
/// `HTTPS_PROXY` and `NO_PROXY` from the environment, as the other languages do.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// A transport whose connections time out after `connect_timeout`.
    pub fn new(connect_timeout: Duration) -> Result<ReqwestTransport> {
        let builder =
            reqwest::Client::builder().connect_timeout(connect_timeout).tcp_keepalive(Duration::from_secs(30));
        let client = builder.build().map_err(Error::transport)?;
        Ok(ReqwestTransport { client })
    }
}

impl Transport for ReqwestTransport {
    fn send(&self, request: Request) -> BoxFuture<'static, Result<Response>> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Head => reqwest::Method::HEAD,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Delete => reqwest::Method::DELETE,
        };
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        Box::pin(async move {
            let reply = builder.send().await.map_err(Error::transport)?;
            let status = reply.status().as_u16();
            let headers = reply
                .headers()
                .iter()
                .map(|(name, value)| {
                    (name.as_str().to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned())
                })
                .collect();
            let chunks = futures::stream::unfold(Some(reply), |state| async move {
                let mut reply = state?;
                match reply.chunk().await {
                    Ok(Some(chunk)) => Some((Ok(chunk), Some(reply))),
                    Ok(None) => None,
                    Err(error) => Some((Err(Error::transport(error)), None)),
                }
            });
            Ok(Response { status, headers: Headers(headers), body: Body::from_stream(chunks) })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_and_headers() {
        assert_eq!(
            [Method::Get, Method::Head, Method::Post, Method::Put, Method::Delete].map(Method::as_str),
            ["GET", "HEAD", "POST", "PUT", "DELETE"]
        );
        assert!(Method::Get.is_read() && Method::Head.is_read() && !Method::Post.is_read());
        let h = Headers(vec![("Retry-After".into(), " 2 ".into()), ("ETag".into(), "\"x\"".into())]);
        assert_eq!(h.get("etag"), Some("\"x\""));
        assert_eq!(h.retry_after_s(), Some(2.0));
        assert_eq!(Headers(vec![("retry-after".into(), "Wed, 21 Oct 2015 07:28:00 GMT".into())]).retry_after_s(), None);
        assert_eq!(Headers::default().retry_after_s(), None);
        let r = Request {
            method: Method::Get,
            url: "http://x/".into(),
            headers: vec![("Accept".into(), "a".into())],
            body: None,
            timeout: None,
        };
        assert_eq!(r.header("accept"), Some("a"));
        assert_eq!(r.header("nope"), None);
    }

    #[tokio::test]
    async fn bodies_collect_and_stream() {
        assert_eq!(Body::from_bytes("abc").bytes().await.unwrap(), Bytes::from("abc"));
        assert!(Body::from_bytes(Bytes::new()).bytes().await.unwrap().is_empty());
        let chunks = futures::stream::iter(vec![Ok(Bytes::from("a")), Err(Error::decode(None, "cut"))]);
        let mut body = Body::from_stream(chunks);
        assert_eq!(format!("{body:?}"), "Body");
        assert_eq!(body.next().await.unwrap().unwrap(), Bytes::from("a"));
        assert!(body.next().await.unwrap().is_err());
        let failing = Body::from_stream(futures::stream::iter(vec![Err(Error::decode(None, "cut"))]));
        assert!(failing.bytes().await.is_err());
    }
}
