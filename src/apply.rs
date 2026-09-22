//! `idk apply`: write tag edits from `idk show --json` output back to files.
//!
//! For each entry, every `tags` key naming a field is written when it differs
//! from what `idk show` would print; `null` clears the field. Keys that aren't
//! field names (frame IDs such as `TENC`, or `date#2`) are skipped with a warning.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use id3::{Tag, Version};
use serde_json::Value;

use crate::cli::ApplyArgs;
use crate::input::Inputs;
use crate::outcome::{Plan, run_writes};
use crate::show::display_value;
use crate::tag_field::TagField;

/// Exit code for unusable input, matching clap's usage errors.
const INPUT_ERROR: u8 = 2;

/// One field edit: a new value, or `None` to clear the field.
pub type Edit = (TagField, Option<String>);

/// A validated edit file.
#[derive(Debug, Default, PartialEq)]
pub struct Edits {
    /// Files and their edits, in input order.
    pub entries: Vec<(PathBuf, Vec<Edit>)>,
    /// Keys that were not field names, reported once each.
    pub skipped_keys: BTreeSet<String>,
}

/// Validates `json`, reporting every structural problem at once.
pub fn parse(json: &str) -> Result<Edits, Vec<String>> {
    let value: Value =
        serde_json::from_str(json).map_err(|err| vec![format!("invalid JSON: {err}")])?;
    let Value::Array(items) = value else {
        return Err(vec![
            "expected a JSON array of {\"path\", \"tags\"} objects, as printed by `idk show --json`"
                .to_owned(),
        ]);
    };
    let mut edits = Edits::default();
    let mut errors = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match parse_entry(index, item, &mut edits.skipped_keys) {
            Ok(entry) => edits.entries.push(entry),
            Err(mut entry_errors) => errors.append(&mut entry_errors),
        }
    }
    if errors.is_empty() {
        Ok(edits)
    } else {
        Err(errors)
    }
}

/// Validates one array element, e.g. `[3].tags.genre: expected a string or null`.
fn parse_entry(
    index: usize,
    item: &Value,
    skipped: &mut BTreeSet<String>,
) -> Result<(PathBuf, Vec<Edit>), Vec<String>> {
    let at = format!("[{index}]");
    let Value::Object(object) = item else {
        return Err(vec![format!("{at}: expected an object")]);
    };
    let mut errors = Vec::new();
    let path = match object.get("path") {
        Some(Value::String(path)) if !path.is_empty() => Some(PathBuf::from(path)),
        Some(_) => {
            errors.push(format!("{at}.path: expected a non-empty string"));
            None
        }
        None => {
            errors.push(format!("{at}: missing \"path\""));
            None
        }
    };
    let mut edits: Vec<Edit> = Vec::new();
    match object.get("tags") {
        Some(Value::Object(tags)) => {
            for (key, value) in tags {
                let value = match value {
                    Value::String(value) => Some(value.clone()),
                    Value::Null => None,
                    _ => {
                        errors.push(format!("{at}.tags.{key}: expected a string or null"));
                        continue;
                    }
                };
                let Ok(field) = key.parse::<TagField>() else {
                    skipped.insert(key.clone());
                    continue;
                };
                if edits.iter().any(|(existing, _)| *existing == field) {
                    errors.push(format!("{at}.tags.{key}: {field} is given more than once"));
                    continue;
                }
                edits.push((field, value));
            }
        }
        Some(_) => errors.push(format!("{at}.tags: expected an object")),
        None => errors.push(format!("{at}: missing \"tags\"")),
    }
    match path {
        Some(path) if errors.is_empty() => Ok((path, edits)),
        _ => Err(errors),
    }
}

