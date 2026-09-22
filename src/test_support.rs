//! Fixtures shared by unit tests.

use std::path::PathBuf;

use id3::{Tag, Version};
use tempfile::TempDir;

/// Bytes standing in for MPEG audio; only their preservation matters.
pub const AUDIO: &[u8] = &[0xFF, 0xFB, 0x90, 0x64, 0x00, 0x11, 0x22, 0x33];

/// Writes `song.mp3` in `dir`: stand-in audio, plus `tag` at `version` when given.
pub fn write_mp3(dir: &TempDir, tag: Option<&Tag>, version: Version) -> PathBuf {
    let path = dir.path().join("song.mp3");
    std::fs::write(&path, AUDIO).unwrap();
    if let Some(tag) = tag {
        tag.write_to_path(&path, version).unwrap();
    }
    path
}

/// Writes `song.mp3` in `dir` with an ID3v2.4 tag built by `build`.
pub fn mp3(dir: &TempDir, build: impl FnOnce(&mut Tag)) -> PathBuf {
    let mut tag = Tag::new();
    build(&mut tag);
    write_mp3(dir, Some(&tag), Version::Id3v24)
}
