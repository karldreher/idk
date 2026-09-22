//! `idk set tags` and `idk clear tags`: write fixed values into fields, or remove them.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use id3::{Tag, Version};

use clap::error::ErrorKind;

use crate::cli::{ClearTagsArgs, InputArgs, RunOptions, SetTagsArgs, WriteOptions, usage_error};
use crate::input;
use crate::outcome::{Change, Plan, Report};
use crate::runner::{self, Order};
use crate::tag_field::TagField;

/// Reads the file at `path` and works out the effect of setting every field in `assignments`.
///
/// Nothing is written; see [`Plan::apply`]. A file without a tag gets a new ID3v2.4 tag.
pub fn plan_set(path: &Path, assignments: &[(TagField, String)]) -> id3::Result<Plan> {
    let mut tag = id3::no_tag_ok(Tag::read_from_path(path))?
        .unwrap_or_else(|| Tag::with_version(Version::Id3v24));
    let before = tag.clone();
    for (field, value) in assignments {
        field.write(&mut tag, value);
    }
    let fields = assignments.iter().map(|(field, _)| field);
    Ok(plan_changes(fields, before, tag))
}

/// Reads the file at `path` and works out the effect of removing every field in `fields`.
///
/// Nothing is written; see [`Plan::apply`]. A file without a tag is unchanged.
pub fn plan_clear(path: &Path, fields: &[TagField]) -> id3::Result<Plan> {
    let Some(mut tag) = id3::no_tag_ok(Tag::read_from_path(path))? else {
        return Ok(Plan::Unchanged);
    };
    let before = tag.clone();
    for field in fields {
        field.remove(&mut tag);
    }
    Ok(plan_changes(fields.iter(), before, tag))
}

/// A plan writing `after` for every field whose value differs from `before`.
fn plan_changes<'a>(fields: impl Iterator<Item = &'a TagField>, before: Tag, after: Tag) -> Plan {
    let changes: Vec<Change> = fields
        .filter_map(|field| Change::between(field, &before, &after))
        .collect();
    if changes.is_empty() {
        Plan::Unchanged
    } else {
        Plan::Write {
            tag: after,
            changes,
        }
    }
}

/// Runs `idk set tags` over every input file and returns the process exit code.
pub async fn run_set(args: SetTagsArgs) -> ExitCode {
    reject_duplicates(args.fields.iter().map(|(field, _)| field));
    let assignments = Arc::new(args.fields);
    run_plans(args.input, &args.run, &args.write, move |path| {
        plan_set(path, &assignments)
    })
    .await
}

/// Runs `idk clear tags` over every input file and returns the process exit code.
pub async fn run_clear(args: ClearTagsArgs) -> ExitCode {
    reject_duplicates(args.fields.iter());
    let fields = Arc::new(args.fields);
    run_plans(args.input, &args.run, &args.write, move |path| {
        plan_clear(path, &fields)
    })
    .await
}

/// Exits with a usage error when any `--field` is named twice (aliases included).
fn reject_duplicates<'a>(fields: impl Iterator<Item = &'a TagField>) {
    let mut seen = Vec::new();
    for field in fields {
        if seen.contains(&field) {
            usage_error(
                ErrorKind::ArgumentConflict,
                format!("--field {field} given more than once"),
            );
        }
        seen.push(field);
    }
}

