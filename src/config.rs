//! YAML configuration for tag operations.
//!
//! ```yaml
//! tags:
//!   copy:
//!     from: artist
//!     to: albumartist
//! ```

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::tag_field::TagField;

/// A parsed config file. The top-level `tags` key is required.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Tag operation settings.
    pub tags: Tags,
}

/// The `tags` section; each operation reads only its own key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tags {
    /// Settings for `idk copy tags`.
    pub copy: Option<CopySpec>,
}

/// `tags.copy`: the fields `idk copy tags` copies between.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopySpec {
    /// Field to read the value from.
    pub from: TagField,
    /// Field to write the value to.
    pub to: TagField,
}

/// A config problem, reported with the file it came from.
#[derive(Debug)]
pub struct ConfigError {
    path: PathBuf,
    message: String,
}

impl ConfigError {
    /// An error about the config file at `path`.
    pub fn new(path: &Path, message: impl Into<String>) -> Self {
        ConfigError {
            path: path.to_owned(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl Config {
    /// Reads and validates the config file at `path`.
    pub async fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|err| ConfigError::new(path, err.to_string()))?;
        Config::parse(&text).map_err(|message| ConfigError::new(path, message))
    }

    /// Parses config text. Errors name the key path, e.g. `tags.copy: unknown field ...`.
    fn parse(text: &str) -> Result<Config, String> {
        serde_yaml_ng::from_str(text).map_err(|err| err.to_string())
    }

    /// The `tags.copy` section, or an error naming the missing key.
    pub fn copy(&self) -> Result<&CopySpec, String> {
        self.tags
            .copy
            .as_ref()
            .ok_or_else(|| "missing `tags.copy`".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_copy_section() {
        let config =
            Config::parse("tags:\n  copy:\n    from: TPE1\n    to: album-artist\n").unwrap();
        let copy = config.copy().unwrap();
        assert_eq!(copy.from, TagField::Artist);
        assert_eq!(copy.to, TagField::AlbumArtist);
    }

    #[test]
    fn requires_top_level_tags() {
        let err = Config::parse("copy:\n  from: artist\n  to: title\n").unwrap_err();
        assert!(err.contains("unknown field `copy`"), "{err}");

        let err = Config::parse("{}").unwrap_err();
        assert!(err.contains("missing field `tags`"), "{err}");
    }

    #[test]
    fn rejects_unknown_keys_with_their_path() {
        let err = Config::parse("tags:\n  copy:\n    from: artist\n    too: title\n").unwrap_err();
        assert!(err.starts_with("tags.copy: unknown field `too`"), "{err}");
    }

    #[test]
    fn rejects_unknown_tag_names_with_their_path() {
        let err = Config::parse("tags:\n  copy:\n    from: artists\n    to: title\n").unwrap_err();
        assert!(
            err.starts_with("tags.copy: unknown tag \"artists\""),
            "{err}"
        );
    }

    #[test]
    fn reports_missing_section() {
        let config = Config::parse("tags: {}\n").unwrap();
        assert_eq!(config.copy().unwrap_err(), "missing `tags.copy`");
    }
}
