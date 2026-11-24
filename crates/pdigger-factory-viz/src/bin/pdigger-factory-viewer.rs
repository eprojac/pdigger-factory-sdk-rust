//! `pdigger-factory-viewer [recording.ndjson.gz | recording URL | server URL]`: plays a recording
//! or follows a server (default `http://factory-orin.local:8120`) in a window.
//!
//! Excluded from coverage with `native.rs` (it only opens the window): reading the argument is
//! [`ViewerInput::parse`], opening it [`ViewerApp::open`] and the views [`ViewerApp`], all tested
//! headless.

use std::process::ExitCode;

use pdigger_factory_viz::{ViewerApp, ViewerInput};

fn main() -> ExitCode {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    if arg == "-h" || arg == "--help" {
        println!(
            "usage: pdigger-factory-viewer [recording.ndjson.gz | https://.../recording.ndjson.gz | http://server:8120]"
        );
        return ExitCode::SUCCESS;
    }
    let (title, app) = match ViewerApp::open(&ViewerInput::parse(&arg)) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("pdigger-factory-viewer: {error}");
            return ExitCode::FAILURE;
        }
    };
    match pdigger_factory_viz::native::run(&format!("pDigger Factory viewer: {title}"), app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pdigger-factory-viewer: {error}");
            ExitCode::FAILURE
        }
    }
}
