//! `skill-tree render` subcommand: full fetch → model → render pipeline.
//!
//! See `md/design/render.md` for the CLI surface, default behavior, and
//! format-inference rules.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{Config, SkillTree};
use crate::error::{CliError, ConfigError};
use crate::github::GitHubClient;
use crate::github::issues::fetch_issue_edges;
use crate::github::projects::{ItemContent, fetch_project};
use crate::graph::Graph;
use crate::render::{DEFAULT_COLOR, RenderOpts, dot_to_svg, to_dot};

const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Flags for `skill-tree render`.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct RenderArgs {
    /// Output format. When omitted, inferred from `--output`'s extension
    /// (`.svg` → svg, `.dot` → dot) and defaults to `dot` otherwise.
    #[arg(long, value_enum)]
    pub format: Option<Format>,

    /// Destination file. When omitted, write to stdout.
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Path to `.skill-tree.toml`. When omitted, walk up from CWD looking
    /// for the file.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Output formats. Matches `--format <value>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Dot,
    Svg,
}

/// Entry point invoked by the dispatcher in `cli::dispatch`.
pub async fn run(args: RenderArgs) -> Result<(), CliError> {
    let sk = load_config(args.config.as_deref())?;
    let client = GitHubClient::new(None, HTTP_TIMEOUT)?;

    let bytes = render_to_bytes(&client, &sk.config, &args).await?;

    write_output(args.output.as_deref(), &bytes)
}

/// Run the full pipeline and return the rendered bytes. Public so
/// integration tests can wire a mock-pointed `GitHubClient` and a
/// `Config` built from a TOML string (no disk I/O).
pub async fn render_to_bytes(
    client: &GitHubClient,
    config: &Config,
    args: &RenderArgs,
) -> Result<Vec<u8>, CliError> {
    let project = fetch_project(client, config).await?;

    // Only Issue IDs go to fetch_issue_edges — the query filters at
    // `... on Issue { ... }` and `fetch_issue_edges` drops non-Issue
    // nodes internally, so passing PR / Draft / Redacted IDs would just
    // be wasted wire bytes.
    let issue_ids: Vec<String> = project
        .items
        .iter()
        .filter_map(|item| match &item.content {
            ItemContent::Issue(c) => Some(c.id.clone()),
            ItemContent::PullRequest(_) | ItemContent::DraftIssue(_) | ItemContent::Redacted => {
                None
            }
        })
        .collect();

    let edges = fetch_issue_edges(client, &issue_ids).await?;
    let graph = Graph::from_fetch(project, edges, config)?;
    graph.validate()?;

    let opts = build_render_opts(config);
    let dot = to_dot(&graph, &opts);

    match resolve_format(args.format, args.output.as_deref()) {
        Format::Dot => Ok(dot.into_bytes()),
        Format::Svg => Ok(dot_to_svg(&dot)?),
    }
}

fn load_config(config_flag: Option<&Path>) -> Result<SkillTree, CliError> {
    match config_flag {
        Some(path) => Ok(SkillTree::from_path(path)?),
        None => {
            let cwd = std::env::current_dir().map_err(ConfigError::CwdUnreadable)?;
            Ok(SkillTree::discover(cwd)?)
        }
    }
}

fn write_output(output: Option<&Path>, bytes: &[u8]) -> Result<(), CliError> {
    match output {
        Some(path) => std::fs::write(path, bytes).map_err(|source| CliError::FileWrite {
            path: path.to_path_buf(),
            source,
        }),
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(bytes).map_err(CliError::StdoutWrite)
        }
    }
}

fn build_render_opts(config: &Config) -> RenderOpts {
    RenderOpts {
        colors: config.colors.values.clone(),
        cluster_labels: config.cluster.values.clone(),
        default_color: DEFAULT_COLOR.to_string(),
    }
}

/// Permissive resolution: explicit `--format` always wins; otherwise
/// infer from `--output`'s extension; otherwise default to DOT.
pub(crate) fn resolve_format(flag: Option<Format>, output: Option<&Path>) -> Format {
    if let Some(f) = flag {
        return f;
    }
    if let Some(ext) = output.and_then(|p| p.extension()).and_then(|e| e.to_str()) {
        match ext.to_ascii_lowercase().as_str() {
            "svg" => return Format::Svg,
            "dot" => return Format::Dot,
            _ => {}
        }
    }
    Format::Dot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_format_overrides_extension() {
        assert_eq!(
            resolve_format(Some(Format::Dot), Some(Path::new("graph.svg"))),
            Format::Dot
        );
        assert_eq!(
            resolve_format(Some(Format::Svg), Some(Path::new("graph.dot"))),
            Format::Svg
        );
    }

    #[test]
    fn extension_inferred_when_flag_absent() {
        assert_eq!(
            resolve_format(None, Some(Path::new("graph.svg"))),
            Format::Svg
        );
        assert_eq!(
            resolve_format(None, Some(Path::new("graph.dot"))),
            Format::Dot
        );
    }

    #[test]
    fn extension_case_insensitive() {
        assert_eq!(
            resolve_format(None, Some(Path::new("graph.SVG"))),
            Format::Svg
        );
        assert_eq!(
            resolve_format(None, Some(Path::new("graph.Dot"))),
            Format::Dot
        );
    }

    #[test]
    fn unknown_extension_defaults_to_dot() {
        assert_eq!(
            resolve_format(None, Some(Path::new("graph.txt"))),
            Format::Dot
        );
    }

    #[test]
    fn no_extension_defaults_to_dot() {
        assert_eq!(resolve_format(None, Some(Path::new("graph"))), Format::Dot);
    }

    #[test]
    fn no_flags_defaults_to_dot() {
        assert_eq!(resolve_format(None, None), Format::Dot);
    }
}
