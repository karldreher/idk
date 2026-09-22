//! Command-line argument definitions.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::find::{Condition, ConditionParser};
use crate::tag_field::{AssignmentParser, TagField, TagFieldParser};

/// Config file used when `--config` is given without a path.
pub const DEFAULT_CONFIG: &str = "idk.yaml";

/// Where `idk schema write` puts the schema by default.
pub const DEFAULT_SCHEMA: &str = "idk.yaml.json";

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
    /// Apply tag edits from `idk show --json` output.
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
pub struct ApplyArgs {
    /// JSON in the `idk show --json` format; "-" reads stdin.
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
    #[arg(short, long)]
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
    use clap::CommandFactory;
    Cli::command().error(kind, message).exit()
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
