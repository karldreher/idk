//! `idk copy tags`: copy one tag's value into another tag.

use std::path::Path;
use std::process::ExitCode;

use id3::Tag;
use indicatif::ProgressBar;

use crate::cli::CopyTagsArgs;
use crate::runner;
use crate::tag_field::TagField;

/// A field's value before and after a copy.
#[derive(Debug, PartialEq, Eq)]
pub struct Change {
    /// The destination's previous value, if it had one.
    pub old: Option<String>,
    /// The value written to the destination.
    pub new: String,
}

/// What happened to a single file.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The destination tag was written.
    Updated(Change),
    /// The destination already held the source value; the file was not written.
    Unchanged,
    /// The source tag is missing or empty; the file was not written.
    SourceEmpty,
}

/// What copying would do to a file, decided before anything is written.
pub enum Plan {
    /// The tag changes; `tag` holds the new state to write.
    Write {
        /// The updated tag.
        tag: Tag,
        /// The destination field's change.
        change: Change,
    },
    /// The destination already holds the source value.
    Unchanged,
    /// The source tag is missing or empty.
    SourceEmpty,
}

impl Plan {
    /// Writes the planned tag to `path` if it changed, and reports the outcome.
    ///
    /// With `dry_run`, nothing is written but the outcome is the same.
    pub fn apply(self, path: &Path, dry_run: bool) -> id3::Result<Outcome> {
        match self {
            Plan::Write { tag, change } => {
                if !dry_run {
                    tag.write_to_path(path, tag.version())?;
                }
                Ok(Outcome::Updated(change))
            }
            Plan::Unchanged => Ok(Outcome::Unchanged),
            Plan::SourceEmpty => Ok(Outcome::SourceEmpty),
        }
    }
}

/// Reads the file at `path` and works out the effect of copying `from` into `to`.
///
/// Nothing is written; see [`Plan::apply`].
pub fn plan_copy(path: &Path, from: &TagField, to: &TagField) -> id3::Result<Plan> {
    let Some(mut tag) = id3::no_tag_ok(Tag::read_from_path(path))? else {
        return Ok(Plan::SourceEmpty);
    };
    let Some(value) = from.read(&tag).filter(|value| !value.is_empty()) else {
        return Ok(Plan::SourceEmpty);
    };
    let old = to.read(&tag);
    to.write(&mut tag, &value);
    let new = to.read(&tag);
    if new == old {
        return Ok(Plan::Unchanged);
    }
    let change = Change {
        old,
        new: new.unwrap_or_default(),
    };
    Ok(Plan::Write { tag, change })
}

/// Copies the value of `from` into `to` in the file at `path`.
///
/// The destination is overwritten. Every other frame, the tag version and the
/// audio data are preserved. The file is only written when its tag changes.
pub fn copy_tag(
    path: &Path,
    from: &TagField,
    to: &TagField,
    dry_run: bool,
) -> id3::Result<Outcome> {
    plan_copy(path, from, to)?.apply(path, dry_run)
}

/// Per-run tallies used for the final summary and exit code.
#[derive(Default)]
struct Report {
    updated: usize,
    unchanged: usize,
    skipped: usize,
    failed: usize,
}

impl Report {
    /// Non-zero when any file failed.
    fn exit_code(&self) -> ExitCode {
        if self.failed > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }

    /// Records one file's result, printing failures to stderr above the progress bar.
    ///
    /// In a dry run, each pending change is printed to stdout.
    fn record(
        &mut self,
        path: &Path,
        result: id3::Result<Outcome>,
        args: &CopyTagsArgs,
        progress: &ProgressBar,
    ) {
        match result {
            Ok(Outcome::Updated(change)) => {
                self.updated += 1;
                if args.write.dry_run {
                    let old = change
                        .old
                        .map_or("(none)".to_owned(), |old| format!("{old:?}"));
                    progress.suspend(|| {
                        println!("{}: {} {old} -> {:?}", path.display(), args.to, change.new)
                    });
                }
            }
            Ok(Outcome::Unchanged) => self.unchanged += 1,
            Ok(Outcome::SourceEmpty) if args.fail_on_empty => {
                self.failed += 1;
                progress.suspend(|| eprintln!("error: {}: no {} value", path.display(), args.from));
            }
            Ok(Outcome::SourceEmpty) => self.skipped += 1,
            Err(err) => {
                self.failed += 1;
                progress.suspend(|| eprintln!("error: {}: {err}", path.display()));
            }
        }
    }
}

