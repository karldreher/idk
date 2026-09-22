//! End-to-end tests for `idk apply`.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use id3::frame::{Comment, ExtendedText};
use id3::{Tag, TagLike, Version};
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::{Value, json};
use tempfile::TempDir;

fn mp3(dir: &Path, name: &str, build: impl FnOnce(&mut Tag)) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    build(&mut tag);
    tag.write_to_path(&path, Version::Id3v24).unwrap();
    path
}

fn rich(tag: &mut Tag) {
    tag.set_artist("Artist");
    tag.set_title("Title");
    tag.set_genre("Rock\0Pop");
    tag.set_text("TDRC", "2021-06-01");
    tag.set_text("TENC", "Encoder");
    tag.add_frame(Comment {
        lang: "eng".into(),
        description: "".into(),
        text: "plain".into(),
    });
    tag.add_frame(Comment {
        lang: "eng".into(),
        description: "note".into(),
        text: "described".into(),
    });
    tag.add_frame(ExtendedText {
        description: "Source".into(),
        value: "CD".into(),
    });
}

fn idk() -> Command {
    Command::cargo_bin("idk").unwrap()
}

fn show_json(dir: &TempDir, files: &[&str]) -> Value {
    let output = idk()
        .current_dir(dir.path())
        .args(["show", "--json"])
        .args(files)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn unedited_show_output_is_a_no_op() {
    let dir = TempDir::new().unwrap();
    let a = mp3(dir.path(), "a.mp3", rich);
    let b = mp3(dir.path(), "b.mp3", |tag| tag.set_artist("B"));
    let before = [std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()];
    let json = show_json(&dir, &["a.mp3", "b.mp3"]).to_string();

    idk()
        .current_dir(dir.path())
        .args(["apply", "-"])
        .write_stdin(json)
        .assert()
        .success()
        .stdout(contains("0 updated, 2 unchanged, 0 failed"))
        .stderr(
            contains("warning: skipping `COMM:note`").and(contains("warning: skipping `TENC`")),
        );

    assert_eq!(
        [std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()],
        before
    );
}

#[test]
fn edited_values_are_written_and_nulls_cleared() {
    let dir = TempDir::new().unwrap();
    let a = mp3(dir.path(), "a.mp3", rich);
    let mut edits = show_json(&dir, &["a.mp3"]);
    edits[0]["tags"]["title"] = json!("New Title");
    edits[0]["tags"]["comment"] = Value::Null;
    edits[0]["tags"]["albumartist"] = json!("Various Artists");
    let path = dir.path().join("edits.json");
    std::fs::write(&path, edits.to_string()).unwrap();

    idk()
        .current_dir(dir.path())
        .args(["apply", "edits.json"])
        .assert()
        .success()
        .stdout(contains("1 updated"));

    let tag = Tag::read_from_path(&a).unwrap();
    assert_eq!(tag.title(), Some("New Title"));
    assert_eq!(tag.album_artist(), Some("Various Artists"));
    let comments: Vec<_> = tag.comments().map(|c| c.description.as_str()).collect();
    assert_eq!(comments, ["note"]);
    assert_eq!(tag.genres(), Some(vec!["Rock", "Pop"]));
    assert!(tag.get("TENC").is_some());
}

#[test]
fn dry_run_previews_edits() {
    let dir = TempDir::new().unwrap();
    let a = mp3(dir.path(), "a.mp3", |tag| tag.set_genre("Pop"));
    let bytes = std::fs::read(&a).unwrap();

    idk()
        .current_dir(dir.path())
        .args(["apply", "-", "--dry-run"])
        .write_stdin(r#"[{"path": "a.mp3", "tags": {"genre": "Rock", "artist": null}}]"#)
        .assert()
        .success()
        .stdout(contains(r#"a.mp3: genre "Pop" -> "Rock""#).and(contains("1 would be updated")));

    assert_eq!(std::fs::read(&a).unwrap(), bytes);
}

#[test]
fn missing_files_fail_while_others_apply() {
    let dir = TempDir::new().unwrap();
    let a = mp3(dir.path(), "a.mp3", |_| {});

    idk()
        .current_dir(dir.path())
        .args(["apply", "-"])
        .write_stdin(
            r#"[{"path": "missing.mp3", "tags": {"genre": "Rock"}},
                {"path": "a.mp3", "tags": {"genre": "Rock"}}]"#,
        )
        .assert()
        .code(1)
        .stderr(contains("missing.mp3"))
        .stdout(contains("1 updated, 0 unchanged, 1 failed"));

    assert_eq!(Tag::read_from_path(&a).unwrap().genre(), Some("Rock"));
}

#[test]
fn invalid_input_exits_2_before_touching_files() {
    let dir = TempDir::new().unwrap();
    let a = mp3(dir.path(), "a.mp3", |tag| tag.set_genre("Pop"));
    let bytes = std::fs::read(&a).unwrap();
    let cases = [
        ("not json", "invalid JSON"),
        (r#"{"path": "a.mp3"}"#, "expected a JSON array"),
        (
            r#"[{"path": "a.mp3", "tags": {"genre": 5}}]"#,
            "[0].tags.genre: expected a string or null",
        ),
        (
            r#"[{"path": "a.mp3", "tags": {"genre": "Rock"}}, {"path": "./a.mp3", "tags": {}}]"#,
            "[1].path: ./a.mp3 is already edited by [0]",
        ),
    ];

    for (json, message) in cases {
        idk()
            .current_dir(dir.path())
            .args(["apply", "-"])
            .write_stdin(json)
            .assert()
            .code(2)
            .stderr(contains(message));
    }
    idk()
        .current_dir(dir.path())
        .args(["apply", "nope.json"])
        .assert()
        .code(2)
        .stderr(contains("nope.json"));

    assert_eq!(std::fs::read(&a).unwrap(), bytes);
}
