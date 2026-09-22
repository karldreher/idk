//! Command-line argument definitions.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use crate::find::{Condition, ConditionParser};
use crate::tag_field::{AssignmentParser, TagField, TagFieldParser};

/// Config file used when `--config` is given without a path.
pub const DEFAULT_CONFIG: &str = "idk.yaml";

/// Where `idk schema write` puts the schema by default.
pub const DEFAULT_SCHEMA: &str = "idk.yaml.json";

/// Input format shown at the end of `idk apply --help`.
const APPLY_HELP: &str = r#"Input is a JSON array with one object per file:

  [
    {"path": "a.mp3", "tags": {"artist": "Artist", "albumartist": "Artist"}},
    {"path": "b.mp3", "tags": {"genre": null}}
  ]

A string sets the field, null clears it, and fields left out are untouched.
Keys that aren't field names are skipped with a warning; other keys on the
object are ignored. `idk show --json` produces this format, if you want to
start from the current tags."#;

/// Top-level help groups: heading and the subcommands listed under it, alphabetized.
const COMMAND_GROUPS: &[(&str, &[&str])] = &[
    ("Read", &["find", "show"]),
    ("Edit", &["apply", "clear", "copy", "merge", "set"]),
    ("Setup", &["completions", "schema"]),
];

/// The `help` subcommand clap adds at build time, listed last in the top-level help.
const HELP_SUBCOMMAND: (&str, &str) = (
    "help",
    "Print this message or the help of the given subcommand(s)",
);

/// The command model with top-level subcommands listed by [`COMMAND_GROUPS`].
///
/// clap lists every subcommand under one heading, so the top-level help is a custom template.
pub fn command() -> clap::Command {
    let cmd = Cli::command();
    let about = |name: &str| {
        cmd.find_subcommand(name)
            .and_then(|sub| sub.get_about())
            .map_or_else(String::new, ToString::to_string)
    };
    let mut rows: Vec<Option<(&str, String)>> = Vec::new();
    for (_, names) in COMMAND_GROUPS {
        rows.extend(names.iter().map(|name| Some((*name, about(name)))));
        rows.push(None);
    }
    let width = rows
        .iter()
        .flatten()
        .map(|(name, _)| name.len())
        .chain([HELP_SUBCOMMAND.0.len()])
        .max()
        .unwrap_or_default();
    let mut groups = String::new();
    let mut rows = rows.into_iter();
    for (heading, _) in COMMAND_GROUPS {
        groups.push_str(&format!("{heading}:\n"));
        for (name, about) in rows.by_ref().map_while(|row| row) {
            groups.push_str(&format!("  {name:width$}  {about}\n"));
        }
        groups.push('\n');
    }
    let (name, about) = HELP_SUBCOMMAND;
    groups.push_str(&format!("Other:\n  {name:width$}  {about}\n"));
    // Braces in the listing would be read as template tags.
    let groups = groups.replace('{', "{{").replace('}', "}}");
    let template = format!(
        "{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n{groups}\nOptions:\n{{options}}{{after-help}}"
    );
    cmd.help_template(template)
}

