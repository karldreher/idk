//! Per-file plans, outcomes and the shared runner for every write command.

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use clap::error::ErrorKind;
use id3::Tag;
use indicatif::ProgressBar;

use crate::cli::{RunOptions, WriteOptions, usage_error};
use crate::input::Inputs;
use crate::runner::{self, Order};
use crate::tag_field::TagField;

/// Plans and applies an edit to every file concurrently, then prints the summary.
///
/// In a dry run each pending change is printed instead of written. Without
/// `--confirm`, files are planned concurrently but shown and written one at a time,
/// in input order, after the user answers. `skipped` labels the skipped count in
/// the summary, or omits it when `None`.
pub async fn run_writes(
    inputs: Inputs,
    run: &RunOptions,
    write: &WriteOptions,
    skipped: Option<&str>,
    plan: impl Fn(&Path) -> Result<Plan, String> + Send + Sync + 'static,
) -> ExitCode {
    let dry_run = write.dry_run;
    let prompts = write.prompts();
    if prompts && !std::io::stdin().is_terminal() {
        usage_error(
            ErrorKind::MissingRequiredArgument,
            "writing needs confirmation but stdin is not a terminal: \
             pass --confirm to write without prompting, or --dry-run to preview",
        );
    }
    let mut report = Report::new(dry_run);
    report.failed = inputs.failures;
    let mut prompter = Prompter::new(BufReader::new(std::io::stdin()));
    let files = if inputs.unique {
        inputs.files
    } else {
        runner::unique_files(inputs.files).await
    };
    let order = if prompts {
        Order::Input
    } else {
        Order::Completion
    };
    runner::process_unique(
        files,
        run.jobs(),
        order,
        !run.quiet,
        move |path| {
            let plan = plan(path)?;
            if prompts {
                Ok(Pending::Review(plan))
            } else {
                plan.apply(path, dry_run)
                    .map(Pending::Done)
                    .map_err(|err| err.to_string())
            }
        },
        |progress, path, result| {
            let result = result.and_then(|pending| match pending {
                Pending::Done(outcome) => Ok(outcome),
                Pending::Review(Plan::Write { tag, changes }) => {
                    if prompter.approve(&path, &changes, progress) {
                        Plan::Write { tag, changes }
                            .apply(&path, false)
                            .map_err(|err| err.to_string())
                    } else {
                        Ok(Outcome::Declined)
                    }
                }
                Pending::Review(other) => other.apply(&path, false).map_err(|err| err.to_string()),
            });
            report.record(&path, result, progress);
        },
    )
    .await;

    if !run.quiet {
        println!("{}", report.summary(skipped));
    }
    runner::exit_code(report.failed > 0)
}

/// A planned file on its way back from a worker.
enum Pending {
    /// Already written (or previewed); only the report is left.
    Done(Outcome),
    /// Waiting for the user's answer before anything is written.
    Review(Plan),
}

/// What the user said at a prompt.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
    /// Write this file and every remaining one without asking.
    All,
    /// Write nothing more.
    Quit,
}

/// Reads an answer: `y`, `n`, `a` or `q` (or the full word), case-insensitive; blank means no.
fn parse_answer(line: &str) -> Option<Answer> {
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(Answer::Yes),
        "" | "n" | "no" => Some(Answer::No),
        "a" | "all" => Some(Answer::All),
        "q" | "quit" => Some(Answer::Quit),
        _ => None,
    }
}

/// Asks, file by file, whether to write.
struct Prompter<R> {
    input: R,
    /// Set by `a` (write everything) or `q` and end of input (write nothing).
    decided: Option<bool>,
}

impl<R: BufRead> Prompter<R> {
    fn new(input: R) -> Self {
        Prompter {
            input,
            decided: None,
        }
    }

    /// Shows the pending changes and returns whether to write the file.
    fn approve(&mut self, path: &Path, changes: &[Change], progress: &ProgressBar) -> bool {
        if let Some(decision) = self.decided {
            return decision;
        }
        progress.suspend(|| {
            print_changes(path, changes);
            loop {
                eprint!("Write {}? [y/N/a/q] ", path.display());
                let _ = std::io::stderr().flush();
                let mut line = String::new();
                if !matches!(self.input.read_line(&mut line), Ok(1..)) {
                    self.decided = Some(false);
                    return false;
                }
                match parse_answer(&line) {
                    Some(Answer::Yes) => return true,
                    Some(Answer::No) => return false,
                    Some(Answer::All) => {
                        self.decided = Some(true);
                        return true;
                    }
                    Some(Answer::Quit) => {
                        self.decided = Some(false);
                        return false;
                    }
                    None => eprintln!("Answer y, n, a (all) or q (quit)."),
                }
            }
        })
    }
}

/// Prints one `path: field old -> new` line per change to stdout.
fn print_changes(path: &Path, changes: &[Change]) {
    let describe = |value: &Option<String>| {
        value
            .as_ref()
            .map_or("(none)".to_owned(), |value| format!("{value:?}"))
    };
    for change in changes {
        println!(
            "{}: {} {} -> {}",
            path.display(),
            change.field,
            describe(&change.old),
            describe(&change.new)
        );
    }
}

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

