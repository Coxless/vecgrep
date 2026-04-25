mod chunk;
mod cli;
mod walk;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, ModelCmd};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        None => {
            if cli.search.query.is_none() {
                eprintln!("vsgrep: missing QUERY. Run `vsgrep --help` for usage.");
                return ExitCode::from(2);
            }
            not_implemented("search")
        }
        Some(Command::Index(_)) => not_implemented("index"),
        Some(Command::Status(_)) => not_implemented("status"),
        Some(Command::Clean) => not_implemented("clean"),
        Some(Command::Model { action }) => match action {
            ModelCmd::List => not_implemented("model list"),
            ModelCmd::Use { .. } => not_implemented("model use"),
            ModelCmd::Add { .. } => not_implemented("model add"),
        },
    }
}

fn not_implemented(what: &str) -> ExitCode {
    eprintln!("vsgrep: `{what}` is not implemented yet");
    ExitCode::from(2)
}
