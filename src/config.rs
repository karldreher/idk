//! YAML configuration for tag operations.
//!
//! Every config file is validated against a JSON Schema generated from these
//! types before it is used (see [`schema`]).
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
use std::process::ExitCode;
use std::sync::LazyLock;

use jsonschema::error::ValidationErrorKind;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::tag_field::TagField;

/// A parsed config file. The top-level `tags` key is required.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "idk config")]
pub struct Config {
    /// Tag operation settings.
    pub tags: Tags,
}

/// The `tags` section; each operation reads only its own key.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tags {
    /// Settings for `idk merge`.
    #[serde(default)]
    #[schemars(with = "Merge")]
    pub merge: Option<Merge>,
    /// Settings for `idk copy tags`.
    #[serde(default)]
    #[schemars(with = "CopySpec")]
    pub copy: Option<CopySpec>,
}

/// `tags.merge`: one rule per mergeable field.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Merge {
    /// Rule for `idk merge genres`.
    #[serde(default)]
    #[schemars(with = "MergeSpec")]
    pub genres: Option<MergeSpec>,
    /// Rule for `idk merge artists`.
    #[serde(default)]
    #[schemars(with = "MergeSpec")]
    pub artists: Option<MergeSpec>,
}

/// A merge rule: every value in `from` becomes `to`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeSpec {
    /// Values to replace (case-insensitive; "" matches a missing or empty value).
    #[schemars(length(min = 1))]
    pub from: Vec<String>,
    /// The single replacement value.
    #[schemars(regex(pattern = r"\S"))]
    pub to: String,
}

/// `tags.copy`: the fields `idk copy tags` copies between.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopySpec {
    /// Field to read the value from.
    pub from: TagField,
    /// Field to write the value to.
    pub to: TagField,
}

/// The JSON Schema every config file must satisfy.
pub fn schema() -> Value {
    schemars::schema_for!(Config).to_value()
}

/// The compiled schema, built once per process.
static VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    jsonschema::validator_for(&schema()).expect("generated schema is a valid JSON Schema")
});

/// Config problems, reported with the file they came from.
#[derive(Debug)]
pub struct ConfigError {
    path: PathBuf,
    messages: Vec<String>,
}

impl ConfigError {
    /// A single error about the config file at `path`.
    pub fn new(path: &Path, message: impl Into<String>) -> Self {
        ConfigError::many(path, vec![message.into()])
    }

    /// Several errors about the config file at `path`.
    fn many(path: &Path, messages: Vec<String>) -> Self {
        ConfigError {
            path: path.to_owned(),
            messages,
        }
    }
}

impl ConfigError {
    /// Prints every error to stderr and returns exit code 1.
    pub fn report(&self) -> ExitCode {
        for line in self.to_string().lines() {
            eprintln!("error: {line}");
        }
        ExitCode::FAILURE
    }
}

/// One `path: message` line per error.
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lines: Vec<String> = self
            .messages
            .iter()
            .map(|message| format!("{}: {message}", self.path.display()))
            .collect();
        f.write_str(&lines.join("\n"))
    }
}

impl Config {
    /// Reads the config file at `path`, validates it against [`schema`], and parses it.
    pub async fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|err| ConfigError::new(path, err.to_string()))?;
        Config::parse(&text).map_err(|messages| ConfigError::many(path, messages))
    }

    /// Validates config text against the schema, reporting every violation, then parses it.
    fn parse(text: &str) -> Result<Config, Vec<String>> {
        let value: Value = serde_yaml_ng::from_str(text).map_err(|err| vec![err.to_string()])?;
        let violations: Vec<String> = VALIDATOR.iter_errors(&value).map(describe).collect();
        if !violations.is_empty() {
            return Err(violations);
        }
        serde_json::from_value(value).map_err(|err| vec![err.to_string()])
    }

    /// The `tags.merge.<key>` section, or an error naming the missing key.
    pub fn merge(&self, key: &str) -> Result<&MergeSpec, String> {
        self.tags
            .merge
            .as_ref()
            .and_then(|merge| match key {
                "genres" => merge.genres.as_ref(),
                "artists" => merge.artists.as_ref(),
                _ => None,
            })
            .ok_or_else(|| format!("missing `tags.merge.{key}`"))
    }

    /// The `tags.copy` section, or an error naming the missing key.
    pub fn copy(&self) -> Result<&CopySpec, String> {
        self.tags
            .copy
            .as_ref()
            .ok_or_else(|| "missing `tags.copy`".to_owned())
    }
}

