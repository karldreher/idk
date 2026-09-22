//! End-to-end tests for `idk find`.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use id3::{Tag, TagLike, Version};
use predicates::prelude::*;
use predicates::str::contains;
use tempfile::TempDir;

fn mp3(dir: &Path, name: &str, build: impl FnOnce(&mut Tag)) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    build(&mut tag);
    tag.write_to_path(&path, Version::Id3v24).unwrap();
    path
}

fn idk() -> Command {
    Command::cargo_bin("idk").unwrap()
}

/// A library of three files, run from inside `dir` so output paths are short.
fn library() -> TempDir {
    let dir = TempDir::new().unwrap();
    mp3(dir.path(), "a.mp3", |tag| {
        tag.set_artist("Artist");
        tag.set_album_artist("Artist");
        tag.set_genre("Rock");
        tag.set_title("Live at Home");
    });
    mp3(dir.path(), "b.mp3", |tag| {
        tag.set_artist("Guest");
        tag.set_album_artist("Various Artists");
        tag.set_genre("rock");
    });
    mp3(dir.path(), "c.mp3", |tag| tag.set_artist("Solo"));
    dir
}

fn find(dir: &TempDir, args: &[&str]) -> String {
    let output = idk()
        .current_dir(dir.path())
        .arg("find")
        .args(args)
        .args(["a.mp3", "b.mp3", "c.mp3"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn filters_by_each_operator() {
    let dir = library();
    assert_eq!(find(&dir, &["--where", "genre=Rock"]), "a.mp3\n");
    assert_eq!(find(&dir, &["--where", "genre!=Rock"]), "b.mp3\nc.mp3\n");
    assert_eq!(find(&dir, &["--where", "title~^Live"]), "a.mp3\n");
    assert_eq!(find(&dir, &["--missing", "genre"]), "c.mp3\n");
    assert_eq!(find(&dir, &["--present", "albumartist"]), "a.mp3\nb.mp3\n");
    assert_eq!(find(&dir, &["--where", "genre="]), "c.mp3\n");
}

#[test]
fn compares_fields_with_each_other() {
    let dir = library();
    assert_eq!(
        find(&dir, &["--where", "albumartist!=@artist"]),
        "b.mp3\nc.mp3\n"
    );
    assert_eq!(find(&dir, &["--where", "albumartist=@artist"]), "a.mp3\n");
}

#[test]
fn combines_conditions_and_ignores_case_on_request() {
    let dir = library();
    assert_eq!(find(&dir, &["--where", "genre=rock"]), "b.mp3\n");
    assert_eq!(
        find(&dir, &["-i", "--where", "genre=rock"]),
        "a.mp3\nb.mp3\n"
    );
    assert_eq!(find(&dir, &["--where", "albumartist~various"]), "");
    assert_eq!(
        find(
            &dir,
            &[
                "-i",
                "--where",
                "genre=ROCK",
                "--where",
                "albumartist~^various"
            ]
        ),
        "b.mp3\n"
    );
}

#[test]
fn prints_every_file_without_conditions() {
    let dir = library();
    assert_eq!(find(&dir, &[]), "a.mp3\nb.mp3\nc.mp3\n");
}

#[test]
fn null_separates_output() {
    let dir = library();
    assert_eq!(find(&dir, &["--present", "genre", "-0"]), "a.mp3\0b.mp3\0");
}

#[test]
fn output_pipes_into_other_commands() {
    let dir = library();
    let matches = find(&dir, &["--missing", "genre", "-0"]);

    idk()
        .current_dir(dir.path())
        .args([
            "set",
            "tags",
            "--field",
            "genre=Unknown",
            "--files-from",
            "-",
        ])
        .write_stdin(matches)
        .assert()
        .success()
        .stdout(contains("1 updated"));

    assert_eq!(
        Tag::read_from_path(dir.path().join("c.mp3"))
            .unwrap()
            .genre(),
        Some("Unknown")
    );
}

#[test]
fn unreadable_files_fail_but_matches_still_print() {
    let dir = library();

    idk()
        .current_dir(dir.path())
        .args(["find", "--where", "genre=Rock", "missing.mp3", "a.mp3"])
        .assert()
        .code(1)
        .stdout("a.mp3\n")
        .stderr(contains("missing.mp3"));
}

#[test]
fn invalid_conditions_exit_2() {
    let cases = [
        ("genre", "expected FIELD=VALUE, FIELD!=VALUE or FIELD~REGEX"),
        ("genres=Rock", "invalid value 'genres'"),
        ("title~(", "bad regex"),
        ("artist=@nope", "invalid value 'nope'"),
    ];
    for (condition, message) in cases {
        idk()
            .args(["find", "--where", condition, "x.mp3"])
            .assert()
            .code(2)
            .stderr(contains(message).and(contains("possible values").or(contains("condition"))));
    }
}
