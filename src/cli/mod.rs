//! CLI argument definitions and dispatch.

pub mod render;
mod unblocked;
mod validate;

use crate::error::CliError;

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

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Render the project as DOT (default) or SVG.
    Render(render::RenderArgs),
}

pub async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Render(args) => render::run(args).await,
    }
}
