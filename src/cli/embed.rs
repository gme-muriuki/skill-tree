//! `skill-tree embed` subcommand: render the project, then wrap the SVG
//! in an HTML page with a click-to-detail side panel.
//!
//! Shares the fetch → build → validate → DOT → SVG pipeline with
//! `render` (see [`super::render::fetch_graph`]). See
//! `md/design/html-embed.md` for the design.

use std::path::PathBuf;
use std::time::Duration;

use crate::config::Config;
use crate::error::CliError;
use crate::github::GitHubClient;
use crate::render::{Shape, dot_to_svg, to_dot, to_html};

use super::render::{build_render_opts, fetch_graph, load_config, write_output};

const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Flags for `skill-tree embed`.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct EmbedArgs {
    /// Emit an embeddable `<div>` fragment (for pasting into an mdbook
    /// chapter or blog) instead of a standalone HTML document.
    #[arg(long)]
    pub fragment: bool,

    /// Destination file. When omitted, write to stdout.
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Path to `.skill-tree.toml`. When omitted, walk up from CWD looking
    /// for the file.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Entry point invoked by the dispatcher in `cli::dispatch`.
pub async fn run(args: EmbedArgs) -> Result<(), CliError> {
    let sk = load_config(args.config.as_deref())?;
    let client = GitHubClient::new(None, HTTP_TIMEOUT)?;

    let html = embed_to_html(&client, &sk.config, &args).await?;

    write_output(args.output.as_deref(), html.as_bytes())
}

/// Run the full pipeline and return the HTML. Public so integration tests
/// can wire a mock-pointed `GitHubClient` and a `Config` from a TOML
/// string (no disk I/O), mirroring `render::render_to_bytes`.
pub async fn embed_to_html(
    client: &GitHubClient,
    config: &Config,
    args: &EmbedArgs,
) -> Result<String, CliError> {
    let (graph, project_title) = fetch_graph(client, config).await?;

    let opts = build_render_opts(config, Some(project_title));
    let dot = to_dot(&graph, &opts);
    let svg = dot_to_svg(&dot)?;

    let shape = if args.fragment {
        Shape::Fragment
    } else {
        Shape::Standalone
    };
    Ok(to_html(&graph, &svg, shape, &opts))
}
