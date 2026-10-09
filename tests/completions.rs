//! End-to-end tests for `idk completions`.

use predicates::prelude::*;
use predicates::str::contains;

mod common;
use common::idk;

#[test]
fn prints_a_script_for_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        idk()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(contains("idk"))
            .stdout(predicate::str::is_empty().not())
            .stderr(predicate::str::is_empty());
    }
}

#[test]
fn scripts_cover_nested_subcommands_and_flags() {
    for shell in ["bash", "zsh", "fish"] {
        idk()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(contains("copy").and(contains("dry-run")));
    }
}

#[test]
fn tag_field_values_complete() {
    for shell in ["zsh", "fish"] {
        idk()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(contains("albumartist"));
    }
}

#[test]
fn rejects_unknown_shell() {
    idk()
        .args(["completions", "nushell"])
        .assert()
        .code(2)
        .stderr(contains("invalid value"));
}

#[test]
fn listed_in_help() {
    idk()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("completions"));
}

#[test]
fn help_groups_subcommands_by_action() {
    idk().arg("--help").assert().success().stdout(
        contains("Read:\n  find")
            .and(contains("Edit:\n  apply"))
            .and(contains("Setup:\n  completions"))
            .and(contains("Other:\n  help")),
    );
}
