//! Tags idk can operate on, and their ID3v2 frame mapping.

use std::fmt;

use clap::ValueEnum;

/// A tag named by its common field name (as used by MusicBrainz Picard and mutagen).
///
/// The underlying ID3v2 frame ID is accepted as an alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TagField {
    /// Lead artist (ID3v2 frame TPE1).
    #[value(alias = "tpe1")]
    Artist,
    /// Album artist (ID3v2 frame TPE2).
    #[value(name = "albumartist", alias = "album-artist", alias = "tpe2")]
    AlbumArtist,
}

impl TagField {
    /// The ID3v2.3/v2.4 frame ID that stores this tag.
    pub fn frame_id(self) -> &'static str {
        match self {
            TagField::Artist => "TPE1",
            TagField::AlbumArtist => "TPE2",
        }
    }
}

impl fmt::Display for TagField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().expect("no skipped variants");
        f.write_str(value.get_name())
    }
}
