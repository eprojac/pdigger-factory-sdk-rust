//! Simple mode: one report per tray, which stones were removed, which were added and which
//! moved, when and where, and whether that is the tray's whole account. A recording needs no unit; with
//! `--follow` the same report comes from a unit as each tray leaves.
//!
//!     cargo run --example quickstart_reports -- fixtures/recordings/dry1-0018-0153.ndjson.gz
//!     cargo run --example quickstart_reports -- --follow http://factory-orin.local:8120
use futures::StreamExt;
use pdigger_factory_sdk::{FactoryClient, Replay};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let mut args = std::env::args().skip(1);
    let source = args.next().unwrap_or_else(|| "fixtures/recordings/dry1-0018-0153.ndjson.gz".to_string());
    if source != "--follow" {
        for report in Replay::open(&source)?.tray_reports()? {
            println!("{}", report.describe());
        }
        return Ok(());
    }
    let url = args.next().unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let factory = FactoryClient::new(&url)?;
    let mut reports = factory.interpreter().tray_reports(); // one as each tray leaves
    while let Some(report) = reports.next().await {
        println!("{}", report?.describe()); // serde_json::to_string(&report) is the same as data
    }
    Ok(())
}