/// Runs `idk copy tags` over every input file and returns the process exit code.
///
/// Files are processed concurrently, up to `--jobs` at a time.
pub async fn run(args: CopyTagsArgs) -> ExitCode {
    let (from, to) = (args.from.clone(), args.to.clone());
    let dry_run = args.write.dry_run;
    let mut report = Report::default();
    runner::process(
        args.files.clone(),
        args.run.jobs(),
        !args.run.quiet,
        move |path| copy_tag(path, &from, &to, dry_run),
        |progress, path, result| report.record(&path, result, &args, progress),
    )
    .await;

    if args.run.quiet {
        return report.exit_code();
    }
    let updated = if args.write.dry_run {
        "would be updated"
    } else {
        "updated"
    };
    println!(
        "{} {updated}, {} unchanged, {} skipped (no {}), {} failed",
        report.updated, report.unchanged, report.skipped, args.from, report.failed
    );
    report.exit_code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use id3::frame::{Comment, ExtendedText};
    use id3::{Frame, TagLike, Version};
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Bytes standing in for MPEG audio; only their preservation matters.
    const AUDIO: &[u8] = &[0xFF, 0xFB, 0x90, 0x64, 0x00, 0x11, 0x22, 0x33];

    fn write_mp3(dir: &TempDir, tag: Option<&Tag>, version: Version) -> PathBuf {
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, AUDIO).unwrap();
        if let Some(tag) = tag {
            tag.write_to_path(&path, version).unwrap();
        }
        path
    }

    fn rich_tag() -> Tag {
        let mut tag = Tag::new();
        tag.set_artist("Lead Artist");
        tag.set_album_artist("Old Album Artist");
        tag.set_title("Title");
        tag.set_album("Album");
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: "".into(),
            text: "a comment".into(),
        });
        tag.add_frame(ExtendedText {
            description: "custom".into(),
            value: "value".into(),
        });
        tag
    }

    fn frames_except(tag: &Tag, id: &str) -> Vec<Frame> {
        tag.frames().filter(|f| f.id() != id).cloned().collect()
    }

    #[test]
    fn copies_artist_to_album_artist_and_preserves_everything_else() {
        let dir = TempDir::new().unwrap();
        let path = write_mp3(&dir, Some(&rich_tag()), Version::Id3v24);
        let before = Tag::read_from_path(&path).unwrap();

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, false).unwrap();

        let after = Tag::read_from_path(&path).unwrap();
        assert_eq!(
            outcome,
            Outcome::Updated(Change {
                old: Some("Old Album Artist".into()),
                new: "Lead Artist".into(),
            })
        );
        assert_eq!(after.album_artist(), Some("Lead Artist"));
        assert_eq!(
            frames_except(&after, "TPE2"),
            frames_except(&before, "TPE2")
        );
        assert_eq!(after.version(), Version::Id3v24);
        assert!(std::fs::read(&path).unwrap().ends_with(AUDIO));
    }

    #[test]
    fn copies_album_artist_to_artist() {
        let dir = TempDir::new().unwrap();
        let path = write_mp3(&dir, Some(&rich_tag()), Version::Id3v23);

        let outcome = copy_tag(&path, &TagField::AlbumArtist, &TagField::Artist, false).unwrap();

        let after = Tag::read_from_path(&path).unwrap();
        assert!(matches!(outcome, Outcome::Updated(_)));
        assert_eq!(after.artist(), Some("Old Album Artist"));
        assert_eq!(after.version(), Version::Id3v23);
    }

    #[test]
    fn creates_missing_destination() {
        let dir = TempDir::new().unwrap();
        let mut tag = Tag::new();
        tag.set_artist("Lead Artist");
        let path = write_mp3(&dir, Some(&tag), Version::Id3v24);

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, false).unwrap();

        assert!(matches!(outcome, Outcome::Updated(_)));
        let after = Tag::read_from_path(&path).unwrap();
        assert_eq!(after.album_artist(), Some("Lead Artist"));
    }

    #[test]
    fn skips_without_writing_when_values_match() {
        let dir = TempDir::new().unwrap();
        let mut tag = Tag::new();
        tag.set_artist("Same");
        tag.set_album_artist("Same");
        let path = write_mp3(&dir, Some(&tag), Version::Id3v24);
        let bytes = std::fs::read(&path).unwrap();

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, false).unwrap();

        assert_eq!(outcome, Outcome::Unchanged);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn reports_empty_source_without_writing() {
        let dir = TempDir::new().unwrap();
        let mut tag = Tag::new();
        tag.set_album_artist("Album Artist");
        let path = write_mp3(&dir, Some(&tag), Version::Id3v24);
        let bytes = std::fs::read(&path).unwrap();

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, false).unwrap();

        assert_eq!(outcome, Outcome::SourceEmpty);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn reports_untagged_file_as_empty_source() {
        let dir = TempDir::new().unwrap();
        let path = write_mp3(&dir, None, Version::Id3v24);

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, false).unwrap();

        assert_eq!(outcome, Outcome::SourceEmpty);
        assert_eq!(std::fs::read(&path).unwrap(), AUDIO);
    }

    #[test]
    fn dry_run_reports_change_without_writing() {
        let dir = TempDir::new().unwrap();
        let path = write_mp3(&dir, Some(&rich_tag()), Version::Id3v24);
        let bytes = std::fs::read(&path).unwrap();

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, true).unwrap();

        assert_eq!(
            outcome,
            Outcome::Updated(Change {
                old: Some("Old Album Artist".into()),
                new: "Lead Artist".into(),
            })
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn errors_on_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.mp3");

        assert!(copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, false).is_err());
    }
}
