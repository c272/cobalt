use clap::Parser;
use cli::Command;

mod cli;
mod commands;
mod compiler;
mod config;
mod linker;

#[cfg(test)]
mod tests;

fn main() -> miette::Result<()> {
    let cli = cli::Cli::parse();
    match cli.command() {
        Command::Build(args) => commands::build(args)?,
    }

    Ok(())
}
