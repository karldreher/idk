//! `idk show`: print the tags of each input file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use id3::{Content, Frame, Tag, Version};
use serde_json::{Map, Value, json};

use crate::cli::ShowArgs;
use crate::input;
use crate::runner::{self, Order};
use crate::tag_field::TagField;

/// One file's tags, sorted by key.
#[derive(Debug, PartialEq, Eq)]
pub struct FileTags {
    /// The ID3v2 version, or `None` when the file has no ID3v2 tag.
    pub version: Option<Version>,
    /// `(key, value)` pairs; keys are field names where known, otherwise frame IDs.
    /// The value is `None` for a wanted field the tag does not have.
    pub entries: Vec<(String, Option<String>)>,
}

/// Fields listed even when absent, unless `--field` picks others.
const CORE_FIELDS: [TagField; 8] = [
    TagField::Title,
    TagField::Artist,
    TagField::AlbumArtist,
    TagField::Album,
    TagField::TrackNumber,
    TagField::DiscNumber,
    TagField::Date,
    TagField::Genre,
];

/// Frames omitted unless `--verbose`: vendor data (e.g. Amazon's PRIV), copyright
/// notices and encoder settings, which are rarely useful and often very large.
const VERBOSE_ONLY: &[&str] = &["PRIV", "TCOP", "TSSE"];

/// Reads the tags of the file at `path`, keeping only `fields` when it is non-empty.
///
/// Frames in [`VERBOSE_ONLY`] are skipped unless `verbose` is set. Wanted fields
/// (`fields`, or [`CORE_FIELDS`] when empty) that the tag lacks are listed with no value.
/// Entries are sorted by key: field names first, then frame IDs (upper case).
pub fn read_tags(path: &Path, fields: &[TagField], verbose: bool) -> id3::Result<FileTags> {
    let Some(tag) = id3::no_tag_ok(Tag::read_from_path(path))? else {
        return Ok(FileTags {
            version: None,
            entries: Vec::new(),
        });
    };
    let mut entries: Vec<(String, Option<String>)> = Vec::new();
    let mut keys = UniqueKeys::default();
    let mut seen: HashSet<TagField> = HashSet::new();
    for frame in tag.frames() {
        if !verbose && VERBOSE_ONLY.contains(&frame.id()) {
            continue;
        }
        let field = TagField::of_frame(frame);
        if !fields.is_empty() && !field.as_ref().is_some_and(|field| fields.contains(field)) {
            continue;
        }
        if let Some(field) = &field {
            seen.insert(field.clone());
        }
        let key = field.map_or_else(|| frame_key(frame), |field| field.to_string());
        let key = keys.claim(key);
        entries.push((key, Some(frame_value(frame))));
    }
    let wanted = if fields.is_empty() {
        &CORE_FIELDS
    } else {
        fields
    };
    for field in wanted {
        if seen.insert(field.clone()) {
            entries.push((keys.claim(field.to_string()), None));
        }
    }
    entries.sort_by(|(a, _), (b, _)| frame_ids_last(a).cmp(&frame_ids_last(b)));
    Ok(FileTags {
        version: Some(tag.version()),
        entries,
    })
}

/// Sort key placing field names (lower case) before frame IDs (upper case).
fn frame_ids_last(key: &str) -> (bool, &str) {
    (key.starts_with(|c: char| c.is_ascii_uppercase()), key)
}

/// Key for a frame without a field name: its ID, plus the description for comments.
fn frame_key(frame: &Frame) -> String {
    match frame.content().comment() {
        Some(comment) => format!("{}:{}", frame.id(), comment.description),
        None => frame.id().to_owned(),
    }
}

/// Hands out unique JSON keys, remembering which are taken and where each suffix left off.
#[derive(Default)]
struct UniqueKeys {
    taken: HashSet<String>,
    next_suffix: HashMap<String, usize>,
}

impl UniqueKeys {
    /// Appends `#2`, `#3`, ... when `key` is already taken, so JSON keys stay unique.
    fn claim(&mut self, key: String) -> String {
        if self.taken.insert(key.clone()) {
            return key;
        }
        let suffix = self.next_suffix.entry(key.clone()).or_insert(2);
        loop {
            let candidate = format!("{key}#{suffix}");
            *suffix += 1;
            if self.taken.insert(candidate.clone()) {
                return candidate;
            }
        }
    }
}

