//! # skill-tree
//!
//! skill-tree turns a GitHub Project into a visual dependency graph.
//!
//! ## Architecture
//!
//! The codebase is organized as a three-stage pipeline:
//!
//! ```text
//! GitHub API  ──►  graph::Graph  ──►  rendered output
//!  (fetch)          (model)              (render)
//! ```
//!
//! Each stage has its own module:
//!
//! - [`config`]  — reads `.skill-tree.toml`; drives all three stages
//! - [`github`]  — fetches data from the GitHub GraphQL API
//! - [`graph`]   — the platform-agnostic data model (nodes + edges)
//! - [`render`]  — turns a [`graph`] into Graphviz DOT / SVG
//!

pub mod cli;
pub mod config;
pub mod error;
pub mod github;
pub mod graph;
pub mod render;
