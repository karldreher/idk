//! Per-file plans, outcomes and run reporting shared by write operations.

use std::path::Path;
use std::process::ExitCode;

use id3::Tag;
use indicatif::ProgressBar;

use crate::tag_field::TagField;

/// A field's value before and after a write.
#[derive(Debug, PartialEq, Eq)]
pub struct Change {
    /// The field that changed.
    pub field: TagField,
    /// The field's previous value, if it had one.
    pub old: Option<String>,
    /// The field's new value, or `None` when it was removed.
    pub new: Option<String>,
}

impl Change {
    /// Compares `field` in `before` and `after`, returning the change if its value differs.
    pub fn between(field: &TagField, before: &Tag, after: &Tag) -> Option<Change> {
        let (old, new) = (field.read(before), field.read(after));
        (old != new).then(|| Change {
            field: field.clone(),
            old,
            new,
        })
    }
}

/// What happened to a single file.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The file was written; one entry per changed field.
    Updated(Vec<Change>),
    /// The field already held the target value; the file was not written.
    Unchanged,
    /// The operation did not apply to this file; the file was not written.
    Skipped,
}

/// What an operation would do to a file, decided before anything is written.
pub enum Plan {
    /// The tag changes; `tag` holds the new state to write.
    Write {
        /// The updated tag.
        tag: Tag,
        /// Every changed field; never empty.
        changes: Vec<Change>,
    },
    /// The field already holds the target value.
    Unchanged,
    /// The operation does not apply to this file.
    Skipped,
}

impl Plan {
    /// Writes the planned tag to `path` if it changed, and reports the outcome.
    ///
    /// With `dry_run`, nothing is written but the outcome is the same.
    pub fn apply(self, path: &Path, dry_run: bool) -> id3::Result<Outcome> {
        match self {
            Plan::Write { tag, changes } => {
                if !dry_run {
                    tag.write_to_path(path, tag.version())?;
                }
                Ok(Outcome::Updated(changes))
            }
            Plan::Unchanged => Ok(Outcome::Unchanged),
            Plan::Skipped => Ok(Outcome::Skipped),
        }
    }
}

/// Per-run tallies used for dry-run output, the final summary and the exit code.
pub struct Report {
    dry_run: bool,
    updated: usize,
    unchanged: usize,
    skipped: usize,
    failed: usize,
}

impl Report {
    /// A report for a run; `dry_run` prints changes instead of writing them.
    pub fn new(dry_run: bool) -> Self {
        Report {
            dry_run,
            updated: 0,
            unchanged: 0,
            skipped: 0,
            failed: 0,
        }
    }

    /// Records one file's result, printing failures to stderr above the progress bar.
    ///
    /// In a dry run, each pending change is printed to stdout.
    pub fn record(&mut self, path: &Path, result: Result<Outcome, String>, progress: &ProgressBar) {
        match result {
            Ok(Outcome::Updated(changes)) => {
                self.updated += 1;
                if self.dry_run {
                    let describe = |value: &Option<String>| {
                        value
                            .as_ref()
                            .map_or("(none)".to_owned(), |value| format!("{value:?}"))
                    };
                    progress.suspend(|| {
                        for change in &changes {
                            println!(
                                "{}: {} {} -> {}",
                                path.display(),
                                change.field,
                                describe(&change.old),
                                describe(&change.new)
                            );
                        }
                    });
                }
            }
            Ok(Outcome::Unchanged) => self.unchanged += 1,
            Ok(Outcome::Skipped) => self.skipped += 1,
            Err(err) => {
                self.failed += 1;
                progress.suspend(|| eprintln!("error: {}: {err}", path.display()));
            }
        }
    }

    /// One-line summary; `skipped` labels the skipped count, or omits it when `None`.
    pub fn summary(&self, skipped: Option<&str>) -> String {
        let updated = if self.dry_run {
            "would be updated"
        } else {
            "updated"
        };
        let skipped = skipped.map_or(String::new(), |label| {
            format!(", {} skipped ({label})", self.skipped)
        });
        format!(
            "{} {updated}, {} unchanged{skipped}, {} failed",
            self.updated, self.unchanged, self.failed
        )
    }

    /// Non-zero when any file failed.
    pub fn exit_code(&self) -> ExitCode {
        if self.failed > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_counts_with_optional_skip_label() {
        let mut report = Report::new(false);
        let progress = ProgressBar::hidden();
        report.record(Path::new("a"), Ok(Outcome::Unchanged), &progress);
        report.record(Path::new("b"), Ok(Outcome::Skipped), &progress);
        report.record(Path::new("c"), Err("boom".into()), &progress);

        assert_eq!(
            report.summary(Some("no artist")),
            "0 updated, 1 unchanged, 1 skipped (no artist), 1 failed"
        );
        assert_eq!(report.summary(None), "0 updated, 1 unchanged, 1 failed");
        assert_eq!(report.exit_code(), ExitCode::FAILURE);
    }

    #[test]
    fn dry_run_summary_says_would_be_updated() {
        let report = Report::new(true);
        assert_eq!(
            report.summary(None),
            "0 would be updated, 0 unchanged, 0 failed"
        );
    }
}
