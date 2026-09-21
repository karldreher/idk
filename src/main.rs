mod cli;
mod config;
mod copy;
mod merge;
mod outcome;
mod runner;
mod show;
mod tag_field;

use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use cli::{Cli, Command, CopyTagsArgs, CopyTarget, MergeArgs, MergeTarget};
use config::{Config, ConfigError};
use tag_field::TagField;

/// Exit code for invalid usage, matching clap's.
const USAGE_ERROR: u8 = 2;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Copy {
            target: CopyTarget::Tags(args),
        } => {
            let (from, to) = match copy_fields(&args).await {
                Ok(fields) => fields,
                Err(err) => return config_failure(&err),
            };
            if from == to {
                Cli::command()
                    .error(
                        ErrorKind::ArgumentConflict,
                        "--from and --to must name different tags",
                    )
                    .exit();
            }
            copy::run(args, from, to).await
        }
        Command::Merge { target } => {
            let (field, key, args) = match target {
                MergeTarget::Genres(args) => (TagField::Genre, "genres", args),
                MergeTarget::Artists(args) => (TagField::Artist, "artists", args),
            };
            let rule = match merge_rule(&args, key).await {
                Ok(rule) => rule,
                Err(MergeRuleError::Usage(message)) => {
                    eprintln!("error: {message}");
                    return ExitCode::from(USAGE_ERROR);
                }
                Err(MergeRuleError::Config(err)) => return config_failure(&err),
            };
            merge::run(args, field, rule).await
        }
        Command::Show(args) => show::run(args).await,
    }
}

/// Prints every config error and returns exit code 1.
fn config_failure(err: &ConfigError) -> ExitCode {
    for line in err.to_string().lines() {
        eprintln!("error: {line}");
    }
    ExitCode::FAILURE
}

/// The fields `copy tags` copies between: from the command line, or from `tags.copy` in `--config`.
async fn copy_fields(args: &CopyTagsArgs) -> Result<(TagField, TagField), ConfigError> {
    let Some(path) = &args.config else {
        let from = args
            .from
            .clone()
            .expect("clap requires --from without --config");
        let to = args
            .to
            .clone()
            .expect("clap requires --to without --config");
        return Ok((from, to));
    };
    let config = Config::load(path).await?;
    let copy = config
        .copy()
        .map_err(|message| ConfigError::new(path, message))?;
    if copy.from == copy.to {
        return Err(ConfigError::new(
            path,
            "tags.copy: from and to must name different tags",
        ));
    }
    Ok((copy.from.clone(), copy.to.clone()))
}

/// Why a merge rule could not be built.
enum MergeRuleError {
    /// Invalid command-line arguments (exit 2).
    Usage(String),
    /// Invalid or incomplete config file (exit 1).
    Config(ConfigError),
}

/// The merge rule: from the command line, or from `tags.merge.<key>` in `--config`.
async fn merge_rule(args: &MergeArgs, key: &str) -> Result<merge::Rule, MergeRuleError> {
    let Some(path) = &args.config else {
        let to = args
            .to
            .as_deref()
            .expect("clap requires --to without --config");
        if to.trim().is_empty() {
            return Err(MergeRuleError::Usage("--to must not be empty".to_owned()));
        }
        return Ok(merge::Rule::new(&args.from, to));
    };
    let config = Config::load(path).await.map_err(MergeRuleError::Config)?;
    let spec = config
        .merge(key)
        .map_err(|message| MergeRuleError::Config(ConfigError::new(path, message)))?;
    Ok(merge::Rule::new(&spec.from, &spec.to))
}
