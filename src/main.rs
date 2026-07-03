use std::path::PathBuf;

use clap::{Parser, Subcommand};
use stm32_devtools_package_lab::commands::{inspect_manifest, plan_manifest, validate_manifest};

#[derive(Debug, Parser)]
#[command(
    name = "stm32pkg",
    version,
    about = "Validate and resolve STM32-style developer-tool package manifests"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a package manifest and print a concise status message.
    Validate { manifest: PathBuf },
    /// Resolve package dependencies and print a JSON installation plan.
    Plan { manifest: PathBuf },
    /// Print normalized manifest metadata as JSON.
    Inspect { manifest: PathBuf },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Validate { manifest } => validate_manifest(&manifest),
        Command::Plan { manifest } => plan_manifest(&manifest),
        Command::Inspect { manifest } => inspect_manifest(&manifest),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
