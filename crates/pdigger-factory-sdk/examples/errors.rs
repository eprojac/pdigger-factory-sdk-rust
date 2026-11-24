//! Errors: one type with a stable code to branch on. GET requests are retried (connection
//! errors, 429, 502, 503, 504) before an error reaches you; a control only on 429.
//!
//!     cargo run --example errors -- http://factory-orin.local:8120
use std::time::Duration;

use pdigger_factory_sdk::api::FrameQuery;
use pdigger_factory_sdk::models::EventKind;
use pdigger_factory_sdk::{FactoryClient, WireEnum};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let factory = FactoryClient::new(&url)?;
    // An API error: branch on the code, never on the message.
    match factory.api().get_frame(1, &FrameQuery::default()).await {
        Ok(frame) => println!("frame 1 is still held (session {})", frame.session),
        Err(e) if e.is_not_retained() => {
            println!("frame 1: HTTP {} {}", e.status.unwrap_or(0), e.code.as_str());
            if let Some(held) = e.retained() {
                println!("the unit holds frames {:?} to {:?} of session {}", held.g0, held.g1, held.session);
            }
        }
        Err(e) => return Err(e),
    }
    // A control the unit refuses, checked as a dry run: the shared session does not move.
    if let Err(e) = factory.session().dry_run().seek(-5.0).await {
        println!("seek to -5 s: {} (retryable: {})", e.code.as_str(), e.is_retryable());
    }
    // Time limits: wait_for gives Ok(None) when nothing came in time.
    let split = factory.interpreter().wait_for(EventKind::Split, Duration::from_millis(500)).await?;
    println!("a split within 0.5 s: {}", split.is_some());
    Ok(())
}
