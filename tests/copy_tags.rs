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

#[test]
fn copies_between_text_comment_and_txxx_fields() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), None);

    idk()
        .args(["copy", "tags", "--from", "title", "--to", "comment"])
        .arg(&file)
        .assert()
        .success();
    idk()
        .args([
            "copy",
            "tags",
            "--from",
            "comment",
            "--to",
            "TXXX:Original Title",
        ])
        .arg(&file)
        .assert()
        .success();

    let tag = tag(&file);
    let comment = tag.comments().find(|c| c.description.is_empty()).unwrap();
    assert_eq!(comment.text, "a.mp3");
    let custom = tag
        .extended_texts()
        .find(|t| t.description == "Original Title")
        .unwrap();
    assert_eq!(custom.value, "a.mp3");
    assert_eq!(tag.artist(), Some("Artist"));
}

#[test]
fn copies_date_to_year_on_id3v23() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.mp3");
    std::fs::write(&file, [0xFF, 0xFB, 0x90, 0x64, 0x00]).unwrap();
    let mut tag = Tag::new();
    tag.add_frame(id3::frame::ExtendedText {
        description: "released".into(),
        value: "2021-06-01".into(),
    });
    tag.write_to_path(&file, Version::Id3v23).unwrap();

    idk()
        .args(["copy", "tags", "--from", "txxx:released", "--to", "date"])
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("1 updated"));

    let tag = self::tag(&file);
    assert_eq!(tag.version(), Version::Id3v23);
    assert_eq!(
        tag.get("TYER").and_then(|f| f.content().text()),
        Some("2021")
    );
}

#[test]
fn rejects_unknown_field_listing_valid_ones() {
    idk()
        .args([
            "copy", "tags", "--from", "artists", "--to", "title", "x.mp3",
        ])
        .assert()
        .code(2)
        .stderr(contains("invalid value 'artists'").and(contains("txxx:<description>")));
}

#[test]
fn dry_run_previews_without_modifying_files() {
    let dir = TempDir::new().unwrap();
    let changed = mp3(&dir, "changed.mp3", Some("New"), Some("Old"));
    let unset = mp3(&dir, "unset.mp3", Some("Artist"), None);
    let before: Vec<_> = [&changed, &unset]
        .iter()
        .map(|f| {
            (
                std::fs::read(f).unwrap(),
                std::fs::metadata(f).unwrap().modified().unwrap(),
            )
        })
        .collect();

    idk()
        .args([
            "copy",
            "tags",
            "--from",
            "artist",
            "--to",
            "albumartist",
            "--dry-run",
        ])
        .args([&changed, &unset])
        .assert()
        .success()
        .stdout(
            contains(r#"changed.mp3: albumartist "Old" -> "New""#)
                .and(contains(r#"unset.mp3: albumartist (none) -> "Artist""#))
                .and(contains("2 would be updated")),
        );

    let after: Vec<_> = [&changed, &unset]
        .iter()
        .map(|f| {
            (
                std::fs::read(f).unwrap(),
                std::fs::metadata(f).unwrap().modified().unwrap(),
            )
        })
        .collect();
    assert_eq!(after, before);
}

#[test]
fn dry_run_exit_code_matches_real_run() {
    let dir = TempDir::new().unwrap();
    let empty = mp3(&dir, "empty.mp3", None, None);

    idk()
        .args([
            "copy",
            "tags",
            "--from",
            "artist",
            "--to",
            "albumartist",
            "-n",
            "--fail-on-empty",
        ])
        .arg(&empty)
        .assert()
        .code(1)
        .stderr(contains("empty.mp3: no artist value"));
}

fn write_config(dir: &TempDir, name: &str, yaml: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, yaml).unwrap();
    path
}

#[test]
fn reads_fields_from_config_file() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), Some("Old"));
    let config = write_config(
        &dir,
        "custom.yaml",
        "tags:\n  copy:\n    from: artist\n    to: albumartist\n",
    );

    idk()
        .args(["copy", "tags", "--config"])
        .arg(&config)
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("1 updated"));

    assert_eq!(tag(&file).album_artist(), Some("Artist"));
}

#[test]
fn bare_config_flag_reads_idk_yaml_from_working_directory() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), None);
    write_config(
        &dir,
        "idk.yaml",
        "tags:\n  copy:\n    from: artist\n    to: albumartist\n",
    );

    idk()
        .current_dir(dir.path())
        .args(["copy", "tags", "a.mp3", "--config"])
        .assert()
        .success();

    assert_eq!(tag(&file).album_artist(), Some("Artist"));
}

#[test]
fn config_conflicts_with_from_and_to() {
    let dir = TempDir::new().unwrap();
    let config = write_config(
        &dir,
        "c.yaml",
        "tags:\n  copy:\n    from: artist\n    to: title\n",
    );

    idk()
        .args(["copy", "tags", "--from", "artist", "--config"])
        .arg(&config)
        .arg("x.mp3")
        .assert()
        .code(2)
        .stderr(contains("cannot be used with"));
}

#[test]
fn invalid_config_exits_1_before_touching_files() {
    let dir = TempDir::new().unwrap();
    let file = mp3(&dir, "a.mp3", Some("Artist"), None);
    let bytes = std::fs::read(&file).unwrap();
    let cases = [
        ("tags: {}\n", "missing `tags.copy`"),
        (
            "copy:\n  from: artist\n  to: title\n",
            "\"tags\" is a required property",
        ),
        (
            "tags:\n  copy:\n    from: artist\n    too: title\n",
            "tags.copy: Additional properties are not allowed ('too' was unexpected)",
        ),
        (
            "tags:\n  copy:\n    from: artists\n    to: title\n",
            "tags.copy.from: \"artists\" is not a known tag name",
        ),
        (
            "tags:\n  copy:\n    from: artist\n    to: TPE1\n",
            "tags.copy: from and to must name different tags",
        ),
    ];

    for (yaml, message) in cases {
        let config = write_config(&dir, "c.yaml", yaml);
        idk()
            .args(["copy", "tags", "--config"])
            .arg(&config)
            .arg(&file)
            .assert()
            .code(1)
            .stderr(contains(message));
    }

    idk()
        .args(["copy", "tags", "--config"])
        .arg(dir.path().join("missing.yaml"))
        .arg(&file)
        .assert()
        .code(1)
        .stderr(contains("missing.yaml"));

    assert_eq!(std::fs::read(&file).unwrap(), bytes);
}