/// Plans and applies an edit on every file concurrently, then prints the summary.
async fn run_plans(
    input: InputArgs,
    run: &RunOptions,
    write: &WriteOptions,
    plan: impl Fn(&Path) -> id3::Result<Plan> + Send + Sync + 'static,
) -> ExitCode {
    let dry_run = write.dry_run;
    let mut report = Report::new(dry_run);
    let inputs = input::resolve(input).await;
    report.add_failures(inputs.failures);
    runner::process(
        inputs.files,
        run.jobs(),
        Order::Completion,
        !run.quiet,
        move |path| {
            plan(path)
                .and_then(|plan| plan.apply(path, dry_run))
                .map_err(|err| err.to_string())
        },
        |progress, path, result| report.record(&path, result, progress),
    )
    .await;

    if !run.quiet {
        println!("{}", report.summary(None));
    }
    report.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Outcome;
    use id3::TagLike;
    use std::path::PathBuf;
    use tempfile::TempDir;

    const AUDIO: &[u8] = &[0xFF, 0xFB, 0x90, 0x64, 0x00, 0x11];

    fn mp3(dir: &TempDir, build: impl FnOnce(&mut Tag), version: Version) -> PathBuf {
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, AUDIO).unwrap();
        let mut tag = Tag::new();
        build(&mut tag);
        tag.write_to_path(&path, version).unwrap();
        path
    }

    fn set(path: &Path, assignments: &[(TagField, &str)]) -> Outcome {
        let assignments: Vec<_> = assignments
            .iter()
            .map(|(field, value)| (field.clone(), value.to_string()))
            .collect();
        plan_set(path, &assignments)
            .unwrap()
            .apply(path, false)
            .unwrap()
    }

    #[test]
    fn sets_several_fields_and_reports_each_change() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_genre("Pop"), Version::Id3v23);

        let outcome = set(
            &path,
            &[
                (TagField::Genre, "Rock"),
                (TagField::AlbumArtist, "Various Artists"),
                (TagField::Txxx("Source".into()), "CD"),
            ],
        );

        let Outcome::Updated(changes) = outcome else {
            panic!("expected update, got {outcome:?}");
        };
        assert_eq!(
            changes,
            [
                Change {
                    field: TagField::Genre,
                    old: Some("Pop".into()),
                    new: Some("Rock".into())
                },
                Change {
                    field: TagField::AlbumArtist,
                    old: None,
                    new: Some("Various Artists".into())
                },
                Change {
                    field: TagField::Txxx("Source".into()),
                    old: None,
                    new: Some("CD".into())
                },
            ]
        );
        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.genre(), Some("Rock"));
        assert_eq!(tag.album_artist(), Some("Various Artists"));
        assert_eq!(tag.version(), Version::Id3v23);
        assert!(std::fs::read(&path).unwrap().ends_with(AUDIO));
    }

    #[test]
    fn matching_values_leave_the_file_untouched() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_genre("Rock"), Version::Id3v24);
        let bytes = std::fs::read(&path).unwrap();

        assert_eq!(set(&path, &[(TagField::Genre, "Rock")]), Outcome::Unchanged);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    fn clear(path: &Path, fields: &[TagField]) -> Outcome {
        plan_clear(path, fields)
            .unwrap()
            .apply(path, false)
            .unwrap()
    }

    #[test]
    fn clears_fields_and_reports_each_removal() {
        let dir = TempDir::new().unwrap();
        let path = mp3(
            &dir,
            |tag| {
                tag.set_genre("Rock");
                tag.set_artist("Artist");
                tag.set_text("TENC", "Encoder");
            },
            Version::Id3v24,
        );

        let outcome = clear(&path, &[TagField::Genre, TagField::Comment]);

        assert_eq!(
            outcome,
            Outcome::Updated(vec![Change {
                field: TagField::Genre,
                old: Some("Rock".into()),
                new: None
            }])
        );
        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.genre(), None);
        assert_eq!(tag.artist(), Some("Artist"));
        assert!(tag.get("TENC").is_some());
        assert!(std::fs::read(&path).unwrap().ends_with(AUDIO));
    }

    #[test]
    fn clearing_absent_fields_or_untagged_files_is_unchanged() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_artist("Artist"), Version::Id3v24);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(clear(&path, &[TagField::Genre]), Outcome::Unchanged);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        let bare = dir.path().join("bare.mp3");
        std::fs::write(&bare, AUDIO).unwrap();
        assert_eq!(clear(&bare, &[TagField::Genre]), Outcome::Unchanged);
        assert_eq!(std::fs::read(&bare).unwrap(), AUDIO);
    }

    #[test]
    fn creates_a_v24_tag_on_untagged_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bare.mp3");
        std::fs::write(&path, AUDIO).unwrap();

        set(&path, &[(TagField::Title, "Title")]);

        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title(), Some("Title"));
        assert_eq!(tag.version(), Version::Id3v24);
        assert!(std::fs::read(&path).unwrap().ends_with(AUDIO));
    }
}
