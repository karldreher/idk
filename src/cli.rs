//! Command-line argument definitions.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::tag_field::TagField;

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
    #[arg(long, value_enum, ignore_case = true)]
    pub from: TagField,

    /// Tag to write the value to.
    #[arg(long, value_enum, ignore_case = true)]
    pub to: TagField,

    /// Treat files with an empty source tag as failures instead of skipping them.
    #[arg(long)]
    pub fail_on_empty: bool,

    /// MP3 files to update.
    #[arg(required = true, value_name = "FILES")]
    pub files: Vec<PathBuf>,

    /// Execution options.
    #[command(flatten)]
    pub run: RunOptions,
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
    /// The effective concurrency limit.
    pub fn jobs(&self) -> usize {
        self.jobs
            .or_else(|| std::thread::available_parallelism().ok())
            .map_or(1, NonZeroUsize::get)
    }
}
