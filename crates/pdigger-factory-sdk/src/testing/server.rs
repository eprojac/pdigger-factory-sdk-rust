//! A local HTTP/1.1 server in front of a [`FakeFactory`]: one request per connection,
//! chunked bodies, a dropped stream drops the socket.

use std::sync::Arc;

use futures::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use super::FakeFactory;
use crate::client::FactoryClient;
use crate::error::{Error, ErrorCode, Result};
use crate::transport::{Method, Request, Transport};

/// A [`FakeFactory`] served on `127.0.0.1`. Dropping it stops the server;
/// [`FakeServer::shutdown`] also waits for that.
#[derive(Debug)]
pub struct FakeServer {
    url: String,
    fake: FakeFactory,
    stop: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

fn io_error(e: std::io::Error) -> Error {
    Error::new(ErrorCode::Transport, format!("the fake server cannot listen: {e}")).with_source(e)
}

impl FakeServer {
    pub(super) async fn start(fake: FakeFactory) -> Result<FakeServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(io_error)?;
        let url = format!("http://{}", listener.local_addr().map_err(io_error)?);
        let (served, base, stop) = (fake.clone(), url.clone(), Arc::new(Notify::new()));
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { return };
                        tokio::spawn(connection(served.clone(), socket, base.clone()));
                    }
                    () = stopped.notified() => return,
                }
            }
        });
        Ok(FakeServer { url, fake, stop, task })
    }

    /// Stops listening and waits until the server has stopped (connections already open run
    /// to their end).
    pub async fn shutdown(mut self) {
        self.stop.notify_one();
        let _ = (&mut self.task).await;
    }

    /// The base URL (`http://127.0.0.1:<port>`).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// A client of this server over the default (`reqwest`) transport, with a seeded random
    /// source and the fake's clock.
    pub fn client(&self) -> FactoryClient {
        FactoryClient::builder(&self.url)
            .random(crate::client::SeededRandom::new(0))
            .clock(self.fake.clock())
            .build()
            .expect("a local URL is valid")
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn connection(fake: FakeFactory, socket: TcpStream, base: String) {
    // An error here is the client going away; the connection just ends.
    let _ = exchange(fake, socket, base).await;
}

async fn exchange(fake: FakeFactory, socket: TcpStream, base: String) -> std::io::Result<()> {
    let (read, mut write) = socket.into_split();
    let mut reader = BufReader::new(read);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let mut parts = line.split_whitespace();
    let method = match parts.next() {
        Some("HEAD") => Method::Head,
        Some("POST") => Method::Post,
        Some("PUT") => Method::Put,
        Some("DELETE") => Method::Delete,
        _ => Method::Get,
    };
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    let mut length = 0;
    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        let Some((name, value)) = line.trim_end().split_once(':') else { break };
        let (name, value) = (name.trim().to_string(), value.trim().to_string());
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse().unwrap_or(0);
        }
        headers.push((name, value));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    let body = (!body.is_empty()).then(|| bytes::Bytes::from(body));
    let request = Request { method, url: format!("{base}{target}"), headers, body, timeout: None };
    let Ok(mut reply) = fake.send(request).await else { return Ok(()) };
    let bodiless = method == Method::Head || reply.status == 304;
    let mut head = format!("HTTP/1.1 {} FakeFactory\r\nConnection: close\r\n", reply.status);
    for (name, value) in &reply.headers.0 {
        if bodiless || !name.eq_ignore_ascii_case("content-length") {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str(if bodiless { "\r\n" } else { "Transfer-Encoding: chunked\r\n\r\n" });
    write.write_all(head.as_bytes()).await?;
    if bodiless {
        return write.shutdown().await;
    }
    while let Some(chunk) = reply.body.next().await {
        let Ok(chunk) = chunk else { return Ok(()) };
        write.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).await?;
        write.write_all(&chunk).await?;
        write.write_all(b"\r\n").await?;
    }
    write.write_all(b"0\r\n\r\n").await?;
    write.shutdown().await
}

#[cfg(test)]
mod tests {
    #[test]
    fn listen_errors_are_transport_errors() {
        let error = super::io_error(std::io::Error::other("address in use"));
        assert_eq!(error.code, crate::ErrorCode::Transport);
        assert!(error.to_string().contains("address in use"));
    }
}
