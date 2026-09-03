use std::path::PathBuf;

use clap::{Parser, Subcommand};
use stm32_devtools_package_lab::commands::{
    inspect_manifest, plan_manifest, repack_command, validate_manifest,
};

#[derive(Debug, Parser)]
#[command(
    name = "stm32pkg",
    version,
    about = "Validate package manifests and build deterministic product distributions"
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
    /// Merge a product overlay onto a base distribution and create a deterministic tar.gz.
    Repack {
        /// Existing distribution directory to use as the base layer.
        #[arg(long)]
        base: PathBuf,
        /// Product-specific overlay directory. Files replace matching base paths.
        #[arg(long)]
        overlay: PathBuf,
        /// YAML or JSON release metadata and executable-path configuration.
        #[arg(long)]
        config: PathBuf,
        /// Destination tar.gz archive.
        #[arg(short, long)]
        output: PathBuf,
        /// Sidecar manifest path; defaults to <output>.manifest.json.
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Replace an existing archive and sidecar manifest.
        #[arg(long)]
        force: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Validate { manifest } => validate_manifest(&manifest),
        Command::Plan { manifest } => plan_manifest(&manifest),
        Command::Inspect { manifest } => inspect_manifest(&manifest),
        Command::Repack {
            base,
            overlay,
            config,
            output,
            manifest,
            force,
        } => repack_command(
            &base,
            &overlay,
            &config,
            &output,
            manifest.as_deref(),
            force,
        ),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
