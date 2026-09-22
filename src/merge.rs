//! `idk merge genres|artists`: replace variant values of a field with one value.

use std::collections::HashSet;
use std::path::Path;
use std::process::ExitCode;

use id3::{Tag, TagLike, Version};

use crate::cli::{MergeArgs, MergeTarget};
use crate::config::{Config, ConfigError};
use crate::input;
use crate::outcome::{Plan, run_writes};
use crate::tag_field::TagField;

/// A merge rule with its `from` values normalized for matching.
#[derive(Debug)]
pub struct Rule {
    from: HashSet<String>,
    to: String,
}

impl Rule {
    /// A rule replacing any of `from` with `to`.
    pub fn new(from: &[String], to: &str) -> Self {
        Rule {
            from: from.iter().map(|value| normalize(value)).collect(),
            to: to.to_owned(),
        }
    }

    /// Whether `value` should be replaced. A missing value matches `""`.
    fn matches(&self, value: Option<&str>) -> bool {
        self.from.contains(&normalize(value.unwrap_or_default()))
    }
}

/// Matching ignores case and surrounding whitespace.
fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

/// The value to match against. Genre references such as `(9)` resolve to their names.
fn current_value(field: &TagField, tag: &Tag) -> Option<String> {
    match field {
        TagField::Genre => tag.genre_parsed().map(|genre| genre.into_owned()),
        field => field.read(tag),
    }
}

/// Reads the file at `path` and works out the effect of applying `rule` to `field`.
///
/// Nothing is written; see [`Plan::apply`]. Files whose value doesn't match are
/// unchanged. A file without a tag gets a new ID3v2.4 tag only when `""` matches.
pub fn plan_merge(path: &Path, field: &TagField, rule: &Rule) -> id3::Result<Plan> {
    let tag = id3::no_tag_ok(Tag::read_from_path(path))?;
    let current = tag.as_ref().and_then(|tag| current_value(field, tag));
    if !rule.matches(current.as_deref()) {
        return Ok(Plan::Unchanged);
    }
    let mut tag = tag.unwrap_or_else(|| Tag::with_version(Version::Id3v24));
    let before = tag.clone();
    field.write(&mut tag, &rule.to);
    Ok(Plan::from_diff([field], &before, tag))
}

/// The merge rule: `--from`/`--to`, or `tags.merge.<key>` in `--config`.
async fn rule(args: &MergeArgs, key: &str) -> Result<Rule, ConfigError> {
    let Some(path) = &args.config else {
        let to = args
            .to
            .as_deref()
            .expect("clap requires --to without --config");
        return Ok(Rule::new(&args.from, to));
    };
    let config = Config::load(path).await?;
    let spec = config
        .merge(key)
        .map_err(|message| ConfigError::new(path, message))?;
    Ok(Rule::new(&spec.from, &spec.to))
}