/// Rejects entries that name the same file twice, which would race when applied concurrently.
fn reject_duplicate_paths(edits: &Edits) -> Result<(), Vec<String>> {
    let mut seen = HashMap::new();
    let mut errors = Vec::new();
    for (index, (path, _)) in edits.entries.iter().enumerate() {
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if let Some(first) = seen.insert(key, index) {
            errors.push(format!(
                "[{index}].path: {} is already edited by [{first}]",
                path.display()
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Reads the file at `path` and works out the effect of `edits`.
///
/// A value equal to what `idk show` would print leaves the field untouched. A
/// file without a tag gets a new ID3v2.4 tag only when a value is set.
pub fn plan_apply(path: &Path, edits: &[Edit]) -> id3::Result<Plan> {
    let sets_value = edits.iter().any(|(_, value)| value.is_some());
    let tag = id3::no_tag_ok(Tag::read_from_path(path))?;
    let Some(mut tag) = tag.or_else(|| sets_value.then(|| Tag::with_version(Version::Id3v24)))
    else {
        return Ok(Plan::Unchanged);
    };
    let before = tag.clone();
    for (field, value) in edits {
        match value {
            Some(value) => {
                let shown = field.read(&tag).map(|current| display_value(&current));
                if shown.as_deref() != Some(value.as_str()) {
                    field.write(&mut tag, value);
                }
            }
            None => field.remove(&mut tag),
        }
    }
    Ok(Plan::from_diff(
        edits.iter().map(|(field, _)| field),
        &before,
        tag,
    ))
}

/// Reads and validates the edit source (`-` for stdin).
fn load(source: &Path) -> Result<Edits, Vec<String>> {
    let mut json = String::new();
    let read = if source == Path::new("-") {
        std::io::stdin().read_to_string(&mut json).map(|_| ())
    } else {
        std::fs::read_to_string(source).map(|text| json = text)
    };
    read.map_err(|err| vec![err.to_string()])?;
    let edits = parse(&json)?;
    reject_duplicate_paths(&edits)?;
    Ok(edits)
}

/// Runs `idk apply` and returns the process exit code.
pub async fn run(args: ApplyArgs) -> ExitCode {
    let source = args.source.clone();
    let loaded = tokio::task::spawn_blocking(move || load(&source))
        .await
        .expect("load task panicked");
    let edits = match loaded {
        Ok(edits) => edits,
        Err(errors) => {
            for error in errors {
                eprintln!("error: {}: {error}", args.source.display());
            }
            return ExitCode::from(INPUT_ERROR);
        }
    };
    for key in &edits.skipped_keys {
        eprintln!("warning: skipping `{key}`: not a field name");
    }

    let files: Vec<PathBuf> = edits.entries.iter().map(|(path, _)| path.clone()).collect();
    let by_path: HashMap<PathBuf, Vec<Edit>> = edits.entries.into_iter().collect();
    let inputs = Inputs { files, failures: 0 };
    run_writes(inputs, &args.run, &args.write, None, move |path| {
        let edits = by_path.get(path).map_or(&[][..], Vec::as_slice);
        plan_apply(path, edits).map_err(|err| err.to_string())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Outcome;
    use id3::TagLike;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn keys(edits: &Edits) -> HashSet<&str> {
        edits.skipped_keys.iter().map(String::as_str).collect()
    }

    const AUDIO: &[u8] = &[0xFF, 0xFB, 0x90, 0x64, 0x00];

    #[test]
    fn parses_edits_and_skips_non_field_keys() {
        let edits = parse(
            r#"[{"path": "a.mp3", "version": "2.4",
                 "tags": {"genre": "Rock", "comment": null, "TENC": "x", "date#2": "1999",
                          "txxx:Source": "CD"}},
                {"path": "b.mp3", "tags": {"TENC": "y"}}]"#,
        )
        .unwrap();

        assert_eq!(
            edits.entries,
            [
                (
                    PathBuf::from("a.mp3"),
                    vec![
                        (TagField::Genre, Some("Rock".into())),
                        (TagField::Comment, None),
                        (TagField::Txxx("Source".into()), Some("CD".into())),
                    ]
                ),
                (PathBuf::from("b.mp3"), vec![]),
            ]
        );
        assert_eq!(keys(&edits), HashSet::from(["TENC", "date#2"]));
    }

    #[test]
    fn reports_every_structural_problem() {
        let errors = parse(
            r#"[{"tags": {}}, {"path": 3, "tags": []}, "x",
                {"path": "a", "tags": {"genre": 1, "artist": "A", "TPE1": "B"}}]"#,
        )
        .unwrap_err();

        assert_eq!(
            errors,
            [
                "[0]: missing \"path\"",
                "[1].path: expected a non-empty string",
                "[1].tags: expected an object",
                "[2]: expected an object",
                "[3].tags.genre: expected a string or null",
                "[3].tags.TPE1: artist is given more than once",
            ]
        );
        assert!(parse("{}").unwrap_err()[0].starts_with("expected a JSON array"));
        assert!(parse("[").unwrap_err()[0].starts_with("invalid JSON"));
    }

    #[test]
    fn rejects_the_same_file_twice_under_any_spelling() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.mp3");
        std::fs::write(&path, AUDIO).unwrap();
        let alias = dir.path().join(".").join("a.mp3");
        let edits = Edits {
            entries: vec![(path, vec![]), (alias.clone(), vec![])],
            skipped_keys: BTreeSet::new(),
        };

        assert_eq!(
            reject_duplicate_paths(&edits).unwrap_err(),
            [format!(
                "[1].path: {} is already edited by [0]",
                alias.display()
            )]
        );
    }

    fn mp3(dir: &TempDir, build: impl FnOnce(&mut Tag)) -> PathBuf {
        let path = dir.path().join("a.mp3");
        std::fs::write(&path, AUDIO).unwrap();
        let mut tag = Tag::new();
        build(&mut tag);
        tag.write_to_path(&path, Version::Id3v24).unwrap();
        path
    }

    fn apply(path: &Path, edits: &[Edit]) -> Outcome {
        plan_apply(path, edits).unwrap().apply(path, false).unwrap()
    }

    #[test]
    fn writes_changed_values_and_clears_nulls() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| {
            tag.set_genre("Pop");
            tag.set_artist("Artist");
            tag.set_text("TENC", "Encoder");
        });

        let outcome = apply(
            &path,
            &[
                (TagField::Genre, Some("Rock".into())),
                (TagField::Artist, None),
                (TagField::Album, None),
            ],
        );

        let Outcome::Updated(changes) = outcome else {
            panic!("expected update");
        };
        assert_eq!(changes.len(), 2);
        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.genre(), Some("Rock"));
        assert_eq!(tag.artist(), None);
        assert!(tag.get("TENC").is_some());
    }

    #[test]
    fn shown_multi_values_are_not_rewritten() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_genre("Rock\0Pop"));
        let bytes = std::fs::read(&path).unwrap();

        let outcome = apply(&path, &[(TagField::Genre, Some("Rock; Pop".into()))]);

        assert_eq!(outcome, Outcome::Unchanged);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn untagged_files_get_a_tag_only_when_setting_values() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bare.mp3");
        std::fs::write(&path, AUDIO).unwrap();

        assert_eq!(apply(&path, &[(TagField::Genre, None)]), Outcome::Unchanged);
        assert_eq!(std::fs::read(&path).unwrap(), AUDIO);

        apply(&path, &[(TagField::Genre, Some("Rock".into()))]);
        assert_eq!(Tag::read_from_path(&path).unwrap().genre(), Some("Rock"));
    }
}
