//! `idk set tags`: write fixed values into fields.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use id3::{Tag, Version};

use crate::cli::{RunOptions, SetTagsArgs, WriteOptions};
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
    let assignments = Arc::new(args.fields);
    run_plans(args.files, &args.run, &args.write, move |path| {
        plan_set(path, &assignments)
    })
    .await
}

/// Plans and applies an edit on every file concurrently, then prints the summary.
async fn run_plans(
    files: Vec<std::path::PathBuf>,
    run: &RunOptions,
    write: &WriteOptions,
    plan: impl Fn(&Path) -> id3::Result<Plan> + Send + Sync + 'static,
) -> ExitCode {
    let dry_run = write.dry_run;
    let mut report = Report::new(dry_run);
    runner::process(
        files,
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
