use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use codegraide_core::{InventoryOptions, inventory_repository_with_options};
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

        /// Include files matching a repository-relative glob even when Git ignores them
        #[arg(long, value_name = "GLOB", action = clap::ArgAction::Append)]
        include_ignored: Vec<String>,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();

    match &args.command {
        Command::Inventory {
            path,
            include_ignored,
        } => run_inventory(path, include_ignored),
    }
}

fn run_inventory(path: &Path, include_ignored: &[String]) -> ExitCode {
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

    let options = InventoryOptions {
        include_ignored: include_ignored.to_vec(),
    };
    let inventory = match inventory_repository_with_options(path, &options) {
        Ok(inventory) => inventory,
        Err(error) => {
            eprintln!("error: failed to inventory {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };

    println!("Repository:  {}", path.display());
    println!("Total files: {}", inventory.total_files);
    println!(
        "Built-in ignored directories: {}",
        inventory.num_builtin_ignored_directories
    );
    println!(
        "Included ignored files: {}",
        inventory.num_included_ignored_files
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_include_ignored_patterns() {
        let args = Args::try_parse_from([
            "codegraide",
            "inventory",
            "example",
            "--include-ignored",
            "generated/**",
            "--include-ignored",
            "vendor/**",
        ])
        .expect("arguments should parse");

        let Command::Inventory {
            path,
            include_ignored,
        } = args.command;

        assert_eq!(path, PathBuf::from("example"));
        assert_eq!(include_ignored, ["generated/**", "vendor/**"]);
    }
}