/// `tags.merge.genres.to: ...` style message for a schema violation.
///
/// Pattern violations are reworded, since the raw message repeats the regex.
fn describe(error: jsonschema::ValidationError<'_>) -> String {
    let key = error
        .instance_path()
        .to_string()
        .trim_start_matches('/')
        .replace('/', ".");
    let message = match error.kind() {
        ValidationErrorKind::Pattern { .. }
            if error.schema_path().to_string().contains("TagField") =>
        {
            format!("{} is not a known tag name", error.instance())
        }
        ValidationErrorKind::Pattern { .. } => "must not be blank".to_owned(),
        _ => error.to_string(),
    };
    if key.is_empty() {
        message
    } else {
        format!("{key}: {message}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All error messages for `yaml`, one per line.
    fn errors(yaml: &str) -> String {
        Config::parse(yaml).unwrap_err().join("\n")
    }

    #[test]
    fn parses_copy_and_merge_sections() {
        let yaml = "tags:\n  merge:\n    genres:\n      from: [Heavy Metal, Metal]\n      to: Rock\n    artists:\n      from: [Beatles]\n      to: The Beatles\n  copy:\n    from: TPE1\n    to: Album-Artist\n";
        let config = Config::parse(yaml).unwrap();

        let genres = config.merge("genres").unwrap();
        assert_eq!(genres.from, ["Heavy Metal", "Metal"]);
        assert_eq!(genres.to, "Rock");
        assert_eq!(config.merge("artists").unwrap().to, "The Beatles");
        let copy = config.copy().unwrap();
        assert_eq!(
            (&copy.from, &copy.to),
            (&TagField::Artist, &TagField::AlbumArtist)
        );
    }

    #[test]
    fn requires_top_level_tags() {
        let err = errors("copy:\n  from: artist\n  to: title\n");
        assert!(err.contains("\"tags\" is a required property"), "{err}");
        assert!(err.contains("'copy'"), "{err}");
        assert!(errors("").contains("null is not of type \"object\""));
    }

    #[test]
    fn rejects_unknown_keys_with_their_path() {
        let err = errors("tags:\n  merge:\n    genre:\n      from: [Metal]\n      to: Rock\n");
        assert!(err.starts_with("tags.merge: "), "{err}");
        assert!(err.contains("'genre'"), "{err}");
    }

    #[test]
    fn merge_to_must_be_a_single_non_blank_string() {
        let err = errors("tags:\n  merge:\n    genres:\n      from: [Metal]\n      to: [Rock]\n");
        assert!(err.starts_with("tags.merge.genres.to: "), "{err}");
        assert!(err.contains("is not of type \"string\""), "{err}");

        let err = errors("tags:\n  merge:\n    genres:\n      from: [Metal]\n      to: \" \"\n");
        assert!(err.starts_with("tags.merge.genres.to: "), "{err}");
    }

    #[test]
    fn merge_from_must_not_be_empty() {
        let err = errors("tags:\n  merge:\n    genres:\n      from: []\n      to: Rock\n");
        assert!(err.starts_with("tags.merge.genres.from: "), "{err}");
    }

    #[test]
    fn reports_every_violation() {
        let err = errors("tags:\n  merge:\n    genres:\n      from: []\n      to: [Rock]\n");
        assert_eq!(err.lines().count(), 2, "{err}");
    }

    #[test]
    fn tag_names_are_validated_case_insensitively() {
        for name in [
            "artist",
            "ARTIST",
            "tpe1",
            "Album-Artist",
            "TYER",
            "txxx:Custom",
            "TXXX:x",
        ] {
            let yaml = format!("tags:\n  copy:\n    from: \"{name}\"\n    to: title\n");
            assert!(Config::parse(&yaml).is_ok(), "{name}");
        }
        for name in ["artists", "txxx:", "tpe"] {
            let yaml = format!("tags:\n  copy:\n    from: \"{name}\"\n    to: title\n");
            let err = errors(&yaml);
            assert!(err.starts_with("tags.copy.from: "), "{name}: {err}");
        }
    }

    #[test]
    fn schema_lists_canonical_tag_names() {
        let schema = schema().to_string();
        assert!(schema.contains("\"albumartist\""), "{schema}");
        assert!(schema.contains("\"idk config\""), "{schema}");
    }

    #[test]
    fn reports_missing_sections() {
        let config = Config::parse("tags: {}\n").unwrap();
        assert_eq!(config.copy().unwrap_err(), "missing `tags.copy`");
        assert_eq!(
            config.merge("genres").unwrap_err(),
            "missing `tags.merge.genres`"
        );
    }
}
