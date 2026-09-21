//! End-to-end tests for `idk copy tags`.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use id3::{Tag, TagLike, Version};
use predicates::prelude::*;
use predicates::str::contains;
use tempfile::TempDir;

/// Writes an MP3 stand-in with the given artist / album artist.
fn mp3(dir: &TempDir, name: &str, artist: Option<&str>, album_artist: Option<&str>) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    tag.set_title(name);
    if let Some(artist) = artist {
        tag.set_artist(artist);
    }
    if let Some(album_artist) = album_artist {
        tag.set_album_artist(album_artist);
    }
    tag.write_to_path(&path, Version::Id3v24).unwrap();
    path
}

fn tag(path: &Path) -> Tag {
    Tag::read_from_path(path).unwrap()
}

fn idk() -> Command {
    Command::cargo_bin("idk").unwrap()
}

#[test]
fn prints_version() {
    idk()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("idk {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn copies_artist_to_album_artist_across_many_files() {
    let dir = TempDir::new().unwrap();
    let files: Vec<_> = (0..3)
        .map(|i| {
            mp3(
                &dir,
                &format!("{i}.mp3"),
                Some(&format!("Artist {i}")),
                Some("Old"),
            )
        })
        .collect();

    idk()
        .args(["copy", "tags", "--from", "artist", "--to", "albumartist"])
        .args(&files)
        .assert()
        .success()
        .stdout(contains("3 updated"));

    for (i, file) in files.iter().enumerate() {
        let tag = tag(file);
        assert_eq!(tag.album_artist(), Some(format!("Artist {i}").as_str()));
        assert_eq!(tag.title(), Some(format!("{i}.mp3").as_str()));
    }
}

#[test]
fn copies_album_artist_to_artist() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), Some("Album Artist"));

    idk()
        .args(["copy", "tags", "--from", "albumartist", "--to", "artist"])
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("1 updated"));

    assert_eq!(tag(&file).artist(), Some("Album Artist"));
}

#[test]
fn accepts_frame_id_and_hyphenated_aliases() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), None);

    idk()
        .args(["copy", "tags", "--from", "TPE1", "--to", "album-artist"])
        .arg(&file)
        .assert()
        .success();

    assert_eq!(tag(&file).album_artist(), Some("Artist"));
}

#[test]
fn skips_empty_source_by_default() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", None, Some("Album Artist"));

    idk()
        .args(["copy", "tags", "--from", "artist", "--to", "albumartist"])
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("1 skipped (no artist)"));

    assert_eq!(tag(&file).album_artist(), Some("Album Artist"));
}

#[test]
fn fail_on_empty_reports_and_fails() {
    let dir = TempDir::new().unwrap();
    let empty = mp3(&dir, "empty.mp3", None, Some("Album Artist"));
    let full = mp3(&dir, "full.mp3", Some("Artist"), None);

    idk()
        .args([
            "copy",
            "tags",
            "--from",
            "artist",
            "--to",
            "albumartist",
            "--fail-on-empty",
        ])
        .args([&empty, &full])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("empty.mp3: no artist value"))
        .stdout(contains("1 updated").and(contains("1 failed")));

    assert_eq!(tag(&full).album_artist(), Some("Artist"));
}

#[test]
fn missing_file_fails_without_stopping_others() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), None);
    let missing = dir.path().join("missing.mp3");

    idk()
        .args(["copy", "tags", "--from", "artist", "--to", "albumartist"])
        .args([&missing, &file])
        .assert()
        .code(1)
        .stderr(contains("missing.mp3"))
        .stdout(contains("1 updated").and(contains("1 failed")));

    assert_eq!(tag(&file).album_artist(), Some("Artist"));
}

#[test]
fn rejects_identical_source_and_destination() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), None);

    idk()
        .args(["copy", "tags", "--from", "artist", "--to", "TPE1"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(contains("--from and --to must name different tags"));
}

#[test]
fn requires_from_to_and_files() {
    idk()
        .args(["copy", "tags", "--from", "artist", "x.mp3"])
        .assert()
        .code(2)
        .stderr(contains("--to <TO>"));
    idk()
        .args(["copy", "tags", "--from", "artist", "--to", "albumartist"])
        .assert()
        .code(2)
        .stderr(contains("<FILES>"));
}

#[test]
fn quiet_hides_summary() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), None);

    idk()
        .args([
            "copy",
            "tags",
            "--from",
            "artist",
            "--to",
            "albumartist",
            "--quiet",
        ])
        .arg(&file)
        .assert()
        .success()
        .stdout("");
}
