//! `idk show`: print the tags of each input file.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use id3::{Content, Frame, Tag, Version};
use serde_json::{Map, Value, json};

use crate::cli::ShowArgs;
use crate::runner::{self, Order};
use crate::tag_field::TagField;

/// One file's tags, in frame order.
#[derive(Debug, PartialEq, Eq)]
pub struct FileTags {
    /// The ID3v2 version, or `None` when the file has no ID3v2 tag.
    pub version: Option<Version>,
    /// `(key, value)` pairs; keys are field names where known, otherwise frame IDs.
    pub entries: Vec<(String, String)>,
}

/// Frames omitted unless `--verbose`: vendor data (e.g. Amazon's PRIV), copyright
/// notices and encoder settings, which are rarely useful and often very large.
const VERBOSE_ONLY: &[&str] = &["PRIV", "TCOP", "TSSE"];

/// Reads the tags of the file at `path`, keeping only `fields` when it is non-empty.
///
/// Frames in [`VERBOSE_ONLY`] are skipped unless `verbose` is set.
pub fn read_tags(path: &Path, fields: &[TagField], verbose: bool) -> id3::Result<FileTags> {
    let Some(tag) = id3::no_tag_ok(Tag::read_from_path(path))? else {
        return Ok(FileTags {
            version: None,
            entries: Vec::new(),
        });
    };
    let mut entries: Vec<(String, String)> = Vec::new();
    for frame in tag.frames() {
        if !verbose && VERBOSE_ONLY.contains(&frame.id()) {
            continue;
        }
        let field = TagField::of_frame(frame);
        if !fields.is_empty() && !field.as_ref().is_some_and(|field| fields.contains(field)) {
            continue;
        }
        let key = field.map_or_else(|| frame_key(frame), |field| field.to_string());
        let key = unique_key(&entries, key);
        entries.push((key, frame_value(frame)));
    }
    Ok(FileTags {
        version: Some(tag.version()),
        entries,
    })
}

/// Key for a frame without a field name: its ID, plus the description for comments.
fn frame_key(frame: &Frame) -> String {
    match frame.content().comment() {
        Some(comment) => format!("{}:{}", frame.id(), comment.description),
        None => frame.id().to_owned(),
    }
}

/// Appends `#2`, `#3`, ... when `key` is already taken, so JSON keys stay unique.
fn unique_key(entries: &[(String, String)], key: String) -> String {
    let taken = |candidate: &str| entries.iter().any(|(existing, _)| existing == candidate);
    if !taken(&key) {
        return key;
    }
    (2..)
        .map(|n| format!("{key}#{n}"))
        .find(|candidate| !taken(candidate))
        .expect("unbounded range")
}

/// Human-readable value of a frame. Pictures and private data are summarized, never dumped.
fn frame_value(frame: &Frame) -> String {
    match frame.content() {
        // ID3v2.4 separates multiple values with NUL.
        Content::Text(text) => text.replace('\0', "; "),
        Content::ExtendedText(extended) => extended.value.clone(),
        Content::Comment(comment) => comment.text.clone(),
        Content::Unknown(unknown) if matches!(frame.id(), "APIC" | "PIC") => {
            picture_summary(frame.id(), &unknown.data)
        }
        Content::Private(private) => format!(
            "{}, {} bytes",
            private.owner_identifier,
            private.private_data.len()
        ),
        content => content.to_string(),
    }
}

/// `image/jpeg, 12345 bytes` for a raw (undecoded) picture frame.
///
/// Pictures are kept undecoded for speed, so the image format is read straight
/// from the frame header: a NUL-terminated MIME type (APIC) or a three-letter
/// format (ID3v2.2 PIC), after the one-byte text encoding.
fn picture_summary(id: &str, data: &[u8]) -> String {
    let header = data.get(1..).unwrap_or_default();
    let format = if id == "PIC" {
        header.get(..3).unwrap_or(header)
    } else {
        header.split(|&b| b == 0).next().unwrap_or_default()
    };
    let format = String::from_utf8_lossy(format);
    let format = if format.is_empty() {
        "picture".into()
    } else {
        format
    };
    format!("{format}, {} bytes", data.len())
}

/// `"2.4"` style version string for JSON output.
fn version_number(version: Version) -> &'static str {
    match version {
        Version::Id3v22 => "2.2",
        Version::Id3v23 => "2.3",
        Version::Id3v24 => "2.4",
    }
}

/// Renders one file for readable output.
fn render_text(path: &Path, tags: &FileTags) -> String {
    let version = tags
        .version
        .map_or("no ID3v2 tag".to_owned(), |version| version.to_string());
    let mut out = format!("{} ({version})\n", path.display());
    for (key, value) in &tags.entries {
        out.push_str(&format!("  {key}: {value}\n"));
    }
    out
}

/// Renders one file as a JSON object.
fn render_json(path: &Path, tags: FileTags) -> Value {
    let entries: Map<String, Value> = tags
        .entries
        .into_iter()
        .map(|(key, value)| (key, Value::String(value)))
        .collect();
    json!({
        "path": path.display().to_string(),
        "version": tags.version.map(version_number),
        "tags": entries,
    })
}

