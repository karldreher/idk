//! Tags idk can operate on, and their ID3v2 frame mapping.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt;
use std::str::FromStr;

use clap::builder::{PossibleValue, TypedValueParser};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use id3::frame::{Comment, ExtendedText};
use id3::{Frame, Tag, TagLike, Version};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer};

/// A tag named by its common field name (as used by MusicBrainz Picard and mutagen).
///
/// Names are case-insensitive, and each field also accepts its ID3v2 frame ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagField {
    /// Lead artist (TPE1).
    Artist,
    /// Album artist (TPE2).
    AlbumArtist,
    /// Track title (TIT2).
    Title,
    /// Album title (TALB).
    Album,
    /// Track number, optionally `n/total` (TRCK).
    TrackNumber,
    /// Disc number, optionally `n/total` (TPOS).
    DiscNumber,
    /// Recording date: TDRC on ID3v2.4, TYER (year only) on ID3v2.3.
    Date,
    /// Genre (TCON).
    Genre,
    /// Composer (TCOM).
    Composer,
    /// Comment with an empty description (COMM).
    Comment,
    /// User-defined text frame with the given description (TXXX).
    Txxx(String),
}

/// A field with a fixed name, for parsing and help output.
struct Named {
    field: TagField,
    name: &'static str,
    aliases: &'static [&'static str],
    /// The single text frame storing the field; `None` for `date` and `comment`.
    frame: Option<&'static str>,
    /// Help text; the frame, when there is one, is appended in parentheses.
    description: &'static str,
}

const fn named(
    field: TagField,
    name: &'static str,
    aliases: &'static [&'static str],
    frame: Option<&'static str>,
    description: &'static str,
) -> Named {
    Named {
        field,
        name,
        aliases,
        frame,
        description,
    }
}

/// Every fixed-name field, in help order.
#[rustfmt::skip]
const NAMED: &[Named] = &[
    named(TagField::Artist, "artist", &["tpe1"], Some("TPE1"), "Lead artist"),
    named(TagField::AlbumArtist, "albumartist", &["album-artist", "tpe2"], Some("TPE2"), "Album artist"),
    named(TagField::Title, "title", &["tit2"], Some("TIT2"), "Track title"),
    named(TagField::Album, "album", &["talb"], Some("TALB"), "Album title"),
    named(TagField::TrackNumber, "tracknumber", &["track", "trck"], Some("TRCK"), "Track number"),
    named(TagField::DiscNumber, "discnumber", &["disc", "tpos"], Some("TPOS"), "Disc number"),
    named(TagField::Date, "date", &["year", "tdrc", "tyer"], None, "Recording date (TDRC on v2.4, TYER on v2.3)"),
    named(TagField::Genre, "genre", &["tcon"], Some("TCON"), "Genre"),
    named(TagField::Composer, "composer", &["tcom"], Some("TCOM"), "Composer"),
    named(TagField::Comment, "comment", &["comm"], None, "Comment (COMM)"),
];

impl Named {
    /// Help text for `--help`, e.g. `Lead artist (TPE1)`.
    fn help(&self) -> String {
        match self.frame {
            Some(frame) => format!("{} ({frame})", self.description),
            None => self.description.to_owned(),
        }
    }
}

/// Prefix for user-defined text fields, e.g. `txxx:MusicBrainz Album Id`.
const TXXX_PREFIX: &str = "txxx:";

