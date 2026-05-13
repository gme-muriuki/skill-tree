//! skill-tree binary entry point. Parses CLI arguments and dispatches.

use std::process::ExitCode;

use clap::Parser as _;
use skill_tree::cli::{Cli, dispatch};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}
