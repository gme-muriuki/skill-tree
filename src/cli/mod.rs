//! CLI argument definitions and dispatch.
//!
//! Slice 3 ships only `render`; `unblocked` and `validate` will land in
//! slice 4 against the existing stub modules.

pub mod render;
mod unblocked;
mod validate;

use crate::error::CliError;

/// Top-level `skill-tree` CLI.
#[derive(Debug, clap::Parser)]
#[command(
    name = "skill-tree",
    about = "Render a GitHub Project as a directed dependency graph",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands. Only `Render` is wired up in slice 3.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Render the project as DOT (default) or SVG.
    Render(render::RenderArgs),
}

/// Entry point: dispatch the parsed CLI to its subcommand.
pub async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Render(args) => render::run(args).await,
    }
}
