//! End-to-end tests for `idk schema write` and `idk schema validate`.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::Value;
use tempfile::TempDir;

fn idk() -> Command {
    Command::cargo_bin("idk").unwrap()
}

const VALID: &str = r#"# yaml-language-server: $schema=./idk.yaml.json
tags:
  merge:
    genres:
      from:
        - "Heavy Metal"
        - "Metal"
      to: Rock
    artists:
      from: [Beatles]
      to: The Beatles
  copy:
    from: artist
    to: albumartist
"#;

#[test]
fn writes_schema_to_default_path() {
    let dir = TempDir::new().unwrap();

    idk()
        .current_dir(dir.path())
        .args(["schema", "write"])
        .assert()
        .success()
        .stdout(
            contains("wrote idk.yaml.json")
                .and(contains("# yaml-language-server: $schema=./idk.yaml.json")),
        );

    let schema: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("idk.yaml.json")).unwrap()).unwrap();
    assert_eq!(schema["title"], "idk config");
    assert_eq!(schema["required"], serde_json::json!(["tags"]));
}

#[test]
fn writes_schema_to_given_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("my-cool-file.yaml.json");

    idk()
        .args(["schema", "write", "--file"])
        .arg(&path)
        .assert()
        .success();

    assert!(path.exists());
}

#[test]
fn written_schema_validates_configs_on_its_own() {
    let dir = TempDir::new().unwrap();
    idk()
        .current_dir(dir.path())
        .args(["schema", "write"])
        .assert()
        .success();
    let schema: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("idk.yaml.json")).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();

    let valid: Value = serde_yaml_ng::from_str(VALID).unwrap();
    assert!(validator.is_valid(&valid));

    let invalid: Value =
        serde_yaml_ng::from_str("tags:\n  merge:\n    genres:\n      from: []\n      to: [Rock]\n")
            .unwrap();
    assert!(!validator.is_valid(&invalid));
}

#[test]
fn validates_default_config() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("idk.yaml"), VALID).unwrap();

    idk()
        .current_dir(dir.path())
        .args(["schema", "validate"])
        .assert()
        .success()
        .stdout("idk.yaml: valid\n");
}

#[test]
fn validate_reports_every_violation_and_exits_1() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("bad.yaml");
    std::fs::write(
        &config,
        "tags:\n  merge:\n    genres:\n      from: []\n      to: [Rock]\n  copy:\n    from: artists\n    to: title\n",
    )
    .unwrap();

    idk()
        .args(["schema", "validate", "--config"])
        .arg(&config)
        .assert()
        .code(1)
        .stderr(
            contains("tags.merge.genres.from: [] has less than 1 item")
                .and(contains(
                    "tags.merge.genres.to: [\"Rock\"] is not of type \"string\"",
                ))
                .and(contains(
                    "tags.copy.from: \"artists\" is not a known tag name",
                )),
        );
}

#[test]
fn validate_missing_file_exits_1() {
    let dir = TempDir::new().unwrap();

    idk()
        .current_dir(dir.path())
        .args(["schema", "validate"])
        .assert()
        .code(1)
        .stderr(contains("idk.yaml"));
}
