//! Reads and validates .skill-tree.toml.
//!
//! Two types carry configuration through the application:
//!
//! - [`Config`] -- the raw parsed TOML. Just data.
//! - [`SkillTree`] -- the application context. Wraps `Config` with
//!   resolved paths and provides the methods the rest of the pipeline calls.
//!
//! ## Field auto-discovery.
//!
//! skill-tree fetches ALL custom fields GitHub returns for every project item.
//! `[[field]]` entries are display declarations only -- they give a field a
//! friendly `display-name` for CLI output.
//! Fields not declared in `[[field]]` are still fetched and stored on each node.

use crate::error::config::{ConfigError, ConfigIssue};
use crate::github::projects::{FieldKind, ProjectMeta};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

type Fallible<T> = Result<T, ConfigError>;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub github: GithubConfig,
    #[serde(default, rename = "field")]
    pub fields: Vec<FieldConfig>,
    #[serde(default)]
    pub colors: ColorsConfig,
    #[serde(default)]
    pub cluster: ClusterConfig,
    #[serde(default)]
    pub edges: EdgesConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubConfig {
    /// GitHub organization or user that owns the project.
    ///
    /// For `github.com/orgs/rust-lang/projects/42` -> `rust-lang`.
    pub owner: String,

    /// Project number from the GitHub Projects URL.
    ///
    /// For `github.com/orgs/rust-lang/projects/42` -> `42`.
    pub project: u64,
}

/// Declares one GitHub Project custom field that skill-tree should read.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FieldConfig {
    #[serde(rename = "display-name")]
    pub display_name: String,

    /// Exact field name as it appears in GitHub Projects.
    ///
    /// Case-sensitive. Must match the field header in GitHub Projects.
    #[serde(rename = "github-name")]
    pub github_name: String,
}

/// Controls node color in the rendered graph.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ColorsConfig {
    /// Which GitHub field drives node color.
    #[serde(rename = "github-name", default)]
    pub github_name: String,

    /// Maps field option values to hex colors.
    ///
    /// Keys are the option names from the GitHub Projects single-select field.
    /// Nodes whose value is not in this map render with the default gray.
    #[serde(default)]
    pub values: HashMap<String, String>,
}

/// Groups nodes into Graphviz subgraphs by the value of a GitHub field.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ClusterConfig {
    /// Which GitHub field drives cluster membership.
    #[serde(rename = "github-name", default)]
    pub github_name: String,

    /// Maps option names to friendly display labels for the cluster box.
    ///
    /// Unmapped options render with the raw option name.
    #[serde(default)]
    pub values: HashMap<String, String>,
}

/// Configures edge sources beyond sub-issues and blockers.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct EdgesConfig {
    #[serde(rename = "cross-ref", default)]
    pub cross_ref: CrossRefConfig,
}

/// Filters which GitHub cross-references become edges.
///
/// Cross-references are noisy by default (every `Fixes #123` mention).
/// `require_labels` opts in: a cross-reference becomes an edge only if its
/// source issue/PR carries at least one of the listed labels. An empty list
/// (the default) drops every cross-reference.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CrossRefConfig {
    #[serde(rename = "require-labels", default)]
    pub require_labels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkillTree {
    /// The parsed configuration.
    pub config: Config,

    /// Directory containing the config file. Used to resolve relative paths.
    config_dir: PathBuf,
}

impl SkillTree {
    /// The default filename skill-tree looks for.
    pub const CONFIG_FILENAME: &'static str = ".skill-tree.toml";

    /// Load config from `.skill-tree.toml` in `dir`.
    ///
    /// If the file does not exist, return an error
    pub fn from_dir(dir: impl AsRef<Path>) -> Fallible<Self> {
        let dir = dir.as_ref();
        Self::from_path(dir.join(Self::CONFIG_FILENAME))
    }