impl TagField {
    /// The plain text frame backing this field, for fields stored in exactly one text frame.
    fn text_frame(&self) -> Option<&'static str> {
        NAMED.iter().find(|named| named.field == *self)?.frame
    }

    /// The field a frame belongs to, if idk has a name for it.
    pub fn of_frame(frame: &Frame) -> Option<TagField> {
        match frame.id() {
            "TDRC" | "TYER" => Some(TagField::Date),
            "COMM" => frame
                .content()
                .comment()
                .filter(|comment| comment.description.is_empty())
                .map(|_| TagField::Comment),
            "TXXX" => frame
                .content()
                .extended_text()
                .map(|extended| TagField::Txxx(extended.description.clone())),
            id => NAMED
                .iter()
                .find(|named| named.frame == Some(id))
                .map(|named| named.field.clone()),
        }
    }

    /// Reads this field's value from `tag`, if present.
    pub fn read(&self, tag: &Tag) -> Option<String> {
        let text = |id| tag.get(id).and_then(|frame| frame.content().text());
        let value = match self {
            TagField::Date => match tag.version() {
                Version::Id3v24 => text("TDRC").or_else(|| text("TYER")),
                _ => text("TYER").or_else(|| text("TDRC")),
            },
            TagField::Comment => tag
                .comments()
                .find(|comment| comment.description.is_empty())
                .map(|comment| comment.text.as_str()),
            TagField::Txxx(description) => tag
                .extended_texts()
                .find(|extended| extended.description == *description)
                .map(|extended| extended.value.as_str()),
            field => text(field.text_frame().expect("text field")),
        };
        value.map(str::to_owned)
    }

    /// Writes `value` into this field, replacing any existing value.
    pub fn write(&self, tag: &mut Tag, value: &str) {
        match self {
            TagField::Date if tag.version() == Version::Id3v24 => {
                tag.remove("TYER");
                tag.set_text("TDRC", value);
            }
            TagField::Date => {
                // ID3v2.3 has no full date frame; TYER holds only the year.
                tag.remove("TDRC");
                tag.set_text("TYER", year_of(value));
            }
            TagField::Comment => {
                tag.remove_comment(Some(""), None);
                tag.add_frame(Comment {
                    lang: "eng".to_owned(),
                    description: String::new(),
                    text: value.to_owned(),
                });
            }
            TagField::Txxx(description) => {
                tag.add_frame(ExtendedText {
                    description: description.clone(),
                    value: value.to_owned(),
                });
            }
            field => tag.set_text(field.text_frame().expect("text field"), value),
        }
    }

    /// Removes this field from `tag`.
    ///
    /// `date` removes both TDRC and TYER; `comment` removes only comments with an
    /// empty description; `txxx:<desc>` removes only that description.
    pub fn remove(&self, tag: &mut Tag) {
        match self {
            TagField::Date => {
                tag.remove("TDRC");
                tag.remove("TYER");
            }
            TagField::Comment => tag.remove_comment(Some(""), None),
            TagField::Txxx(description) => tag.remove_extended_text(Some(description), None),
            field => {
                tag.remove(field.text_frame().expect("text field"));
            }
        }
    }
}

/// The leading four-digit year of a date such as `2021-06-01`, or the input unchanged.
fn year_of(value: &str) -> &str {
    match value.get(..4) {
        Some(year) if year.bytes().all(|b| b.is_ascii_digit()) => year,
        _ => value,
    }
}

impl FromStr for TagField {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(prefix) = s.get(..TXXX_PREFIX.len())
            && prefix.eq_ignore_ascii_case(TXXX_PREFIX)
            && s.len() > TXXX_PREFIX.len()
        {
            return Ok(TagField::Txxx(s[TXXX_PREFIX.len()..].to_owned()));
        }
        NAMED
            .iter()
            .find(|named| {
                named.name.eq_ignore_ascii_case(s)
                    || named
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(s))
            })
            .map(|named| named.field.clone())
            .ok_or_else(|| {
                let names: Vec<_> = possible_values().map(|v| v.get_name().to_owned()).collect();
                format!("unknown tag '{s}'; expected one of: {}", names.join(", "))
            })
    }
}

impl fmt::Display for TagField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagField::Txxx(description) => write!(f, "{TXXX_PREFIX}{description}"),
            field => {
                let named = NAMED.iter().find(|named| named.field == *field);
                f.write_str(named.expect("every fixed field is named").name)
            }
        }
    }
}

/// Deserializes a field from its name, as accepted on the command line.
impl<'de> Deserialize<'de> for TagField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        name.parse().map_err(serde::de::Error::custom)
    }
}

