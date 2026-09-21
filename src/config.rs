//! YAML configuration for tag operations.
//!
//! ```yaml
//! tags:
//!   merge:
//!     genres:
//!       from: ["Heavy Metal", "Metal"]
//!       to: Rock
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
    /// Settings for `idk merge`.
    pub merge: Option<Merge>,
    /// Settings for `idk copy tags`.
    pub copy: Option<CopySpec>,
}

/// `tags.merge`: one rule per mergeable field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Merge {
    /// Rule for `idk merge genres`.
    pub genres: Option<MergeSpec>,
    /// Rule for `idk merge artists`.
    pub artists: Option<MergeSpec>,
}

/// A merge rule: every value in `from` becomes `to`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeSpec {
    /// Values to replace.
    pub from: Vec<String>,
    /// The single replacement value.
    pub to: String,
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

    /// The validated `tags.merge.<key>` section, or an error naming the offending key.
    pub fn merge(&self, key: &str) -> Result<&MergeSpec, String> {
        let spec = self
            .tags
            .merge
            .as_ref()
            .and_then(|merge| match key {
                "genres" => merge.genres.as_ref(),
                "artists" => merge.artists.as_ref(),
                _ => None,
            })
            .ok_or_else(|| format!("missing `tags.merge.{key}`"))?;
        if spec.from.is_empty() {
            return Err(format!(
                "tags.merge.{key}.from: must list at least one value"
            ));
        }
        if spec.to.trim().is_empty() {
            return Err(format!("tags.merge.{key}.to: must not be empty"));
        }
        Ok(spec)
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
    fn parses_merge_sections() {
        let yaml = "tags:\n  merge:\n    genres:\n      from: [Heavy Metal, Metal]\n      to: Rock\n    artists:\n      from: [Beatles]\n      to: The Beatles\n";
        let config = Config::parse(yaml).unwrap();

        let genres = config.merge("genres").unwrap();
        assert_eq!(genres.from, ["Heavy Metal", "Metal"]);
        assert_eq!(genres.to, "Rock");
        assert_eq!(config.merge("artists").unwrap().to, "The Beatles");
    }

    #[test]
    fn merge_to_must_be_a_single_string() {
        let err =
            Config::parse("tags:\n  merge:\n    genres:\n      from: [Metal]\n      to: [Rock]\n")
                .unwrap_err();
        assert!(
            err.starts_with("tags.merge.genres.to: invalid type: sequence"),
            "{err}"
        );
    }

    #[test]
    fn validates_merge_values() {
        let empty_from =
            Config::parse("tags:\n  merge:\n    genres:\n      from: []\n      to: Rock\n")
                .unwrap();
        assert_eq!(
            empty_from.merge("genres").unwrap_err(),
            "tags.merge.genres.from: must list at least one value"
        );

        let empty_to =
            Config::parse("tags:\n  merge:\n    genres:\n      from: [Metal]\n      to: \" \"\n")
                .unwrap();
        assert_eq!(
            empty_to.merge("genres").unwrap_err(),
            "tags.merge.genres.to: must not be empty"
        );

        let config =
            Config::parse("tags:\n  merge:\n    genres:\n      from: [Metal]\n      to: Rock\n")
                .unwrap();
        assert_eq!(
            config.merge("artists").unwrap_err(),
            "missing `tags.merge.artists`"
        );
    }

    #[test]
    fn rejects_unknown_merge_fields() {
        let err =
            Config::parse("tags:\n  merge:\n    genre:\n      from: [Metal]\n      to: Rock\n")
                .unwrap_err();
        assert!(
            err.starts_with("tags.merge: unknown field `genre`"),
            "{err}"
        );
    }

    #[test]
    fn reports_missing_section() {
        let config = Config::parse("tags: {}\n").unwrap();
        assert_eq!(config.copy().unwrap_err(), "missing `tags.copy`");
    }
}