    /// Walk up from `start` looking for `.skill-tree.toml`. Returns the
    /// first match. The default entry point for `skill-tree render` when
    /// the user has not passed `--config`.
    pub fn discover(start: impl AsRef<Path>) -> Fallible<Self> {
        let start = start.as_ref();
        for dir in start.ancestors() {
            let candidate = dir.join(Self::CONFIG_FILENAME);
            if candidate.is_file() {
                return Self::from_path(candidate);
            }
        }
        Err(ConfigError::NotFound {
            start: start.to_path_buf(),
        })
    }

    /// Load config from an explicit file path.
    pub fn from_path(path: impl AsRef<Path>) -> Fallible<Self> {
        let path = path.as_ref();

        let content = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;

        let config: Config = toml::from_str(&content).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;

        config.validate()?;

        let config_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        Ok(Self { config, config_dir })
    }

    /// Directory containing the config file.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Return the hex color for a field option value.
    ///
    /// Returns `None` if no color is configured for this value -- the
    /// renderer falls back to the default gray.
    pub fn color_for_value(&self, value: &str) -> Option<&str> {
        self.config.colors.values.get(value).map(String::as_str)
    }

    /// Returns the `github_name` of the field that drives node color.
    pub fn color_field_github_name(&self) -> &str {
        &self.config.colors.github_name
    }

    /// Look up fields by its `display-name`.
    ///
    /// Returns `None` if no field with the given display name is found.
    pub fn field_by_display_name(&self, display_name: &str) -> Option<&FieldConfig> {
        self.config
            .fields
            .iter()
            .find(|fconf| fconf.display_name == display_name)
    }
}

impl Config {
    fn validate(&self) -> Fallible<()> {
        for (key, value) in &self.colors.values {
            if !is_valid_hex_color(value) {
                return Err(ConfigError::InvalidColor {
                    key: key.clone(),
                    value: value.clone(),
                });
            }
        }

        Ok(())
    }

    /// Cross-check this config against the project metadata returned by
    /// GitHub. Collects every mismatch — missing field names, wrong field
    /// kinds, value names that aren't options of the SingleSelect — into a
    /// single `Vec<ConfigIssue>` so the user fixes them in one editing
    /// session.
    pub fn validate_against(&self, meta: &ProjectMeta) -> Result<(), Vec<ConfigIssue>> {
        let mut issues = Vec::new();

        check_single_select_field(
            "colors",
            "colors.values",
            &self.colors.github_name,
            self.colors.values.keys(),
            meta,
            &mut issues,
        );

        check_single_select_field(
            "cluster",
            "cluster.values",
            &self.cluster.github_name,
            self.cluster.values.keys(),
            meta,
            &mut issues,
        );

        for field_config in &self.fields {
            if meta.field_by_name(&field_config.github_name).is_none() {
                issues.push(ConfigIssue::FieldNotFound {
                    section: "field",
                    name: field_config.github_name.clone(),
                });
            }
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }
}

/// Records issues for a config section that points at a SingleSelect field
/// and lists option names under it (`colors`, `cluster`). Empty `field_name`
/// means the section is unset — skip.
fn check_single_select_field<'a>(
    section: &'static str,
    values_section: &'static str,
    field_name: &str,
    option_keys: impl Iterator<Item = &'a String>,
    meta: &ProjectMeta,
    issues: &mut Vec<ConfigIssue>,
) {
    if field_name.is_empty() {
        return;
    }
    match meta.field_by_name(field_name) {
        None => issues.push(ConfigIssue::FieldNotFound {
            section,
            name: field_name.to_owned(),
        }),
        Some(field) => match &field.kind {
            FieldKind::SingleSelect { options } => {
                for key in option_keys {
                    if !options.iter().any(|o| &o.name == key) {
                        issues.push(ConfigIssue::OptionNotFound {
                            section: values_section,
                            field: field_name.to_owned(),
                            value: key.clone(),
                        });
                    }
                }
            }
            other => issues.push(ConfigIssue::FieldWrongType {
                section,
                name: field_name.to_owned(),
                expected: "SingleSelect",
                actual: field_kind_name(other),
            }),
        },
    }
}