/// The current values of `fields` in `tag`, to hand to [`Plan::from_diff`] after an edit.
///
/// Reading values up front avoids cloning the whole tag, which can carry large pictures.
pub fn snapshot<'a>(fields: impl Iterator<Item = &'a TagField>, tag: &Tag) -> Vec<Option<String>> {
    fields.map(|field| field.read(tag)).collect()
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
    /// The user declined the change; the file was not written.
    Declined,
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
    /// The plan for an edit of `fields` that left `after`, given their `old` values from
    /// [`snapshot`]. Lists each field whose value changed; `Unchanged` when none did, so
    /// the file is not rewritten.
    pub fn from_diff<'a>(
        fields: impl IntoIterator<Item = &'a TagField>,
        old: Vec<Option<String>>,
        after: Tag,
    ) -> Plan {
        let changes: Vec<Change> = fields
            .into_iter()
            .zip(old)
            .filter_map(|(field, old)| {
                let new = field.read(&after);
                (old != new).then(|| Change {
                    field: field.clone(),
                    old,
                    new,
                })
            })
            .collect();
        if changes.is_empty() {
            Plan::Unchanged
        } else {
            Plan::Write {
                tag: after,
                changes,
            }
        }
    }

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
struct Report {
    dry_run: bool,
    updated: usize,
    unchanged: usize,
    skipped: usize,
    declined: usize,
    failed: usize,
}

impl Report {
    /// A report for a run; `dry_run` prints changes instead of writing them.
    fn new(dry_run: bool) -> Self {
        Report {
            dry_run,
            updated: 0,
            unchanged: 0,
            skipped: 0,
            declined: 0,
            failed: 0,
        }
    }

    /// Records one file's result, printing failures to stderr above the progress bar.
    ///
    /// In a dry run, each pending change is printed to stdout.
    fn record(&mut self, path: &Path, result: Result<Outcome, String>, progress: &ProgressBar) {
        match result {
            Ok(Outcome::Updated(changes)) => {
                self.updated += 1;
                if self.dry_run {
                    progress.suspend(|| print_changes(path, &changes));
                }
            }
            Ok(Outcome::Unchanged) => self.unchanged += 1,
            Ok(Outcome::Skipped) => self.skipped += 1,
            Ok(Outcome::Declined) => self.declined += 1,
            Err(err) => {
                self.failed += 1;
                progress.suspend(|| eprintln!("error: {}: {err}", path.display()));
            }
        }
    }

    /// One-line summary; `skipped` labels the skipped count, or omits it when `None`.
    fn summary(&self, skipped: Option<&str>) -> String {
        let updated = if self.dry_run {
            "would be updated"
        } else {
            "updated"
        };
        let skipped = skipped.map_or(String::new(), |label| {
            format!(", {} skipped ({label})", self.skipped)
        });
        let declined = if self.declined > 0 {
            format!(", {} declined", self.declined)
        } else {
            String::new()
        };
        format!(
            "{} {updated}, {} unchanged{skipped}{declined}, {} failed",
            self.updated, self.unchanged, self.failed
        )
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
        assert_eq!(report.failed, 1);
    }

    #[test]
    fn dry_run_summary_says_would_be_updated() {
        let report = Report::new(true);
        assert_eq!(
            report.summary(None),
            "0 would be updated, 0 unchanged, 0 failed"
        );
    }

    #[test]
    fn summary_lists_declined_only_when_present() {
        let mut report = Report::new(false);
        report.record(
            Path::new("a"),
            Ok(Outcome::Declined),
            &ProgressBar::hidden(),
        );
        assert_eq!(
            report.summary(None),
            "0 updated, 0 unchanged, 1 declined, 0 failed"
        );
    }

    #[test]
    fn parses_answers() {
        assert_eq!(parse_answer("y\n"), Some(Answer::Yes));
        assert_eq!(parse_answer(" YES "), Some(Answer::Yes));
        assert_eq!(parse_answer("n"), Some(Answer::No));
        assert_eq!(parse_answer("\n"), Some(Answer::No));
        assert_eq!(parse_answer("A"), Some(Answer::All));
        assert_eq!(parse_answer("quit"), Some(Answer::Quit));
        assert_eq!(parse_answer("maybe"), None);
    }

    fn approvals(input: &str, files: usize) -> Vec<bool> {
        let mut prompter = Prompter::new(input.as_bytes());
        let progress = ProgressBar::hidden();
        (0..files)
            .map(|_| prompter.approve(Path::new("a.mp3"), &[], &progress))
            .collect()
    }

    #[test]
    fn prompter_asks_for_each_file() {
        assert_eq!(approvals("y\nn\ny\n", 3), [true, false, true]);
    }

    #[test]
    fn prompter_reasks_after_unrecognized_answer() {
        assert_eq!(approvals("what\ny\n", 1), [true]);
    }

    #[test]
    fn all_approves_the_rest_without_reading() {
        assert_eq!(approvals("a\n", 3), [true, true, true]);
    }

    #[test]
    fn quit_and_end_of_input_decline_the_rest() {
        assert_eq!(approvals("y\nq\ny\n", 3), [true, false, false]);
        assert_eq!(approvals("y\n", 3), [true, false, false]);
    }
}
