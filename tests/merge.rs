//! End-to-end tests for `idk merge genres` and `idk merge artists`.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use id3::{Tag, TagLike, Version};
use predicates::prelude::*;
use predicates::str::contains;
use tempfile::TempDir;

fn mp3(dir: &TempDir, name: &str, genre: Option<&str>, artist: Option<&str>) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    tag.set_title(name);
    if let Some(genre) = genre {
        tag.set_genre(genre);
    }
    if let Some(artist) = artist {
        tag.set_artist(artist);
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

fn write_config(dir: &TempDir, name: &str, yaml: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, yaml).unwrap();
    path
}

const CONFIG: &str = r#"tags:
  merge:
    genres:
      from:
        - "Heavy Metal"
        - "Metal"
      to: Rock
    artists:
      from:
        - "Beatles"
        - "the beatles"
      to: The Beatles
"#;

#[test]
fn merges_genres_from_cli() {
    let dir = TempDir::new().unwrap();
    let heavy = mp3(&dir, "a.mp3", Some("Heavy Metal"), None);
    let metal = mp3(&dir, "b.mp3", Some("metal"), None);
    let jazz = mp3(&dir, "c.mp3", Some("Jazz"), None);

    idk()
        .args([
            "merge",
            "genres",
            "--from",
            "Heavy Metal",
            "Metal",
            "--to",
            "Rock",
        ])
        .args([&heavy, &metal, &jazz])
        .assert()
        .success()
        .stdout(contains("2 updated, 1 unchanged, 0 failed"));

    assert_eq!(tag(&heavy).genre(), Some("Rock"));
    assert_eq!(tag(&metal).genre(), Some("Rock"));
    assert_eq!(tag(&jazz).genre(), Some("Jazz"));
    assert_eq!(tag(&heavy).title(), Some("a.mp3"));
}

#[test]
fn repeated_from_flags_are_combined() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Metal"), None);

    idk()
        .args([
            "merge",
            "genres",
            "--to",
            "Rock",
            "--from",
            "Heavy Metal",
            "--from",
            "Metal",
        ])
        .arg("--")
        .arg(&file)
        .assert()
        .success();

    assert_eq!(tag(&file).genre(), Some("Rock"));
}

#[test]
fn merges_genres_and_artists_from_config() {
    let dir = TempDir::new().unwrap();
    let config = write_config(&dir, "idk.yaml", CONFIG);
    let file = mp3(&dir, "a.mp3", Some("Heavy Metal"), Some("the Beatles"));

    idk()
        .args(["merge", "genres", "--config"])
        .arg(&config)
        .arg(&file)
        .assert()
        .success();
    idk()
        .current_dir(dir.path())
        .args(["merge", "artists", "a.mp3", "--config"])
        .assert()
        .success();

    let tag = tag(&file);
    assert_eq!(tag.genre(), Some("Rock"));
    assert_eq!(tag.artist(), Some("The Beatles"));
}

#[test]
fn dry_run_previews_merge() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Metal"), None);
    let bytes = std::fs::read(&file).unwrap();

    idk()
        .args([
            "merge",
            "genres",
            "--from",
            "metal",
            "--to",
            "Rock",
            "--dry-run",
        ])
        .arg(&file)
        .assert()
        .success()
        .stdout(
            contains(r#"a.mp3: genre "Metal" -> "Rock""#)
                .and(contains("1 would be updated, 0 unchanged, 0 failed")),
        );

    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}

#[test]
fn empty_source_fills_missing_genre() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", None, Some("Artist"));

    idk()
        .args(["merge", "genres", "--from", "", "--to", "Unknown"])
        .arg(&file)
        .assert()
        .success();

    assert_eq!(tag(&file).genre(), Some("Unknown"));
}

#[test]
fn config_and_usage_errors_exit_2_before_touching_files() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Metal"), None);
    let bytes = std::fs::read(&file).unwrap();
    let cases = [
        (
            "tags:\n  merge:\n    genres:\n      from: [Metal]\n      to: [Rock]\n",
            "tags.merge.genres.to: invalid type: sequence",
        ),
        (
            "tags:\n  merge:\n    genres:\n      from: []\n      to: Rock\n",
            "tags.merge.genres.from: must list at least one value",
        ),
        (
            "tags:\n  merge:\n    artists:\n      from: [a]\n      to: b\n",
            "missing `tags.merge.genres`",
        ),
        (
            "tags:\n  merge:\n    genre:\n      from: [Metal]\n      to: Rock\n",
            "tags.merge: unknown field `genre`",
        ),
        (
            "merge:\n  genres:\n    from: [Metal]\n    to: Rock\n",
            "unknown field `merge`",
        ),
    ];

    for (yaml, message) in cases {
        let config = write_config(&dir, "c.yaml", yaml);
        idk()
            .args(["merge", "genres", "--config"])
            .arg(&config)
            .arg(&file)
            .assert()
            .code(2)
            .stderr(contains(message));
    }

    idk()
        .args(["merge", "genres", "--from", "Metal", "--to", " "])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(contains("--to must not be empty"));
    idk()
        .args(["merge", "genres", "--to", "Rock", "--config", "c.yaml"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(contains("cannot be used with"));
    idk()
        .args(["merge", "genres", "--from", "Metal"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(contains("--to <VALUE>"));

    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}