/// Runs `idk show` over every input file and returns the process exit code.
///
/// Files are read concurrently, but output follows input order.
pub async fn run(args: ShowArgs) -> ExitCode {
    let show_progress = !args.json && !args.run.quiet && std::io::stdout().is_terminal();
    let fields = args.fields.clone();
    let verbose = args.verbose;
    let mut json_files = Vec::new();
    let mut failed = false;
    let mut first = true;

    runner::process(
        args.files.clone(),
        args.run.jobs(),
        Order::Input,
        show_progress,
        move |path| read_tags(path, &fields, verbose),
        |progress, path: PathBuf, result| match result {
            Ok(tags) if args.json => json_files.push(render_json(&path, tags)),
            Ok(tags) => {
                let separator = if std::mem::take(&mut first) { "" } else { "\n" };
                progress.suspend(|| print!("{separator}{}", render_text(&path, &tags)));
            }
            Err(err) => {
                failed = true;
                progress.suspend(|| eprintln!("error: {}: {err}", path.display()));
            }
        },
    )
    .await;

    if args.json {
        let out = serde_json::to_string_pretty(&json_files).expect("JSON values serialize");
        println!("{out}");
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use id3::TagLike;
    use id3::frame::Comment;
    use tempfile::TempDir;

    fn tagged(dir: &TempDir, build: impl FnOnce(&mut Tag)) -> PathBuf {
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, [0xFF, 0xFB, 0x90, 0x64]).unwrap();
        let mut tag = Tag::new();
        build(&mut tag);
        tag.write_to_path(&path, Version::Id3v24).unwrap();
        path
    }

    fn entry(key: &str, value: &str) -> (String, String) {
        (key.to_owned(), value.to_owned())
    }

    #[test]
    fn names_known_fields_and_keys_the_rest_by_frame_id() {
        let dir = TempDir::new().unwrap();
        let path = tagged(&dir, |tag| {
            tag.set_artist("Artist");
            tag.set_text("TENC", "Encoder");
            TagField::Txxx("custom".into()).write(tag, "x");
            tag.add_frame(Comment {
                lang: "eng".into(),
                description: "note".into(),
                text: "n".into(),
            });
        });

        let tags = read_tags(&path, &[], false).unwrap();

        assert_eq!(tags.version, Some(Version::Id3v24));
        assert_eq!(
            tags.entries,
            [
                entry("artist", "Artist"),
                entry("TENC", "Encoder"),
                entry("txxx:custom", "x"),
                entry("COMM:note", "n"),
            ]
        );
    }

    #[test]
    fn filters_to_requested_fields() {
        let dir = TempDir::new().unwrap();
        let path = tagged(&dir, |tag| {
            tag.set_artist("Artist");
            tag.set_title("Title");
            tag.set_album("Album");
        });

        let tags = read_tags(&path, &[TagField::Album, TagField::Artist], false).unwrap();

        assert_eq!(
            tags.entries,
            [entry("artist", "Artist"), entry("album", "Album")]
        );
    }

    #[test]
    fn joins_multiple_values_and_disambiguates_duplicate_keys() {
        let dir = TempDir::new().unwrap();
        let path = tagged(&dir, |tag| {
            tag.set_text("TCON", "Rock\0Pop");
            tag.set_text("TDRC", "2021");
            tag.set_text("TYER", "2021");
        });

        let tags = read_tags(&path, &[], false).unwrap();

        assert_eq!(
            tags.entries,
            [
                entry("genre", "Rock; Pop"),
                entry("date", "2021"),
                entry("date#2", "2021")
            ]
        );
    }

    #[test]
    fn hides_private_copyright_and_encoder_frames_unless_verbose() {
        let dir = TempDir::new().unwrap();
        let path = tagged(&dir, |tag| {
            tag.set_artist("Artist");
            tag.add_frame(id3::frame::Private {
                owner_identifier: "www.amazon.com".into(),
                private_data: vec![0; 512],
            });
            tag.set_text("TCOP", "");
            tag.set_text("TSSE", "LAME 3.100");
        });

        let quiet = read_tags(&path, &[], false).unwrap();
        assert_eq!(quiet.entries, [entry("artist", "Artist")]);

        let verbose = read_tags(&path, &[], true).unwrap();
        assert_eq!(
            verbose.entries,
            [
                entry("artist", "Artist"),
                entry("PRIV", "www.amazon.com, 512 bytes"),
                entry("TCOP", ""),
                entry("TSSE", "LAME 3.100"),
            ]
        );
    }

    #[test]
    fn untagged_file_has_no_version_or_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, [0xFF, 0xFB]).unwrap();

        let tags = read_tags(&path, &[], false).unwrap();

        assert_eq!(
            tags,
            FileTags {
                version: None,
                entries: vec![]
            }
        );
    }

    #[test]
    fn summarizes_pictures() {
        let mut apic = vec![0u8];
        apic.extend_from_slice(b"image/png\0\x03\0");
        apic.extend_from_slice(&[0x89; 10]);
        assert_eq!(
            picture_summary("APIC", &apic),
            format!("image/png, {} bytes", apic.len())
        );

        let pic = [0u8, b'J', b'P', b'G', 3, 0, 0xFF];
        assert_eq!(picture_summary("PIC", &pic), "JPG, 7 bytes");
        assert_eq!(picture_summary("APIC", &[]), "picture, 0 bytes");
    }

    #[test]
    fn renders_json_with_version_and_ordered_tags() {
        let tags = FileTags {
            version: Some(Version::Id3v23),
            entries: vec![entry("title", "T"), entry("artist", "A")],
        };

        let value = render_json(Path::new("a.mp3"), tags);

        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"path":"a.mp3","version":"2.3","tags":{"title":"T","artist":"A"}}"#
        );
    }
}
