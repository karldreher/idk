//! `idk copy tags`: copy one tag's value into another tag.

use std::path::Path;
use std::process::ExitCode;

use id3::Tag;

use crate::cli::CopyTagsArgs;
use crate::outcome::{Change, Outcome, Plan, Report};
use crate::runner::{self, Order};
use crate::tag_field::TagField;

/// Reads the file at `path` and works out the effect of copying `from` into `to`.
///
/// Nothing is written; see [`Plan::apply`].
pub fn plan_copy(path: &Path, from: &TagField, to: &TagField) -> id3::Result<Plan> {
    let Some(mut tag) = id3::no_tag_ok(Tag::read_from_path(path))? else {
        return Ok(Plan::Skipped);
    };
    let Some(value) = from.read(&tag).filter(|value| !value.is_empty()) else {
        return Ok(Plan::Skipped);
    };
    let before = tag.clone();
    to.write(&mut tag, &value);
    match Change::between(to, &before, &tag) {
        Some(change) => Ok(Plan::Write {
            tag,
            changes: vec![change],
        }),
        None => Ok(Plan::Unchanged),
    }
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

/// Runs `idk copy tags` over every input file and returns the process exit code.
///
/// Files are processed concurrently, up to `--jobs` at a time. A file with an
/// empty source is skipped, or fails with `--fail-on-empty`.
pub async fn run(args: CopyTagsArgs, from: TagField, to: TagField) -> ExitCode {
    let summary_label = format!("no {from}");
    let dry_run = args.write.dry_run;
    let fail_on_empty = args.fail_on_empty;
    let mut report = Report::new(dry_run);
    runner::process(
        args.files.clone(),
        args.run.jobs(),
        Order::Completion,
        !args.run.quiet,
        move |path| match copy_tag(path, &from, &to, dry_run) {
            Ok(Outcome::Skipped) if fail_on_empty => Err(format!("no {from} value")),
            result => result.map_err(|err| err.to_string()),
        },
        |progress, path, result| report.record(&path, result, progress),
    )
    .await;

    if !args.run.quiet {
        println!("{}", report.summary(Some(&summary_label)));
    }
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
            Outcome::Updated(vec![Change {
                field: TagField::AlbumArtist,
                old: Some("Old Album Artist".into()),
                new: Some("Lead Artist".into()),
            }])
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

        assert_eq!(outcome, Outcome::Skipped);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn reports_untagged_file_as_empty_source() {
        let dir = TempDir::new().unwrap();
        let path = write_mp3(&dir, None, Version::Id3v24);

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist, false).unwrap();

        assert_eq!(outcome, Outcome::Skipped);
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
            Outcome::Updated(vec![Change {
                field: TagField::AlbumArtist,
                old: Some("Old Album Artist".into()),
                new: Some("Lead Artist".into()),
            }])
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
