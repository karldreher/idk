//! `idk copy tags`: copy one tag's value into another tag.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use futures_util::{StreamExt, stream};
use id3::Tag;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::cli::CopyTagsArgs;
use crate::tag_field::TagField;

/// What happened to a single file.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The destination tag was written.
    Updated,
    /// The destination already held the source value; the file was not written.
    Unchanged,
    /// The source tag is missing or empty; the file was not written.
    SourceEmpty,
}

/// Copies the value of `from` into `to` in the file at `path`.
///
/// The destination is overwritten. Every other frame, the tag version and the
/// audio data are preserved. The file is only written when its tag changes.
pub fn copy_tag(path: &Path, from: &TagField, to: &TagField) -> id3::Result<Outcome> {
    let Some(mut tag) = id3::no_tag_ok(Tag::read_from_path(path))? else {
        return Ok(Outcome::SourceEmpty);
    };
    let Some(value) = from.read(&tag).filter(|value| !value.is_empty()) else {
        return Ok(Outcome::SourceEmpty);
    };
    let previous = to.read(&tag);
    to.write(&mut tag, &value);
    if to.read(&tag) == previous {
        return Ok(Outcome::Unchanged);
    }
    tag.write_to_path(path, tag.version())?;
    Ok(Outcome::Updated)
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
    fn record(
        &mut self,
        path: &Path,
        result: id3::Result<Outcome>,
        args: &CopyTagsArgs,
        progress: &ProgressBar,
    ) {
        match result {
            Ok(Outcome::Updated) => self.updated += 1,
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
    let files = unique_files(args.files.clone()).await;
    let progress = progress_bar(files.len() as u64, args.run.quiet);
    let mut results = stream::iter(files)
        .map(|path| {
            let (from, to) = (args.from.clone(), args.to.clone());
            async move {
                let result = copy_file(path.clone(), from, to).await;
                (path, result)
            }
        })
        .buffer_unordered(args.run.jobs());

    let mut report = Report::default();
    while let Some((path, result)) = results.next().await {
        report.record(&path, result, &args, &progress);
        progress.inc(1);
    }
    progress.finish_and_clear();

    if args.run.quiet {
        return report.exit_code();
    }
    println!(
        "{} updated, {} unchanged, {} skipped (no {}), {} failed",
        report.updated, report.unchanged, report.skipped, args.from, report.failed
    );
    report.exit_code()
}

/// Builds the overall progress bar on stderr.
///
/// indicatif hides it automatically when stderr is not a terminal, so piped
/// and scripted runs stay clean.
fn progress_bar(len: u64, quiet: bool) -> ProgressBar {
    if quiet {
        return ProgressBar::hidden();
    }
    let style =
        ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} files ({per_sec}, eta {eta})")
            .expect("valid progress template")
            .progress_chars("##-");
    ProgressBar::with_draw_target(Some(len), ProgressDrawTarget::stderr()).with_style(style)
}

/// Drops inputs that resolve to the same file, keeping the first spelling.
///
/// Two concurrent writers on one file would race, so duplicates (for example
/// `a.mp3` and `./a.mp3`) must be collapsed before dispatch.
async fn unique_files(files: Vec<PathBuf>) -> Vec<PathBuf> {
    tokio::task::spawn_blocking(move || {
        let mut seen = HashSet::new();
        files
            .into_iter()
            .filter(|path| {
                seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            })
            .collect()
    })
    .await
    .expect("dedupe task panicked")
}

/// Runs [`copy_tag`] on tokio's blocking pool, since tag writes are synchronous file I/O.
async fn copy_file(path: PathBuf, from: TagField, to: TagField) -> id3::Result<Outcome> {
    tokio::task::spawn_blocking(move || copy_tag(&path, &from, &to))
        .await
        .expect("copy task panicked")
}

#[cfg(test)]
mod tests {
    use super::*;
    use id3::frame::{Comment, ExtendedText};
    use id3::{Frame, TagLike, Version};
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

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist).unwrap();

        let after = Tag::read_from_path(&path).unwrap();
        assert_eq!(outcome, Outcome::Updated);
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

        let outcome = copy_tag(&path, &TagField::AlbumArtist, &TagField::Artist).unwrap();

        let after = Tag::read_from_path(&path).unwrap();
        assert_eq!(outcome, Outcome::Updated);
        assert_eq!(after.artist(), Some("Old Album Artist"));
        assert_eq!(after.version(), Version::Id3v23);
    }

    #[test]
    fn creates_missing_destination() {
        let dir = TempDir::new().unwrap();
        let mut tag = Tag::new();
        tag.set_artist("Lead Artist");
        let path = write_mp3(&dir, Some(&tag), Version::Id3v24);

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist).unwrap();

        assert_eq!(outcome, Outcome::Updated);
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

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist).unwrap();

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

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist).unwrap();

        assert_eq!(outcome, Outcome::SourceEmpty);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn reports_untagged_file_as_empty_source() {
        let dir = TempDir::new().unwrap();
        let path = write_mp3(&dir, None, Version::Id3v24);

        let outcome = copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist).unwrap();

        assert_eq!(outcome, Outcome::SourceEmpty);
        assert_eq!(std::fs::read(&path).unwrap(), AUDIO);
    }

    #[tokio::test]
    async fn collapses_duplicate_inputs() {
        let dir = TempDir::new().unwrap();
        let path = write_mp3(&dir, None, Version::Id3v24);
        let alias = dir.path().join(".").join("song.mp3");

        let files = unique_files(vec![path.clone(), alias, path.clone()]).await;

        assert_eq!(files, vec![path]);
    }

    #[test]
    fn errors_on_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.mp3");

        assert!(copy_tag(&path, &TagField::Artist, &TagField::AlbumArtist).is_err());
    }
}
