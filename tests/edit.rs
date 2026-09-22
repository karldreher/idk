//! End-to-end tests for `idk set tags` and `idk clear tags`.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use id3::{Tag, TagLike, Version};
use predicates::prelude::*;
use predicates::str::contains;
use tempfile::TempDir;

const AUDIO: [u8; 5] = [0xFF, 0xFB, 0x90, 0x64, 0x00];

fn mp3(dir: &TempDir, name: &str, build: impl FnOnce(&mut Tag)) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, AUDIO).unwrap();
    let mut tag = Tag::new();
    tag.set_title(name);
    build(&mut tag);
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
fn sets_fields_across_many_files() {
    let dir = TempDir::new().unwrap();
    let files: Vec<_> = (0..3)
        .map(|i| mp3(&dir, &format!("{i}.mp3"), |tag| tag.set_genre("Pop")))
        .collect();

    idk()
        .args(["set", "tags", "--field", "albumartist=Various Artists"])
        .args(["--field", "genre=Rock", "--field", "txxx:Source=CD"])
        .args(&files)
        .assert()
        .success()
        .stdout(contains("3 updated, 0 unchanged, 0 failed"));

    for (i, file) in files.iter().enumerate() {
        let tag = tag(file);
        assert_eq!(tag.album_artist(), Some("Various Artists"));
        assert_eq!(tag.genre(), Some("Rock"));
        let source = tag
            .extended_texts()
            .find(|t| t.description == "Source")
            .unwrap();
        assert_eq!(source.value, "CD");
        assert_eq!(tag.title(), Some(format!("{i}.mp3").as_str()));
    }
}

#[test]
fn value_may_contain_equals_signs() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", |_| {});

    idk()
        .args(["set", "tags", "--field", "comment=a=b"])
        .arg(&file)
        .assert()
        .success();

    let tag = tag(&file);
    assert_eq!(tag.comments().next().unwrap().text, "a=b");
}

#[test]
fn creates_tag_on_untagged_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("bare.mp3");
    std::fs::write(&file, AUDIO).unwrap();

    idk()
        .args(["set", "tags", "--field", "artist=Artist"])
        .arg(&file)
        .assert()
        .success();

    let tag = tag(&file);
    assert_eq!(tag.artist(), Some("Artist"));
    assert_eq!(tag.version(), Version::Id3v24);
}

#[test]
fn set_dry_run_previews_each_field_without_writing() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", |tag| tag.set_genre("Pop"));
    let bytes = std::fs::read(&file).unwrap();

    idk()
        .args([
            "set",
            "tags",
            "--field",
            "genre=Rock",
            "--field",
            "album=Album",
            "-n",
        ])
        .arg(&file)
        .assert()
        .success()
        .stdout(
            contains(r#"a.mp3: genre "Pop" -> "Rock""#)
                .and(contains(r#"a.mp3: album (none) -> "Album""#))
                .and(contains("1 would be updated")),
        );

    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}

#[test]
fn set_leaves_matching_files_untouched() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", |tag| tag.set_genre("Rock"));
    let bytes = std::fs::read(&file).unwrap();

    idk()
        .args(["set", "tags", "--field", "genre=Rock"])
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("0 updated, 1 unchanged"));

    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}

#[test]
fn set_usage_errors_exit_2() {
    let cases = [
        (vec!["--field", "genre"], "expected FIELD=VALUE"),
        (
            vec!["--field", "genre="],
            "empty value for 'genre'; use `idk clear tags --field genre` to remove it",
        ),
        (vec!["--field", "genres=Rock"], "invalid value 'genres'"),
        (
            vec!["--field", "genre=Rock", "--field", "TCON=Pop"],
            "--field genre given more than once",
        ),
        (vec!["x.mp3"], "--field <FIELD=VALUE>"),
    ];
    for (args, message) in cases {
        idk()
            .args(["set", "tags"])
            .args(&args)
            .arg("x.mp3")
            .assert()
            .code(2)
            .stderr(contains(message));
    }
}

#[test]
fn clears_fields_and_leaves_others() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", |tag| {
        tag.set_genre("Rock");
        tag.set_text("TDRC", "2021");
        tag.add_frame(id3::frame::Comment {
            lang: "eng".into(),
            description: "".into(),
            text: "stray".into(),
        });
        tag.add_frame(id3::frame::ExtendedText {
            description: "Source".into(),
            value: "CD".into(),
        });
    });

    idk()
        .args(["clear", "tags", "--field", "comment", "--field", "year"])
        .args(["--field", "txxx:Source"])
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("1 updated, 0 unchanged, 0 failed"));

    let tag = tag(&file);
    assert_eq!(tag.comments().count(), 0);
    assert!(tag.get("TDRC").is_none());
    assert_eq!(tag.extended_texts().count(), 0);
    assert_eq!(tag.genre(), Some("Rock"));
    assert_eq!(tag.title(), Some("a.mp3"));
}

#[test]
fn clear_dry_run_shows_removals() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", |tag| tag.set_genre("Rock"));
    let bytes = std::fs::read(&file).unwrap();

    idk()
        .args(["clear", "tags", "--field", "genre", "--dry-run"])
        .arg(&file)
        .assert()
        .success()
        .stdout(contains(r#"a.mp3: genre "Rock" -> (none)"#).and(contains("1 would be updated")));

    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}

#[test]
fn clear_on_untagged_or_absent_field_is_unchanged() {
    let dir = TempDir::new().unwrap();
    let bare = dir.path().join("bare.mp3");
    std::fs::write(&bare, AUDIO).unwrap();
    let tagged = mp3(&dir, "a.mp3", |_| {});

    idk()
        .args(["clear", "tags", "--field", "genre"])
        .args([&bare, &tagged])
        .assert()
        .success()
        .stdout(contains("0 updated, 2 unchanged"));

    assert_eq!(std::fs::read(&bare).unwrap(), AUDIO);
}

#[test]
fn clear_usage_errors_exit_2() {
    idk()
        .args([
            "clear", "tags", "--field", "genre", "--field", "TCON", "x.mp3",
        ])
        .assert()
        .code(2)
        .stderr(contains("--field genre given more than once"));
    idk()
        .args(["clear", "tags", "--field", "genres", "x.mp3"])
        .assert()
        .code(2)
        .stderr(contains("invalid value 'genres'"));
}