/// Human-readable value of a frame. Pictures and private data are summarized, never dumped.
fn frame_value(frame: &Frame) -> String {
    match frame.content() {
        Content::Text(text) => display_value(text),
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

/// How a text value is shown: ID3v2.4 multiple values (NUL-separated) are joined with `; `.
///
/// `idk apply` compares against this rendering, so unedited `show --json` output is a no-op.
pub fn display_value(raw: &str) -> String {
    raw.replace('\0', "; ")
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
        out.push_str(&format!(
            "  {key}: {}\n",
            value.as_deref().unwrap_or("(no value)")
        ));
    }
    out
}

/// Renders one file as a JSON object.
fn render_json(path: &Path, tags: FileTags) -> Value {
    let entries: Map<String, Value> = tags
        .entries
        .into_iter()
        .map(|(key, value)| (key, value.map_or(Value::Null, Value::String)))
        .collect();
    json!({
        "path": path.display().to_string(),
        "version": tags.version.map(version_number),
        "tags": entries,
    })
}

/// Prints one element of the top-level JSON array as soon as it is ready.
///
/// Together with the closing `]` this matches pretty-printing the whole array,
/// without holding every file's tags in memory.
fn print_json_item(item: &Value, first: bool) {
    let pretty = serde_json::to_string_pretty(item).expect("JSON values serialize");
    let opening = if first { "[\n" } else { ",\n" };
    print!("{opening}  {}", pretty.replace('\n', "\n  "));
}

/// Runs `idk show` over every input file and returns the process exit code.
///
/// Files are read concurrently, but output follows input order.
pub async fn run(args: ShowArgs) -> ExitCode {
    let show_progress = !args.json && args.run.progress_for_stdout();
    let fields = args.fields.clone();
    let verbose = args.verbose;
    let inputs = input::resolve(args.input.clone()).await;
    let mut failed = inputs.failures > 0;
    let mut first = true;

    runner::process(
        inputs.files,
        args.run.jobs(),
        Order::Input,
        show_progress,
        move |path| read_tags(path, &fields, verbose),
        |progress, path: PathBuf, result| match result {
            Ok(tags) if args.json => {
                print_json_item(&render_json(&path, tags), std::mem::take(&mut first));
            }
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
        // `first` is still set only when no file produced output.
        println!("{}", if first { "[]" } else { "\n]" });
    }
    runner::exit_code(failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mp3;
    use id3::TagLike;
    use id3::frame::Comment;
    use tempfile::TempDir;

    #[test]
    fn repeated_keys_get_numbered_suffixes() {
        let mut keys = UniqueKeys::default();
        let claimed: Vec<String> = ["APIC", "APIC", "TIT2", "APIC#2", "APIC"]
            .into_iter()
            .map(|key| keys.claim(key.to_owned()))
            .collect();

        assert_eq!(claimed, ["APIC", "APIC#2", "TIT2", "APIC#2#2", "APIC#3"]);
    }

    fn entry(key: &str, value: &str) -> (String, Option<String>) {
        (key.to_owned(), Some(value.to_owned()))
    }

    fn absent(key: &str) -> (String, Option<String>) {
        (key.to_owned(), None)
    }

    /// The entries that have a value.
    fn present(tags: &FileTags) -> Vec<(String, Option<String>)> {
        tags.entries
            .iter()
            .filter(|(_, value)| value.is_some())
            .cloned()
            .collect()
    }

    #[test]
    fn names_known_fields_and_keys_the_rest_by_frame_id() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| {
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
                absent("album"),
                absent("albumartist"),
                entry("artist", "Artist"),
                absent("date"),
                absent("discnumber"),
                absent("genre"),
                absent("title"),
                absent("tracknumber"),
                entry("txxx:custom", "x"),
                entry("COMM:note", "n"),
                entry("TENC", "Encoder"),
            ]
        );
    }

    #[test]
    fn lists_absent_albumartist_among_present_frames() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_artist("Artist"));

        let tags = read_tags(&path, &[], false).unwrap();

        assert!(tags.entries.contains(&absent("albumartist")));
        assert!(tags.entries.contains(&entry("artist", "Artist")));
    }

    #[test]
    fn lists_requested_field_even_when_absent() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_artist("Artist"));

        let tags = read_tags(&path, &[TagField::AlbumArtist], false).unwrap();

        assert_eq!(tags.entries, [absent("albumartist")]);
    }

    #[test]
    fn present_albumartist_is_not_listed_twice() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| tag.set_album_artist("Various Artists"));

        let tags = read_tags(&path, &[], false).unwrap();

        let count = tags
            .entries
            .iter()
            .filter(|(key, _)| key.starts_with("albumartist"))
            .count();
        assert_eq!(count, 1);
        assert!(
            tags.entries
                .contains(&entry("albumartist", "Various Artists"))
        );
    }

    #[test]
    fn filters_to_requested_fields() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| {
            tag.set_artist("Artist");
            tag.set_title("Title");
            tag.set_album("Album");
        });

        let tags = read_tags(&path, &[TagField::Album, TagField::Artist], false).unwrap();

        assert_eq!(
            tags.entries,
            [entry("album", "Album"), entry("artist", "Artist")]
        );
    }

    #[test]
    fn joins_multiple_values_and_disambiguates_duplicate_keys() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| {
            tag.set_text("TCON", "Rock\0Pop");
            tag.set_text("TDRC", "2021");
            tag.set_text("TYER", "2021");
        });

        let tags = read_tags(&path, &[], false).unwrap();

        assert_eq!(
            tags.entries,
            [
                absent("album"),
                absent("albumartist"),
                absent("artist"),
                entry("date", "2021"),
                entry("date#2", "2021"),
                absent("discnumber"),
                entry("genre", "Rock; Pop"),
                absent("title"),
                absent("tracknumber"),
            ]
        );
    }

    #[test]
    fn hides_private_copyright_and_encoder_frames_unless_verbose() {
        let dir = TempDir::new().unwrap();
        let path = mp3(&dir, |tag| {
            tag.set_artist("Artist");
            tag.add_frame(id3::frame::Private {
                owner_identifier: "www.amazon.com".into(),
                private_data: vec![0; 512],
            });
            tag.set_text("TCOP", "");
            tag.set_text("TSSE", "LAME 3.100");
        });

        let quiet = read_tags(&path, &[], false).unwrap();
        assert_eq!(present(&quiet), [entry("artist", "Artist")]);

        let verbose = read_tags(&path, &[], true).unwrap();
        assert_eq!(
            present(&verbose),
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
            entries: vec![entry("title", "T"), entry("artist", "A"), absent("album")],
        };

        let value = render_json(Path::new("a.mp3"), tags);

        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"path":"a.mp3","version":"2.3","tags":{"title":"T","artist":"A","album":null}}"#
        );
    }
}