/// The ID3 Knife: automate MP3 ID3 tag operations.
#[derive(Parser)]
#[command(name = "idk", version, about)]
pub struct Cli {
    /// Operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level operations.
#[derive(Subcommand)]
pub enum Command {
    /// Copy metadata within each input file.
    Copy {
        /// What to copy.
        #[command(subcommand)]
        target: CopyTarget,
    },
    /// Print the files whose tags match every condition.
    Find(FindArgs),
    /// Merge variant values of a field into one value.
    Merge {
        /// Which field to merge.
        #[command(subcommand)]
        target: MergeTarget,
    },
    /// Apply per-file tag edits from a JSON file.
    Apply(ApplyArgs),
    /// Remove fields.
    Clear {
        /// What to clear.
        #[command(subcommand)]
        target: ClearTarget,
    },
    /// Set fields to fixed values.
    Set {
        /// What to set.
        #[command(subcommand)]
        target: SetTarget,
    },
    /// Print the tags of each input file.
    Show(ShowArgs),
    /// Work with the JSON Schema for config files.
    Schema {
        /// What to do.
        #[command(subcommand)]
        action: SchemaAction,
    },
    /// Print a shell completion script to stdout.
    Completions {
        /// Shell to generate the script for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// Actions for `idk schema`.
#[derive(Subcommand)]
pub enum SchemaAction {
    /// Write the config JSON Schema to a file.
    Write {
        /// Where to write the schema.
        #[arg(long, value_name = "FILE", default_value = DEFAULT_SCHEMA)]
        file: PathBuf,
    },
    /// Validate a config file against the schema.
    Validate {
        /// Config file to validate.
        #[arg(long, value_name = "FILE", default_value = DEFAULT_CONFIG)]
        config: PathBuf,
    },
}

/// Arguments for `idk apply`.
#[derive(Args)]
#[command(after_help = APPLY_HELP)]
pub struct ApplyArgs {
    /// JSON array of {"path", "tags"} objects; "-" reads stdin.
    #[arg(value_name = "PATH")]
    pub source: PathBuf,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,

    /// Write options.
    #[command(flatten)]
    pub write: WriteOptions,
}

/// Arguments for `idk find`.
#[derive(Args)]
pub struct FindArgs {
    /// Condition (repeatable): FIELD=VALUE, FIELD!=VALUE or FIELD~REGEX.
    ///
    /// A missing field compares as "". A VALUE of @FIELD compares against another
    /// field, e.g. albumartist!=@artist. All conditions must match.
    #[arg(long = "where", value_name = "COND", value_parser = ConditionParser)]
    pub conditions: Vec<Condition>,

    /// Match files where this field is absent or empty (repeatable).
    #[arg(long, value_name = "FIELD", value_parser = TagFieldParser)]
    pub missing: Vec<TagField>,

    /// Match files where this field has a value (repeatable).
    #[arg(long, value_name = "FIELD", value_parser = TagFieldParser)]
    pub present: Vec<TagField>,

    /// Compare values and regexes case-insensitively.
    #[arg(short, long)]
    pub ignore_case: bool,

    /// Separate printed paths with NUL instead of newlines (for xargs -0 or --files-from).
    #[arg(short = '0', long)]
    pub null: bool,

    /// Input files.
    #[command(flatten)]
    pub input: InputArgs,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,
}

/// Things `idk clear` can clear.
#[derive(Subcommand)]
pub enum ClearTarget {
    /// Remove one or more fields.
    Tags(ClearTagsArgs),
}

/// Arguments for `idk clear tags`.
#[derive(Args)]
pub struct ClearTagsArgs {
    /// Field to remove (repeatable).
    #[arg(long = "field", value_name = "FIELD", value_parser = TagFieldParser, required = true)]
    pub fields: Vec<TagField>,

    /// Input files.
    #[command(flatten)]
    pub input: InputArgs,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,

    /// Write options.
    #[command(flatten)]
    pub write: WriteOptions,
}

/// Things `idk set` can set.
#[derive(Subcommand)]
pub enum SetTarget {
    /// Set one or more fields to fixed values, overwriting them.
    Tags(SetTagsArgs),
}

/// Arguments for `idk set tags`.
#[derive(Args)]
pub struct SetTagsArgs {
    /// Field and value, split on the first `=` (repeatable), e.g. albumartist="Various Artists".
    ///
    /// FIELD takes the same names as `idk copy tags --from`, including txxx:<description>.
    #[arg(
        long = "field",
        value_name = "FIELD=VALUE",
        value_parser = AssignmentParser,
        required = true
    )]
    pub fields: Vec<(TagField, String)>,

    /// Input files.
    #[command(flatten)]
    pub input: InputArgs,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,

    /// Write options.
    #[command(flatten)]
    pub write: WriteOptions,
}

/// Fields `idk merge` can merge.
#[derive(Subcommand)]
pub enum MergeTarget {
    /// Replace listed genres (TCON) with a single genre.
    Genres(MergeArgs),
    /// Replace listed artists (TPE1) with a single artist.
    Artists(MergeArgs),
}

impl MergeTarget {
    /// The field merged, its `tags.merge` config key, and the arguments.
    pub fn into_parts(self) -> (TagField, &'static str, MergeArgs) {
        match self {
            MergeTarget::Genres(args) => (TagField::Genre, "genres", args),
            MergeTarget::Artists(args) => (TagField::Artist, "artists", args),
        }
    }
}

/// Arguments for `idk merge genres` and `idk merge artists`.
#[derive(Args)]
pub struct MergeArgs {
    /// Comma-separated values to replace (repeatable); case-insensitive, and "" matches a missing or empty value.
    #[arg(
        long,
        value_name = "VALUES",
        value_delimiter = ',',
        action = ArgAction::Append,
        required_unless_present = "config",
        conflicts_with = "config"
    )]
    pub from: Vec<String>,

    /// Value written in their place.
    #[arg(
        long,
        value_name = "VALUE",
        value_parser = non_blank,
        required_unless_present = "config",
        conflicts_with = "config"
    )]
    pub to: Option<String>,

    /// Read `--from` / `--to` from `tags.merge.<field>` in a YAML file [default: ./idk.yaml].
    #[arg(
        long,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = DEFAULT_CONFIG
    )]
    pub config: Option<PathBuf>,

    /// Input files.
    #[command(flatten)]
    pub input: InputArgs,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,

    /// Write options.
    #[command(flatten)]
    pub write: WriteOptions,
}

