//! Tests your own code against the SDK's fake unit: it answers the API from a recording, in
//! process (no socket, no unit), and injects faults.
//!
//!     cargo run --example testing_quickstart --features testing
use std::time::Duration;

use pdigger_factory_sdk::models::EventKind;
use pdigger_factory_sdk::testing::{FakeFactory, Fault};

#[tokio::main(flavor = "current_thread")]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let fake = FakeFactory::from_recording("fixtures/recordings/dry1-0296-0306-full.ndjson.gz")?;
    let factory = fake.client();
    let lift = factory.interpreter().wait_for(EventKind::Lift, Duration::from_secs(5)).await?.expect("a lift");
    assert_eq!(lift.describe(), "5:01.6 Right hand lifts stone 643");
    println!("{}", lift.describe());
    fake.fail("getSession", Fault::status(503, "busy").times(1)); // one 503, then the retry succeeds
    let state = factory.session().state().await?;
    println!("session {} on {} after {} requests", state.session, state.video, fake.operations().len());
    Ok(())
}
