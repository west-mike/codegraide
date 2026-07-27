use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use codegraide_core::inventory_repository;
#[derive(Debug, Parser)]
#[command(
    name = "codegraide",
    version,
    about = "Tool to analyze a code repository"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List the contents and languages found in a repository
    Inventory {
        /// Path to the repository
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();

    match &args.command {
        Command::Inventory { path } => run_inventory(path),
    }
}

fn run_inventory(path: &Path) -> ExitCode {
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("error: cannot access {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };

    if !metadata.is_dir() {
        eprintln!("error: {} is not a directory", path.display());
        return ExitCode::FAILURE;
    }

    let inventory = match inventory_repository(path) {
        Ok(inventory) => inventory,
        Err(error) => {
            eprintln!("error: failed to inventory {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };

    println!("Repository:  {}", path.display());
    println!("Total files: {}", inventory.total_files);
    println!("Ignored directories: {}", inventory.num_ignored_directories);
    println!(
        "Recognized source files: {}",
        inventory.recognized_source_files()
    );
    println!(
        "Unclassified files:      {}",
        inventory.unclassified_files()
    );
    println!("Code:");

    for (language, count) in &inventory.files_by_language {
        println!("  {:<12} {count}", language.as_str());
    }

    println!("\nUnclassified extensions:");

    for (extension, count) in &inventory.unclassified_files_by_extension {
        println!("  {:<16} {count}", extension.as_str());
    }

    ExitCode::SUCCESS
}
