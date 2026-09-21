//! Tags idk can operate on, and their ID3v2 frame mapping.

use std::ffi::OsStr;
use std::fmt;
use std::str::FromStr;

use clap::builder::{PossibleValue, TypedValueParser};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use id3::frame::{Comment, ExtendedText};
use id3::{Frame, Tag, TagLike, Version};
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
    help: &'static str,
}

/// Every fixed-name field, in help order.
const NAMED: &[Named] = &[
    Named {
        field: TagField::Artist,
        name: "artist",
        aliases: &["tpe1"],
        help: "Lead artist (TPE1)",
    },
    Named {
        field: TagField::AlbumArtist,
        name: "albumartist",
        aliases: &["album-artist", "tpe2"],
        help: "Album artist (TPE2)",
    },
    Named {
        field: TagField::Title,
        name: "title",
        aliases: &["tit2"],
        help: "Track title (TIT2)",
    },
    Named {
        field: TagField::Album,
        name: "album",
        aliases: &["talb"],
        help: "Album title (TALB)",
    },
    Named {
        field: TagField::TrackNumber,
        name: "tracknumber",
        aliases: &["track", "trck"],
        help: "Track number (TRCK)",
    },
    Named {
        field: TagField::DiscNumber,
        name: "discnumber",
        aliases: &["disc", "tpos"],
        help: "Disc number (TPOS)",
    },
    Named {
        field: TagField::Date,
        name: "date",
        aliases: &["year", "tdrc", "tyer"],
        help: "Recording date (TDRC on v2.4, TYER on v2.3)",
    },
    Named {
        field: TagField::Genre,
        name: "genre",
        aliases: &["tcon"],
        help: "Genre (TCON)",
    },
    Named {
        field: TagField::Composer,
        name: "composer",
        aliases: &["tcom"],
        help: "Composer (TCOM)",
    },
    Named {
        field: TagField::Comment,
        name: "comment",
        aliases: &["comm"],
        help: "Comment (COMM)",
    },
];

/// Prefix for user-defined text fields, e.g. `txxx:MusicBrainz Album Id`.
const TXXX_PREFIX: &str = "txxx:";

impl TagField {
    /// The plain text frame backing this field, for fields stored in exactly one text frame.
    fn text_frame(&self) -> Option<&'static str> {
        match self {
            TagField::Artist => Some("TPE1"),
            TagField::AlbumArtist => Some("TPE2"),
            TagField::Title => Some("TIT2"),
            TagField::Album => Some("TALB"),
            TagField::TrackNumber => Some("TRCK"),
            TagField::DiscNumber => Some("TPOS"),
            TagField::Genre => Some("TCON"),
            TagField::Composer => Some("TCOM"),
            TagField::Date | TagField::Comment | TagField::Txxx(_) => None,
        }
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
                .find(|named| named.field.text_frame() == Some(id))
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
}

/// The leading four-digit year of a date such as `2021-06-01`, or the input unchanged.
fn year_of(value: &str) -> &str {
    match value.get(..4) {
        Some(year) if year.bytes().all(|b| b.is_ascii_digit()) => year,
        _ => value,
    }
}

impl FromStr for TagField {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(prefix) = s.get(..TXXX_PREFIX.len())
            && prefix.eq_ignore_ascii_case(TXXX_PREFIX)
        {
            let description = &s[TXXX_PREFIX.len()..];
            return if description.is_empty() {
                Err(())
            } else {
                Ok(TagField::Txxx(description.to_owned()))
            };
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
            .ok_or(())
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
        name.parse()
            .map_err(|()| serde::de::Error::custom(format!("unknown tag {name:?}")))
    }
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
        raw.parse().map_err(|()| {
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

/// Every accepted field for help and error output.
fn possible_values() -> impl Iterator<Item = PossibleValue> {
    NAMED
        .iter()
        .map(|named| {
            PossibleValue::new(named.name)
                .aliases(named.aliases.iter().copied())
                .help(named.help)
        })
        .chain([PossibleValue::new("txxx:<description>")
            .help("User-defined text frame with the given description (TXXX)")])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<TagField, ()> {
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
        assert_eq!(parse("txxx:"), Err(()));
    }

    #[test]
    fn rejects_unknown_names() {
        assert_eq!(parse("artists"), Err(()));
        assert_eq!(parse(""), Err(()));
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
