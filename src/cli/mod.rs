//! CLI argument definitions and dispatch.

pub mod embed;
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
    /// Render the project as an interactive HTML page with a detail panel.
    Embed(embed::EmbedArgs),
}

pub async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Render(args) => render::run(args).await,
        Command::Embed(args) => embed::run(args).await,
    }
}
