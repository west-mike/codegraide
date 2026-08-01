use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use codegraide_core::{
    FileCategory, InventoryJsonReport, InventoryOptions, RepositoryInventory,
    inventory_repository_with_options,
};

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
    /// Inventory the files and languages found in a repository
    Inventory {
        /// Path to the repository
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

        /// Include files matching a repository-relative glob even when Git ignores them
        #[arg(long, value_name = "GLOB", action = clap::ArgAction::Append)]
        include_ignored: Vec<String>,

        /// Layer an explicit JSON ruleset over codegraide's embedded defaults
        #[arg(long, value_name = "PATH")]
        config: Option<PathBuf>,

        /// Enumerate Git-ignored files and directories for exact counts and paths
        #[arg(long)]
        audit_ignored: bool,

        /// Suppress nonfatal configuration and categorization warnings
        #[arg(long)]
        no_warnings: bool,

        /// Print repository-relative paths for a category; may be repeated
        #[arg(long, value_name = "CATEGORY", action = clap::ArgAction::Append)]
        list_files: Vec<FileListSelection>,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Terminal)]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Terminal,
    Json,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, ValueEnum)]
enum FileListSelection {
    Source,
    Documentation,
    Configuration,
    Data,
    Assets,
    Uncategorized,
    All,
}

