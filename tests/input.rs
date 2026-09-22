//! End-to-end tests for input resolution: globs, directories and file lists.
//!
//! `assert_cmd` runs idk without a shell, so glob patterns reach idk
//! unexpanded, exactly as they do from Windows `cmd` and PowerShell.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use id3::{Tag, TagLike, Version};
use predicates::str::contains;
use tempfile::TempDir;

fn mp3(dir: &Path, name: &str, artist: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    tag.set_artist(artist);
    tag.write_to_path(&path, Version::Id3v24).unwrap();
    path
}

fn idk() -> Command {
    Command::cargo_bin("idk").unwrap()
}

#[test]
fn expands_glob_patterns_itself() {
    let dir = TempDir::new().unwrap();
    mp3(dir.path(), "b.mp3", "B");
    mp3(dir.path(), "a.mp3", "A");

    idk()
        .current_dir(dir.path())
        .args(["show", "--field", "artist", "*.mp3"])
        .assert()
        .success()
        .stdout("a.mp3 (ID3v2.4)\n  artist: A\n\nb.mp3 (ID3v2.4)\n  artist: B\n");
}

#[test]
fn recursive_updates_a_nested_library() {
    let dir = TempDir::new().unwrap();
    let files = [
        mp3(dir.path(), "Artist/Album 1/01.mp3", "Artist"),
        mp3(dir.path(), "Artist/Album 2/01.mp3", "Artist"),
    ];
    std::fs::write(dir.path().join("Artist/Album 1/cover.jpg"), b"jpg").unwrap();

    idk()
        .args([
            "copy",
            "tags",
            "--from",
            "artist",
            "--to",
            "albumartist",
            "-r",
        ])
        .arg(dir.path())
        .assert()
        .success()
        .stdout(contains("2 updated"));

    for file in files {
        assert_eq!(
            Tag::read_from_path(file).unwrap().album_artist(),
            Some("Artist")
        );
    }
}

#[test]
fn directory_without_recursive_fails_but_other_inputs_run() {
    let dir = TempDir::new().unwrap();
    let file = mp3(dir.path(), "a.mp3", "Artist");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();

    idk()
        .args(["set", "tags", "--field", "genre=Rock"])
        .args([&sub, &file])
        .assert()
        .code(1)
        .stderr(contains("sub: is a directory (use -r)"))
        .stdout(contains("1 updated, 0 unchanged, 1 failed"));

    assert_eq!(Tag::read_from_path(&file).unwrap().genre(), Some("Rock"));
}

#[test]
fn unmatched_glob_fails_but_other_inputs_run() {
    let dir = TempDir::new().unwrap();
    let file = mp3(dir.path(), "a.mp3", "Artist");

    idk()
        .current_dir(dir.path())
        .args(["show", "*.flac", "a.mp3"])
        .assert()
        .code(1)
        .stderr(contains("*.flac: no files match"))
        .stdout(contains("a.mp3 (ID3v2.4)"));
    assert!(file.exists());
}

#[test]
fn inputs_are_deduplicated_across_sources() {
    let dir = TempDir::new().unwrap();
    mp3(dir.path(), "a.mp3", "Artist");

    idk()
        .current_dir(dir.path())
        .args([
            "set",
            "tags",
            "--field",
            "genre=Rock",
            "a.mp3",
            "*.mp3",
            "-r",
            ".",
        ])
        .assert()
        .success()
        .stdout(contains("1 updated, 0 unchanged, 0 failed"));
}

#[test]
fn reads_inputs_from_stdin() {
    let dir = TempDir::new().unwrap();
    let a = mp3(dir.path(), "a.mp3", "A");
    let b = mp3(dir.path(), "b b.mp3", "B");

    idk()
        .args(["set", "tags", "--field", "genre=Rock", "--files-from", "-"])
        .write_stdin(format!("{}\n{}\n", a.display(), b.display()))
        .assert()
        .success()
        .stdout(contains("2 updated"));

    for file in [a, b] {
        assert_eq!(Tag::read_from_path(file).unwrap().genre(), Some("Rock"));
    }
}

#[test]
fn needs_files_or_files_from() {
    idk()
        .args(["show"])
        .assert()
        .code(2)
        .stderr(contains("<FILES>"));
}