/// Things `idk copy` can copy.
#[derive(Subcommand)]
pub enum CopyTarget {
    /// Copy one tag's value into another tag, overwriting the destination.
    Tags(CopyTagsArgs),
}

/// Arguments for `idk copy tags`.
#[derive(Args)]
pub struct CopyTagsArgs {
    /// Tag to read the value from.
    #[arg(
        long,
        value_parser = TagFieldParser,
        required_unless_present = "config",
        conflicts_with = "config"
    )]
    pub from: Option<TagField>,

    /// Tag to write the value to.
    #[arg(
        long,
        value_parser = TagFieldParser,
        required_unless_present = "config",
        conflicts_with = "config"
    )]
    pub to: Option<TagField>,

    /// Read `--from` / `--to` from `tags.copy` in a YAML file [default: ./idk.yaml].
    #[arg(
        long,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = DEFAULT_CONFIG
    )]
    pub config: Option<PathBuf>,

    /// Treat files with an empty source tag as failures instead of skipping them.
    #[arg(long)]
    pub fail_on_empty: bool,

    /// Input files.
    #[command(flatten)]
    pub input: InputArgs,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,

    /// Write options.
    #[command(flatten)]
    pub write: WriteOptions,
}

/// Arguments for `idk show`.
#[derive(Args)]
pub struct ShowArgs {
    /// Print a single JSON array instead of readable text.
    #[arg(long)]
    pub json: bool,

    /// Only show this field (repeatable).
    #[arg(long = "field", value_name = "FIELD", value_parser = TagFieldParser)]
    pub fields: Vec<TagField>,

    /// Also show frames hidden by default: PRIV (private data), TCOP (copyright), TSSE (encoder settings).
    #[arg(short, long)]
    pub verbose: bool,

    /// Input files.
    #[command(flatten)]
    pub input: InputArgs,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,
}

/// Options shared by operations that modify files.
#[derive(Args)]
pub struct WriteOptions {
    /// Show what would change without writing any file.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

/// Files an operation reads, shared by every file-processing command.
#[derive(Args, Clone)]
pub struct InputArgs {
    /// MP3 files, directories (with -r) or glob patterns such as "*.mp3" or "**/*.mp3".
    #[arg(required_unless_present = "files_from", value_name = "FILES")]
    pub files: Vec<PathBuf>,

    /// Walk directories recursively, including every .mp3 file.
    #[arg(short, long, visible_alias = "recurse")]
    pub recursive: bool,

    /// Also read inputs from a file, one per line (or NUL-separated); "-" reads stdin.
    #[arg(long, value_name = "PATH")]
    pub files_from: Option<PathBuf>,
}

/// Rejects empty or whitespace-only values.
fn non_blank(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("must not be empty".to_owned())
    } else {
        Ok(value.to_owned())
    }
}

/// Exits with clap's usage-error formatting and exit code 2.
pub fn usage_error(kind: clap::error::ErrorKind, message: impl std::fmt::Display) -> ! {
    command().error(kind, message).exit()
}

/// Execution options shared by file-processing operations.
#[derive(Args)]
pub struct RunOptions {
    /// Maximum number of files processed concurrently [default: available CPUs].
    #[arg(short, long, value_name = "N")]
    pub jobs: Option<NonZeroUsize>,

    /// Hide the progress bar and summary; errors are still reported.
    #[arg(short, long)]
    pub quiet: bool,
}

impl RunOptions {
    /// Whether to draw the progress bar for a command whose results go to stdout:
    /// not with `--quiet`, and not when stdout is piped.
    pub fn progress_for_stdout(&self) -> bool {
        use std::io::IsTerminal;
        !self.quiet && std::io::stdout().is_terminal()
    }

    /// The effective concurrency limit.
    pub fn jobs(&self) -> usize {
        self.jobs
            .or_else(|| std::thread::available_parallelism().ok())
            .map_or(1, NonZeroUsize::get)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_groups_are_alphabetized() {
        for (heading, names) in COMMAND_GROUPS {
            assert!(names.is_sorted(), "{heading} group is not alphabetized");
        }
    }

    #[test]
    fn every_subcommand_is_in_exactly_one_help_group() {
        let mut grouped: Vec<&str> = COMMAND_GROUPS
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect();
        grouped.sort_unstable();
        let cmd = Cli::command();
        let mut defined: Vec<&str> = cmd.get_subcommands().map(clap::Command::get_name).collect();
        defined.sort_unstable();
        assert_eq!(grouped, defined);
    }
}
