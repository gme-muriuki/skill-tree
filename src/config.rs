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

use crate::error::ConfigError;
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
}
