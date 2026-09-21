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

/// Exit code for invalid usage or configuration, matching clap's.
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
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::from(USAGE_ERROR);
                }
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
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::from(USAGE_ERROR);
                }
            };
            merge::run(args, field, rule).await
        }
        Command::Show(args) => show::run(args).await,
    }
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
    Ok((copy.from.clone(), copy.to.clone()))
}

/// The merge rule: from the command line, or from `tags.merge.<key>` in `--config`.
async fn merge_rule(args: &MergeArgs, key: &str) -> Result<merge::Rule, String> {
    let Some(path) = &args.config else {
        let to = args
            .to
            .as_deref()
            .expect("clap requires --to without --config");
        if to.trim().is_empty() {
            return Err("--to must not be empty".to_owned());
        }
        return Ok(merge::Rule::new(&args.from, to));
    };
    let config = Config::load(path).await.map_err(|err| err.to_string())?;
    let spec = config
        .merge(key)
        .map_err(|message| ConfigError::new(path, message).to_string())?;
    Ok(merge::Rule::new(&spec.from, &spec.to))
}