impl FileListSelection {
    fn category(self) -> Option<FileCategory> {
        match self {
            Self::Source => Some(FileCategory::Source),
            Self::Documentation => Some(FileCategory::Documentation),
            Self::Configuration => Some(FileCategory::Configuration),
            Self::Data => Some(FileCategory::Data),
            Self::Assets => Some(FileCategory::Assets),
            Self::Uncategorized => Some(FileCategory::Uncategorized),
            Self::All => None,
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();

    match &args.command {
        Command::Inventory {
            path,
            include_ignored,
            config,
            audit_ignored,
            no_warnings,
            list_files,
            format,
        } => run_inventory(
            path,
            include_ignored,
            config.as_deref(),
            *audit_ignored,
            *no_warnings,
            list_files,
            *format,
        ),
    }
}

fn run_inventory(
    path: &Path,
    include_ignored: &[String],
    config_path: Option<&Path>,
    audit_ignored: bool,
    no_warnings: bool,
    list_files: &[FileListSelection],
    format: OutputFormat,
) -> ExitCode {
    if format == OutputFormat::Json && !list_files.is_empty() {
        eprintln!("error: --list-files cannot be combined with --format json");
        return ExitCode::FAILURE;
    }

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
        config_path: config_path.map(Path::to_path_buf),
        audit_ignored,
        emit_warnings: !no_warnings,
    };
    let inventory = match inventory_repository_with_options(path, &options) {
        Ok(inventory) => inventory,
        Err(error) => {
            eprintln!("error: failed to inventory {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };

    if format == OutputFormat::Json {
        return print_json(&inventory);
    }

    for diagnostic in &inventory.diagnostics {
        eprintln!("warning[{}]: {}", diagnostic.code, diagnostic.message);
    }

    print_summary(path, &inventory);
    print_requested_files(&inventory, list_files);

    if audit_ignored {
        print_ignored_paths(&inventory);
    }

    ExitCode::SUCCESS
}

fn print_json(inventory: &RepositoryInventory) -> ExitCode {
    let report = InventoryJsonReport::from_inventory(inventory);
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: could not serialize inventory report: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_summary(path: &Path, inventory: &RepositoryInventory) {
    println!("Repository: {}", path.display());
    println!("Inventoried files: {}", inventory.inventoried_files);
    println!("Source files: {}", inventory.source_files());
    println!(
        "Included ignored files: {}",
        inventory.num_included_ignored_files
    );

    println!("\nCategories:");
    for category in FileCategory::ALL {
        println!(
            "  {:<16} {}",
            category.as_str(),
            inventory.category_count(category)
        );
    }

    println!("\nRecognized languages:");
    if inventory.files_by_language.is_empty() {
        println!("  (none)");
    } else {
        for (language, count) in &inventory.files_by_language {
            println!("  {:<16} {count}", language.as_str());
        }
    }

    println!("\nPhysical line counts:");
    println!("  Files analyzed: {}", inventory.line_counts.total.files);
    println!("  Total lines: {}", inventory.line_counts.total.total);
    println!("  Source lines: {}", inventory.line_counts.total.source);
    println!("  Comment lines: {}", inventory.line_counts.total.comment);
    println!("  Blank lines: {}", inventory.line_counts.total.blank);
    if !inventory.line_counts.by_language.is_empty() {
        println!("  By language:");
        for (language, counts) in &inventory.line_counts.by_language {
            println!(
                "    {:<12} files={} source={} comment={} blank={}",
                language.as_str(),
                counts.files,
                counts.source,
                counts.comment,
                counts.blank
            );
        }
    }

    println!("\nUncategorized extensions:");
    if inventory.uncategorized_files_by_extension.is_empty() {
        println!("  (none)");
    } else {
        for (extension, count) in &inventory.uncategorized_files_by_extension {
            println!("  {:<16} {count}", extension.as_str());
        }
    }

    println!("\nIgnored entries:");
    if inventory.ignored.exact {
        println!("  Files: {}", inventory.ignored.file_count());
        println!("  Directories: {}", inventory.ignored.directory_count());
    } else {
        println!("  Files observed: {}", inventory.ignored.file_count());
        println!(
            "  Directories pruned: {}",
            inventory.ignored.directory_count()
        );
    }
    println!(
        "  Built-in safety directories: {}",
        inventory.ignored.builtin_directory_count()
    );
    if inventory.ignored.exact {
        println!("  Audit: exact except for built-in safety directories");
    } else {
        println!(
            "  Note: contents of ignored directories were not enumerated; use --audit-ignored for exact Git-ignored counts and paths"
        );
    }
}

fn print_requested_files(inventory: &RepositoryInventory, selections: &[FileListSelection]) {
    if selections.is_empty() {
        return;
    }

    let list_all = selections.contains(&FileListSelection::All);
    let categories = if list_all {
        FileCategory::ALL.into_iter().collect::<BTreeSet<_>>()
    } else {
        selections
            .iter()
            .filter_map(|selection| selection.category())
            .collect::<BTreeSet<_>>()
    };

    println!("\nSelected files:");
    for category in categories {
        println!("  {}:", category.as_str());
        print_paths(inventory.category_files(category), 4);
    }
}

fn print_ignored_paths(inventory: &RepositoryInventory) {
    println!("\nIgnored audit paths:");
    println!("  files:");
    print_paths(&inventory.ignored.files, 4);
    println!("  directories:");
    print_paths(&inventory.ignored.directories, 4);
    println!("  built-in safety directories:");
    print_paths(&inventory.ignored.builtin_directories, 4);
}

fn print_paths(paths: &[PathBuf], indentation: usize) {
    if paths.is_empty() {
        println!("{:indentation$}(none)", "");
        return;
    }

    for path in paths {
        let normalized = path.to_string_lossy().replace('\\', "/");
        println!("{:indentation$}{normalized}", "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repeatable_inventory_options() {
        let args = Args::try_parse_from([
            "codegraide",
            "inventory",
            "example",
            "--include-ignored",
            "generated/**",
            "--include-ignored",
            "vendor/**",
            "--config",
            "full.json",
            "--audit-ignored",
            "--no-warnings",
            "--list-files",
            "source",
            "--list-files",
            "uncategorized",
        ])
        .expect("arguments should parse");

        let Command::Inventory {
            path,
            include_ignored,
            config,
            audit_ignored,
            no_warnings,
            list_files,
            format,
        } = args.command;

        assert_eq!(path, PathBuf::from("example"));
        assert_eq!(include_ignored, ["generated/**", "vendor/**"]);
        assert_eq!(config, Some(PathBuf::from("full.json")));
        assert!(audit_ignored);
        assert!(no_warnings);
        assert_eq!(
            list_files,
            [FileListSelection::Source, FileListSelection::Uncategorized]
        );
        assert_eq!(format, OutputFormat::Terminal);
    }
}
