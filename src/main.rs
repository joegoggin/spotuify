//! Binary entrypoint for the interactive `spotuify` terminal application.

use spotuify::AppResult;
use spotuify::app::Model;

fn main() {
    if let Err(err) = run() {
        eprintln!("spotuify: {err}");
        std::process::exit(1);
    }
}

/// Creates and runs the main app model until exit.
fn run() -> AppResult<()> {
    Model::new()?.run()
}
