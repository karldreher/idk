//! End-to-end tests for `idk show`.

use std::path::PathBuf;

use assert_cmd::Command;
use id3::{Tag, TagLike, Version};
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::{Value, json};
use tempfile::TempDir;

fn mp3(dir: &TempDir, name: &str, artist: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    tag.set_artist(artist);
    tag.set_title(name);
    tag.write_to_path(&path, Version::Id3v24).unwrap();
    path
}

fn idk() -> Command {
    Command::cargo_bin("idk").unwrap()
}

#[test]
fn prints_readable_tags() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", "Artist");

    idk().arg("show").arg(&file).assert().success().stdout(
        contains("a.mp3 (ID3v2.4)")
            .and(contains("  artist: Artist"))
            .and(contains("  title: a.mp3")),
    );
}

#[test]
fn prints_json_in_input_order() {
    let dir = TempDir::new().unwrap();
    let files: Vec<_> = (0..20)
        .map(|i| mp3(&dir, &format!("{i:02}.mp3"), &format!("Artist {i}")))
        .collect();

    let output = idk()
        .args(["show", "--json", "-j", "8"])
        .args(&files)
        .output()
        .unwrap();

    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let parsed = parsed.as_array().unwrap();
    assert_eq!(parsed.len(), files.len());
    for (i, (entry, file)) in parsed.iter().zip(&files).enumerate() {
        assert_eq!(entry["path"], json!(file.display().to_string()));
        assert_eq!(entry["version"], json!("2.4"));
        assert_eq!(entry["tags"]["artist"], json!(format!("Artist {i}")));
    }
}

#[test]
fn filters_fields() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", "Artist");

    let output = idk()
        .args(["show", "--json", "--field", "artist"])
        .arg(&file)
        .output()
        .unwrap();

    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed[0]["tags"], json!({ "artist": "Artist" }));
}

#[test]
fn untagged_file_is_not_an_error() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("bare.mp3");
    std::fs::write(&file, [0xFF, 0xFB]).unwrap();

    let output = idk().args(["show", "--json"]).arg(&file).output().unwrap();

    assert!(output.status.success());
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed[0]["version"], Value::Null);
    assert_eq!(parsed[0]["tags"], json!({}));
}

#[test]
fn missing_file_fails_but_others_are_printed() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", "Artist");
    let missing = dir.path().join("missing.mp3");

    let output = idk()
        .args(["show", "--json"])
        .args([&missing, &file])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.mp3"));
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 1);
    assert_eq!(parsed[0]["tags"]["artist"], json!("Artist"));
}

#[test]
fn does_not_modify_files() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", "Artist");
    let bytes = std::fs::read(&file).unwrap();
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();

    idk().arg("show").arg(&file).assert().success();
    idk().args(["show", "--json"]).arg(&file).assert().success();

    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        modified
    );
}

#[test]
fn verbose_reveals_hidden_frames() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("amazon.mp3");
    std::fs::write(&file, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    tag.set_artist("Artist");
    tag.add_frame(id3::frame::Private {
        owner_identifier: "www.amazon.com".into(),
        private_data: vec![0; 1024],
    });
    tag.set_text("TCOP", "");
    tag.set_text("TSSE", "LAME");
    tag.write_to_path(&file, Version::Id3v24).unwrap();

    let default = idk().args(["show", "--json"]).arg(&file).output().unwrap();
    let default: Value = serde_json::from_slice(&default.stdout).unwrap();
    assert_eq!(default[0]["tags"], json!({ "artist": "Artist" }));

    let verbose = idk()
        .args(["show", "--json", "-v"])
        .arg(&file)
        .output()
        .unwrap();
    let verbose: Value = serde_json::from_slice(&verbose.stdout).unwrap();
    assert_eq!(
        verbose[0]["tags"],
        json!({
            "artist": "Artist",
            "PRIV": "www.amazon.com, 1024 bytes",
            "TCOP": "",
            "TSSE": "LAME",
        })
    );

    idk()
        .arg("show")
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("PRIV").not().and(contains("artist: Artist")));
}