/// Schema for a field name: a case-insensitive pattern accepting every name,
/// alias and `txxx:<description>`, with canonical names as examples for editors.
impl JsonSchema for TagField {
    fn schema_name() -> Cow<'static, str> {
        "TagField".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let names: Vec<&str> = NAMED.iter().map(|named| named.name).collect();
        let alternatives: Vec<String> = NAMED
            .iter()
            .flat_map(|named| std::iter::once(named.name).chain(named.aliases.iter().copied()))
            .map(case_insensitive)
            .collect();
        let pattern = format!(
            "^(?:{}|{}.+)$",
            alternatives.join("|"),
            case_insensitive(TXXX_PREFIX)
        );
        json_schema!({
            "description": "A tag name (e.g. artist, albumartist, TPE1) or txxx:<description>",
            "type": "string",
            "pattern": pattern,
            "examples": names,
        })
    }
}

/// A regex matching `literal` in any letter case, e.g. `ab-` → `[aA][bB]-`.
///
/// Field names contain only letters, digits, `-` and `:`, none of which need escaping.
fn case_insensitive(literal: &str) -> String {
    literal
        .chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// clap value parser for [`TagField`], listing every field in `--help`.
#[derive(Clone)]
pub struct TagFieldParser;

impl TypedValueParser for TagFieldParser {
    type Value = TagField;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &OsStr,
    ) -> Result<TagField, clap::Error> {
        let raw = value.to_string_lossy();
        raw.parse().map_err(|_| {
            let mut err = clap::Error::new(ErrorKind::InvalidValue).with_cmd(cmd);
            if let Some(arg) = arg {
                err.insert(
                    ContextKind::InvalidArg,
                    ContextValue::String(arg.to_string()),
                );
            }
            err.insert(
                ContextKind::InvalidValue,
                ContextValue::String(raw.into_owned()),
            );
            err.insert(
                ContextKind::ValidValue,
                ContextValue::Strings(possible_values().map(|v| v.get_name().to_owned()).collect()),
            );
            err
        })
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(possible_values()))
    }
}

/// Parses `FIELD=VALUE` for `idk set tags`, splitting on the first `=`.
///
/// The value must not be empty; removing a field is `idk clear tags`.
pub fn parse_assignment(raw: &str) -> Result<(TagField, String), String> {
    let (name, value) = raw.split_once('=').ok_or("expected FIELD=VALUE")?;
    let field: TagField = name.parse()?;
    if value.is_empty() {
        return Err(format!(
            "empty value for '{field}'; use `idk clear tags --field {field}` to remove it"
        ));
    }
    Ok((field, value.to_owned()))
}

