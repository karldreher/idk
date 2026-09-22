mod apply;
mod cli;
mod config;
mod copy;
mod edit;
mod find;
mod input;
mod merge;
mod outcome;
mod runner;
mod schema;
mod show;
mod tag_field;
#[cfg(test)]
mod test_support;

use std::process::ExitCode;

use clap::Parser;

use cli::{ClearTarget, Cli, Command, CopyTarget, SchemaAction, SetTarget};

#[tokio::main]
async fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Apply(args) => apply::run(args).await,
        Command::Clear {
            target: ClearTarget::Tags(args),
        } => edit::run_clear(args).await,
        Command::Copy {
            target: CopyTarget::Tags(args),
        } => copy::run(args).await,
        Command::Find(args) => find::run(args).await,
        Command::Merge { target } => merge::run(target).await,
        Command::Schema {
            action: SchemaAction::Validate { config },
        } => schema::validate(&config).await,
        Command::Schema {
            action: SchemaAction::Write { file },
        } => schema::write(&file).await,
        Command::Set {
            target: SetTarget::Tags(args),
        } => edit::run_set(args).await,
        Command::Show(args) => show::run(args).await,
    }
}
