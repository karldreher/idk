//! Helpers shared by the end-to-end tests.

// Each test binary compiles this module but uses only some of the helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use id3::{Tag, Version};

/// Bytes standing in for MPEG audio; only their preservation matters.
pub const AUDIO: [u8; 5] = [0xFF, 0xFB, 0x90, 0x64, 0x00];

/// The `idk` binary under test.
pub fn idk() -> Command {
    Command::cargo_bin("idk").unwrap()
}

/// Writes `dir/name` (creating parent directories): stand-in audio with an
/// ID3v2.4 tag built by `build`.
pub fn mp3(dir: &Path, name: &str, build: impl FnOnce(&mut Tag)) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, AUDIO).unwrap();
    let mut tag = Tag::new();
    build(&mut tag);
    tag.write_to_path(&path, Version::Id3v24).unwrap();
    path
}

/// The tag currently in the file at `path`.
pub fn tag(path: &Path) -> Tag {
    Tag::read_from_path(path).unwrap()
}

/// Writes a config file `dir/name` with `yaml`.
pub fn write_config(dir: &Path, name: &str, yaml: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, yaml).unwrap();
    path
}
