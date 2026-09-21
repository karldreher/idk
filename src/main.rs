mod cli;
mod copy;
mod outcome;
mod runner;
mod show;
mod tag_field;

use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use cli::{Cli, Command, CopyTarget};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Copy {
            target: CopyTarget::Tags(args),
        } => {
            if args.from == args.to {
                Cli::command()
                    .error(
                        ErrorKind::ArgumentConflict,
                        "--from and --to must name different tags",
                    )
                    .exit();
            }
            copy::run(args).await
        }
        Command::Show(args) => show::run(args).await,
    }
}
