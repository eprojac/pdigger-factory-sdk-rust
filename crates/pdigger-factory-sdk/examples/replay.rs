//! Reads a recording offline and prints its activities and its summary.
//!
//!     cargo run --example replay -- dry1.world.ndjson.gz
use pdigger_factory_sdk::Replay;

fn main() -> pdigger_factory_sdk::Result<()> {
    let path =
        std::env::args().nth(1).unwrap_or_else(|| "fixtures/recordings/dry1-0296-0306-full.ndjson.gz".to_string());
    let out = Replay::open(&path)?.run()?; // events, activities and the summary in one pass
    for activity in &out.activities {
        println!("{}", activity.describe()); // 5:01.6 Right hand moves stone 643 1.09 units in 1.2 s
    }
    print!("{}", out.summary.describe());
    Ok(())
}