/// Every accepted field for help and error output.
fn possible_values() -> impl Iterator<Item = PossibleValue> {
    NAMED
        .iter()
        .map(|named| {
            PossibleValue::new(named.name)
                .aliases(named.aliases.iter().copied())
                .help(named.help())
        })
        .chain([PossibleValue::new("txxx:<description>")
            .help("User-defined text frame with the given description (TXXX)")])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<TagField, String> {
        s.parse()
    }

    #[test]
    fn parses_names_aliases_and_frame_ids_case_insensitively() {
        assert_eq!(parse("artist"), Ok(TagField::Artist));
        assert_eq!(parse("TPE1"), Ok(TagField::Artist));
        assert_eq!(parse("Album-Artist"), Ok(TagField::AlbumArtist));
        assert_eq!(parse("track"), Ok(TagField::TrackNumber));
        assert_eq!(parse("TYER"), Ok(TagField::Date));
        assert_eq!(parse("comm"), Ok(TagField::Comment));
    }

    #[test]
    fn parses_txxx_preserving_description_case() {
        assert_eq!(
            parse("TXXX:MusicBrainz Album Id"),
            Ok(TagField::Txxx("MusicBrainz Album Id".into()))
        );
        assert!(parse("txxx:").is_err());
    }

    #[test]
    fn rejects_unknown_names() {
        assert!(
            parse("artists")
                .unwrap_err()
                .starts_with("unknown tag 'artists'; expected one of: artist,")
        );
        assert!(parse("").is_err());
    }

    #[test]
    fn displays_canonical_names() {
        assert_eq!(TagField::AlbumArtist.to_string(), "albumartist");
        assert_eq!(TagField::Txxx("custom".into()).to_string(), "txxx:custom");
        for named in NAMED {
            assert_eq!(parse(&named.field.to_string()), Ok(named.field.clone()));
        }
    }

    #[test]
    fn round_trips_every_field() {
        let fields = NAMED
            .iter()
            .map(|named| named.field.clone())
            .chain([TagField::Txxx("custom".into())]);
        for field in fields {
            let mut tag = Tag::with_version(Version::Id3v24);
            field.write(&mut tag, "2021-06-01 value");
            assert_eq!(
                field.read(&tag).as_deref(),
                Some("2021-06-01 value"),
                "{field}"
            );
        }
    }

    #[test]
    fn maps_frames_back_to_fields() {
        let mut tag = Tag::new();
        TagField::Title.write(&mut tag, "t");
        TagField::Txxx("custom".into()).write(&mut tag, "x");
        TagField::Comment.write(&mut tag, "c");
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: "note".into(),
            text: "n".into(),
        });
        tag.set_text("TYER", "1999");
        tag.set_text("TENC", "encoder");

        let fields: Vec<_> = tag.frames().map(TagField::of_frame).collect();

        assert_eq!(
            fields,
            [
                Some(TagField::Title),
                Some(TagField::Txxx("custom".into())),
                Some(TagField::Comment),
                None,
                Some(TagField::Date),
                None,
            ]
        );
    }

    #[test]
    fn date_uses_tdrc_on_v24_and_tyer_on_v23() {
        let mut v24 = Tag::with_version(Version::Id3v24);
        v24.set_text("TYER", "1999");
        TagField::Date.write(&mut v24, "2021-06-01");
        assert_eq!(
            v24.get("TDRC").and_then(|f| f.content().text()),
            Some("2021-06-01")
        );
        assert!(v24.get("TYER").is_none());

        let mut v23 = Tag::with_version(Version::Id3v23);
        TagField::Date.write(&mut v23, "2021-06-01");
        assert_eq!(
            v23.get("TYER").and_then(|f| f.content().text()),
            Some("2021")
        );
        assert!(v23.get("TDRC").is_none());
        assert_eq!(TagField::Date.read(&v23).as_deref(), Some("2021"));
    }

    #[test]
    fn comment_replaces_only_undescribed_comments() {
        let mut tag = Tag::new();
        tag.add_frame(Comment {
            lang: "deu".into(),
            description: "".into(),
            text: "alt".into(),
        });
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: "note".into(),
            text: "keep".into(),
        });

        TagField::Comment.write(&mut tag, "new");

        let comments: Vec<_> = tag
            .comments()
            .map(|c| (c.description.as_str(), c.text.as_str()))
            .collect();
        assert_eq!(comments, [("note", "keep"), ("", "new")]);
    }

    #[test]
    fn remove_is_scoped_like_write() {
        let mut tag = Tag::with_version(Version::Id3v24);
        tag.set_text("TDRC", "2021");
        tag.set_text("TYER", "2021");
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: "".into(),
            text: "plain".into(),
        });
        tag.add_frame(Comment {
            lang: "eng".into(),
            description: "note".into(),
            text: "keep".into(),
        });
        TagField::Txxx("drop".into()).write(&mut tag, "x");
        TagField::Txxx("keep".into()).write(&mut tag, "y");
        tag.set_title("Title");

        for field in [
            TagField::Date,
            TagField::Comment,
            TagField::Txxx("drop".into()),
            TagField::Title,
        ] {
            field.remove(&mut tag);
            assert_eq!(field.read(&tag), None, "{field}");
        }

        let ids: Vec<_> = tag.frames().map(|f| f.id().to_owned()).collect();
        assert_eq!(ids, ["COMM", "TXXX"]);
        assert_eq!(tag.comments().next().unwrap().description, "note");
        assert_eq!(
            TagField::Txxx("keep".into()).read(&tag).as_deref(),
            Some("y")
        );
    }

    #[test]
    fn txxx_is_scoped_to_its_description() {
        let mut tag = Tag::new();
        tag.add_frame(ExtendedText {
            description: "other".into(),
            value: "keep".into(),
        });

        TagField::Txxx("custom".into()).write(&mut tag, "one");
        TagField::Txxx("custom".into()).write(&mut tag, "two");

        assert_eq!(
            TagField::Txxx("custom".into()).read(&tag).as_deref(),
            Some("two")
        );
        assert_eq!(
            TagField::Txxx("other".into()).read(&tag).as_deref(),
            Some("keep")
        );
        assert_eq!(tag.extended_texts().count(), 2);
    }
}