fn field_kind_name(kind: &FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "Text",
        FieldKind::Number => "Number",
        FieldKind::Date => "Date",
        FieldKind::SingleSelect { .. } => "SingleSelect",
        FieldKind::Iteration { .. } => "Iteration",
        FieldKind::Unknown => "Unknown",
    }
}

fn is_valid_hex_color(color: &str) -> bool {
    let Some(hex) = color.strip_prefix('#') else {
        return false;
    };

    matches!(hex.len(), 3 | 6) && hex.chars().all(|hc| hc.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use tempfile::tempdir;

    fn parse(toml: &str) -> Config {
        toml::from_str(toml).expect("test TOML should be valid")
    }

    fn valid_toml() -> &'static str {
        indoc! {"
            [github]
            owner   = \"rust-lang\"
            project = 42

            [[field]]
            display-name = \"status\"
            github-name = \"Status\"

            [[field]]
            display-name = \"priority\"
            github-name = \"Priority\"

            [colors]
            github-name = \"Status\"

            [colors.values]
            \"In Progress\" = \"#4a90d9\"
            \"Blocked\" = \"#e05252\"
            \"Complete\" = \"#57a85a\"
        "}
    }

    fn minimal_toml() -> &'static str {
        indoc! {"
            [github]
            owner   = \"nikomatsakis\"
            project = 1
        "}
    }

    #[test]
    fn parses_github_section() {
        let config = parse(valid_toml());
        assert_eq!(config.github.owner, "rust-lang");
        assert_eq!(config.github.project, 42);
    }

    #[test]
    fn parses_multiple_fields() {
        let config = parse(valid_toml());
        assert_eq!(config.fields.len(), 2);
        assert_eq!(config.fields[0].display_name, "status");
        assert_eq!(config.fields[0].github_name, "Status");
        assert_eq!(config.fields[1].display_name, "priority");
        assert_eq!(config.fields[1].github_name, "Priority");
    }

    #[test]
    fn parses_colors_section() {
        let config = parse(valid_toml());
        assert_eq!(config.colors.github_name, "Status");
        assert_eq!(
            config.colors.values.get("In Progress").map(String::as_str),
            Some("#4a90d9")
        );
    }

    #[test]
    fn minimal_config_is_valid() {
        // No [[field]] and no [colors] -- both are optional after
        // introducing field auto-discovery.
        let config = parse(minimal_toml());
        assert!(config.validate().is_ok());
        assert!(config.fields.is_empty());
        assert!(config.colors.github_name.is_empty());
    }

    #[test]
    fn config_without_fields_is_valid() {
        // [[field]] is optional -- skill-tree fetches all fields regardless.
        let config = parse(indoc! {"
            [github]
            owner   = \"rust-lang\"
            project = 42

            [colors]
            github-name = \"Status\"
        "});
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_passes_on_valid_config() {
        let config = parse(valid_toml());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_fails_on_invalid_hex_color() {
        let config = parse(indoc! {"
            [github]
            owner   = \"rust-lang\"
            project = 42

            [[field]]
            display-name = \"status\"
            github-name  = \"Status\"

            [colors]
            github-name = \"Status\"

            [colors.values]
            \"In Progress\" = \"blue\"
        "});
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidColor { .. })
        ));
    }

    #[test]
    fn from_dir_loads_config_file() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join(".skill-tree.toml"), valid_toml()).unwrap();

        let st = SkillTree::from_dir(tmp.path()).unwrap();
        assert_eq!(st.config.github.owner, "rust-lang");
        assert_eq!(st.config_dir(), tmp.path());
    }

    #[test]
    fn from_dir_fails_when_file_missing() {
        let tmp = tempdir().unwrap();
        assert!(matches!(
            SkillTree::from_dir(tmp.path()),
            Err(ConfigError::Io { .. })
        ));
    }

    #[test]
    fn color_for_value_returns_hex() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join(".skill-tree.toml"), valid_toml()).unwrap();
        let st = SkillTree::from_dir(tmp.path()).unwrap();

        assert_eq!(st.color_for_value("In Progress"), Some("#4a90d9"));
        assert_eq!(st.color_for_value("Unknown"), None);
    }

    #[test]
    fn color_for_value_returns_none_when_colors_not_configured() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join(".skill-tree.toml"), minimal_toml()).unwrap();
        let st = SkillTree::from_dir(tmp.path()).unwrap();

        assert_eq!(st.color_for_value("In Progress"), None);
    }

    #[test]
    fn field_by_display_name_finds_declared_field() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join(".skill-tree.toml"), valid_toml()).unwrap();
        let st = SkillTree::from_dir(tmp.path()).unwrap();

        let field = st.field_by_display_name("status").unwrap();
        assert_eq!(field.github_name, "Status");
        assert!(st.field_by_display_name("nonexistent").is_none());
    }

    #[test]
    fn deny_unknown_fields_on_field_config() {
        let result: Result<Config, _> = toml::from_str(indoc! {"
            [github]
            owner   = \"rust-lang\"
            project = 42

            [[field]]
            display-name = \"status\"
            github-name  = \"Status\"
            unknown-key  = \"oops\"

            [colors]
            github-name = \"Status\"
        "});
        assert!(result.is_err());
    }

    #[test]
    fn hex_color_validation() {
        assert!(is_valid_hex_color("#4a90d9"));
        assert!(is_valid_hex_color("#fff"));
        assert!(is_valid_hex_color("#FFF"));
        assert!(is_valid_hex_color("#AABBCC"));
        assert!(!is_valid_hex_color("blue"));
        assert!(!is_valid_hex_color("#12345"));
        assert!(!is_valid_hex_color("#gggggg"));
        assert!(!is_valid_hex_color(""));
        assert!(!is_valid_hex_color("#"));
    }

    // -- validate_against --

    use crate::github::projects::{FieldOption, OwnerKind, ProjectField};

    fn meta_with_status_field() -> ProjectMeta {
        ProjectMeta {
            id: "PVT_1".into(),
            title: "Test".into(),
            owner_kind: OwnerKind::Organization,
            fields: vec![
                ProjectField {
                    id: "F_status".into(),
                    name: "Status".into(),
                    kind: FieldKind::SingleSelect {
                        options: vec![
                            FieldOption {
                                id: "o1".into(),
                                name: "Done".into(),
                            },
                            FieldOption {
                                id: "o2".into(),
                                name: "In Progress".into(),
                            },
                            FieldOption {
                                id: "o3".into(),
                                name: "Blocked".into(),
                            },
                        ],
                    },
                },
                ProjectField {
                    id: "F_priority".into(),
                    name: "Priority".into(),
                    kind: FieldKind::Number,
                },
            ],
        }
    }

    #[test]
    fn validate_against_passes_on_minimal_config() {
        let config = parse(minimal_toml());
        assert!(config.validate_against(&meta_with_status_field()).is_ok());
    }

    #[test]
    fn validate_against_passes_when_colors_field_and_values_match() {
        let config = parse(valid_toml());
        // valid_toml() declares Status as the colors field with values
        // "In Progress", "Blocked", "Complete". meta has Done, In Progress,
        // Blocked. So "Complete" mismatches.
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            ConfigIssue::OptionNotFound { value, .. } if value == "Complete"
        ));
    }

    #[test]
    fn validate_against_collects_field_not_found() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [colors]
            github-name = \"Statu\"
        "});
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            ConfigIssue::FieldNotFound { section, name }
                if *section == "colors" && name == "Statu"
        ));
    }

    #[test]
    fn validate_against_collects_field_wrong_type() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [colors]
            github-name = \"Priority\"
        "});
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            ConfigIssue::FieldWrongType { name, expected, actual, .. }
                if name == "Priority" && *expected == "SingleSelect" && *actual == "Number"
        ));
    }

    #[test]
    fn validate_against_collects_multiple_issues_in_one_pass() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [[field]]
            display-name = \"prio\"
            github-name  = \"Priorityy\"

            [colors]
            github-name = \"Statu\"

            [colors.values]
            \"In Progress\" = \"#4a90d9\"
        "});
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        // colors field "Statu" doesn't exist; field "Priorityy" doesn't exist
        // (the colors.values entry is skipped because the field lookup
        // already failed — it would otherwise be a third issue).
        assert_eq!(issues.len(), 2);
    }

    // -- cluster + edges parsing --

    #[test]
    fn parses_cluster_section() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [cluster]
            github-name = \"Area\"

            [cluster.values]
            \"compiler-frontend\" = \"Frontend\"
            \"compiler-mir\"      = \"MIR\"
        "});
        assert_eq!(config.cluster.github_name, "Area");
        assert_eq!(
            config
                .cluster
                .values
                .get("compiler-frontend")
                .map(String::as_str),
            Some("Frontend"),
        );
        assert_eq!(config.cluster.values.len(), 2);
    }

    #[test]
    fn parses_edges_cross_ref_section() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [edges.cross-ref]
            require-labels = [\"tracking-issue\", \"meta\"]
        "});
        assert_eq!(
            config.edges.cross_ref.require_labels,
            vec!["tracking-issue", "meta"],
        );
    }

    #[test]
    fn cluster_and_edges_default_when_absent() {
        let config = parse(minimal_toml());
        assert!(config.cluster.github_name.is_empty());
        assert!(config.cluster.values.is_empty());
        assert!(config.edges.cross_ref.require_labels.is_empty());
    }

    // -- validate_against cluster --

    #[test]
    fn validate_against_passes_when_cluster_field_matches() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [cluster]
            github-name = \"Status\"

            [cluster.values]
            \"Done\"        = \"Done\"
            \"In Progress\" = \"Active\"
        "});
        assert!(config.validate_against(&meta_with_status_field()).is_ok());
    }

    #[test]
    fn validate_against_collects_cluster_field_not_found() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [cluster]
            github-name = \"Aera\"
        "});
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        assert!(matches!(
            &issues[0],
            ConfigIssue::FieldNotFound { section, name }
                if *section == "cluster" && name == "Aera"
        ));
    }

    #[test]
    fn validate_against_collects_cluster_field_wrong_type() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [cluster]
            github-name = \"Priority\"
        "});
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        assert!(matches!(
            &issues[0],
            ConfigIssue::FieldWrongType { section, name, .. }
                if *section == "cluster" && name == "Priority"
        ));
    }

    #[test]
    fn validate_against_flags_unknown_cluster_option_values() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [cluster]
            github-name = \"Status\"

            [cluster.values]
            \"Done\"     = \"Done\"
            \"Bogus\"    = \"x\"
        "});
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            ConfigIssue::OptionNotFound { section, value, .. }
                if *section == "cluster.values" && value == "Bogus"
        ));
    }

    #[test]
    fn validate_against_flags_each_unknown_option_value() {
        let config = parse(indoc! {"
            [github]
            owner   = \"o\"
            project = 1

            [colors]
            github-name = \"Status\"

            [colors.values]
            \"Done\"      = \"#57a85a\"
            \"Don done\"  = \"#e05252\"
            \"On Track\"  = \"#4a90d9\"
        "});
        let issues = config
            .validate_against(&meta_with_status_field())
            .unwrap_err();
        // "Done" is fine. "Don done" and "On Track" are not options.
        assert_eq!(issues.len(), 2);
        for issue in &issues {
            assert!(matches!(issue, ConfigIssue::OptionNotFound { .. }));
        }
    }
}