/// Runs `idk merge genres|artists` over every input file and returns the process exit code.
pub async fn run(target: MergeTarget) -> ExitCode {
    let (field, key, args) = target.into_parts();
    let rule = match rule(&args, key).await {
        Ok(rule) => rule,
        Err(err) => return err.report(),
    };
    let inputs = input::resolve(args.input).await;
    run_writes(inputs, &args.run, &args.write, None, move |path| {
        plan_merge(path, &field, &rule).map_err(|err| err.to_string())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Change;
    use crate::outcome::Outcome;
    use id3::Frame;
    use std::path::PathBuf;
    use tempfile::TempDir;

    const AUDIO: &[u8] = &[0xFF, 0xFB, 0x90, 0x64, 0x00, 0x11];

    fn mp3(dir: &TempDir, build: impl FnOnce(&mut Tag)) -> PathBuf {
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, AUDIO).unwrap();
        let mut tag = Tag::new();
        build(&mut tag);
        tag.write_to_path(&path, Version::Id3v24).unwrap();
        path
    }

    fn rule(from: &[&str], to: &str) -> Rule {
        let from: Vec<String> = from.iter().map(|v| v.to_string()).collect();
        Rule::new(&from, to)
    }

    fn merge(path: &Path, field: TagField, rule: &Rule) -> Outcome {
        plan_merge(path, &field, rule)
            .unwrap()
            .apply(path, false)
            .unwrap()
    }

    #[test]
    fn matches_case_and_whitespace_insensitively() {
        let rule = rule(&["Heavy Metal", "metal"], "Rock");
        assert!(rule.matches(Some("heavy metal")));
        assert!(rule.matches(Some("  METAL ")));
        assert!(!rule.matches(Some("Metalcore")));
        assert!(!rule.matches(None));
    }

    #[test]
    fn empty_source_matches_missing_and_blank_values() {
        let rule = rule(&[""], "Unknown");
        assert!(rule.matches(None));
        assert!(rule.matches(Some("  ")));
        assert!(!rule.matches(Some("Rock")));
    }

    #[test]
    fn replaces_matching_genre_and_preserves_other_frames() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| {
            tag.set_genre("Heavy Metal");
            tag.set_artist("Artist");
            tag.set_text("TENC", "Encoder");
        });
        let before: Vec<Frame> = Tag::read_from_path(&path)
            .unwrap()
            .frames()
            .filter(|f| f.id() != "TCON")
            .cloned()
            .collect();

        let outcome = merge(&path, TagField::Genre, &rule(&["heavy metal"], "Rock"));

        assert_eq!(
            outcome,
            Outcome::Updated(vec![Change {
                field: TagField::Genre,
                old: Some("Heavy Metal".into()),
                new: Some("Rock".into()),
            }])
        );
        let after = Tag::read_from_path(&path).unwrap();
        assert_eq!(after.genre(), Some("Rock"));
        let rest: Vec<Frame> = after
            .frames()
            .filter(|f| f.id() != "TCON")
            .cloned()
            .collect();
        assert_eq!(rest, before);
        assert!(std::fs::read(&path).unwrap().ends_with(AUDIO));
    }

    #[test]
    fn resolves_numeric_genre_references() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_genre("(9)"));

        let outcome = merge(&path, TagField::Genre, &rule(&["Metal"], "Rock"));

        assert!(matches!(outcome, Outcome::Updated(_)));
        assert_eq!(Tag::read_from_path(&path).unwrap().genre(), Some("Rock"));
    }

    #[test]
    fn artists_change_only_tpe1() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| {
            tag.set_artist("beatles");
            tag.set_album_artist("beatles");
        });

        merge(&path, TagField::Artist, &rule(&["Beatles"], "The Beatles"));

        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.artist(), Some("The Beatles"));
        assert_eq!(tag.album_artist(), Some("beatles"));
    }

    #[test]
    fn leaves_non_matching_and_already_merged_files_untouched() {
        let dir = TempDir::new().unwrap();
        for genre in ["Jazz", "Rock"] {
            let path = mp3(&dir, |tag| tag.set_genre(genre));
            let bytes = std::fs::read(&path).unwrap();

            let outcome = merge(&path, TagField::Genre, &rule(&["Metal", "rock"], "Rock"));

            assert_eq!(outcome, Outcome::Unchanged, "{genre}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn fills_missing_values_when_empty_source_is_listed() {
        let dir = TempDir::new().unwrap();
        let tagged = mp3(&dir, |tag| tag.set_artist("Artist"));
        merge(&tagged, TagField::Genre, &rule(&[""], "Unknown"));
        assert_eq!(
            Tag::read_from_path(&tagged).unwrap().genre(),
            Some("Unknown")
        );

        let bare = dir.path().join("bare.mp3");
        std::fs::write(&bare, AUDIO).unwrap();
        merge(&bare, TagField::Genre, &rule(&[""], "Unknown"));
        let tag = Tag::read_from_path(&bare).unwrap();
        assert_eq!(tag.genre(), Some("Unknown"));
        assert_eq!(tag.version(), Version::Id3v24);
    }

    #[test]
    fn untagged_file_without_empty_source_is_unchanged() {
        let dir = TempDir::new().unwrap();
        let bare = dir.path().join("bare.mp3");
        std::fs::write(&bare, AUDIO).unwrap();

        assert_eq!(
            merge(&bare, TagField::Genre, &rule(&["Metal"], "Rock")),
            Outcome::Unchanged
        );
        assert_eq!(std::fs::read(&bare).unwrap(), AUDIO);
    }
}
