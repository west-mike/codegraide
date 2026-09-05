use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use codegraide_analyzer_cpp::{
    CppDependencyResolver, CppResolutionOptions, apply_architecture_to_resolution,
    resolve_cpp_calls, resolve_cpp_dependencies,
};
use codegraide_analyzer_python::{
    PythonDependencyResolver, PythonEnvironmentSelection, PythonResolutionOptions,
    resolve_python_calls, resolve_python_dependencies,
};
use codegraide_core::{
    AnalysisJsonReport, AnalysisOptions, AnalyzerCapability, CallDirection, CallGraphAnalysis,
    CallGraphFilter, CallGraphView, CallJsonReport, DependencyBundleJsonReport,
    DependencyDirection, DependencyEnvironmentReport, DependencyGraphAnalysis,
    DependencyGraphFilter, DependencyGraphInputExclusions, DependencyGraphQuery,
    DependencyGraphQueryResult, DependencyGraphView, DependencyJsonReport,
    DependencyLanguageJsonReport, DependencyNode, DependencyNodeKind, DependencyQueryDirection,
    DependencyRelationKind, DependencyResolver, DependencyResolverContextReport,
    DependencyResolverRegistry, DependencyResolverReport, DocumentationJsonReport, FileCategory,
    GateJsonReport, InventoryJsonReport, InventoryOptions, LanguageId, MeasurementConcept,
    RepositoryAnalysis, RepositoryInventory, ReviewJsonReport, ReviewOptions, ReviewStatus,
    UnavailableDependencyLanguage, analyze_call_graph_with_modules, analyze_dependency_graph,
    analyze_repository, call_node_name, dependency_query_view, explain_dependency_cycles,
    filter_call_graph, filter_dependency_graph, inventory_repository_with_options,
    query_dependency_graph, render_call_dot, render_call_html, render_call_html_with_source,
    render_call_mermaid, render_dependency_dot, render_dependency_mermaid, review_status_code,
};

use crate::bootstrap::{BuiltinAnalyzerFeatures, build_builtin_analyzer_registry};

mod bootstrap;
mod review_context;

#[derive(Debug, Parser)]
#[command(
    name = "codegraide",
    version,
    about = "Tool to analyze a code repository",
    after_help = "Run codegraide <COMMAND> --help for all options, examples and configuration formats.\nUse -h for a short option list; --help includes the full reference."
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compare committed C++ functions and retrieve bounded review context
    #[command(after_help = "Use --help for examples and reference retrieval details.", after_long_help = include_str!("help/review-context.txt"))]
    ReviewContext(review_context::ReviewContextArgs),
    /// Inventory the files and languages found in a repository
    #[command(after_help = "Use --help for examples and configuration details.", after_long_help = include_str!("help/inventory.txt"))]
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

        /// Print category paths (source/documentation/configuration/data/assets/uncategorized/all); terminal-only, may repeat
        #[arg(long, value_name = "CATEGORY", action = clap::ArgAction::Append)]
        list_files: Vec<FileListSelection>,

        /// Output format
        #[arg(long, value_enum, default_value_t = InventoryOutputFormat::Terminal)]
        format: InventoryOutputFormat,
    },
    /// Parse supported source files and report syntax recovery diagnostics
    #[command(after_help = "Use --help for examples and configuration details.", after_long_help = concat!(include_str!("help/analyze.txt"), include_str!("help/policy.txt")))]
    Analyze {
        /// File or directory to analyze
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

        /// Select repository-relative paths with a full-match regular expression; may repeat
        #[arg(long = "match", value_name = "REGEX", action = clap::ArgAction::Append)]
        match_patterns: Vec<String>,

        /// Include files matching a repository-relative glob even when Git ignores them
        #[arg(long, value_name = "GLOB", action = clap::ArgAction::Append)]
        include_ignored: Vec<String>,

        /// Print all diagnostics, or diagnostics for one exact file; may repeat
        #[arg(
            long,
            value_name = "FILE",
            num_args = 0..=1,
            action = clap::ArgAction::Append,
            default_missing_value = "__ALL_DIAGNOSTICS__"
        )]
        diagnostics: Vec<String>,

        /// Print complete symbols, imports, and measurements, or details for one exact file
        #[arg(
            long,
            value_name = "FILE",
            num_args = 0..=1,
            action = clap::ArgAction::Append,
            default_missing_value = "__ALL_DETAILS__"
        )]
        details: Vec<String>,

        /// Load an explicit review policy file
        #[arg(long, value_name = "PATH")]
        policy: Option<PathBuf>,

        /// Require human review at this cyclomatic-complexity score
        #[arg(long, value_name = "SCORE")]
        complexity_review_at: Option<u64>,

        /// Block at this cyclomatic-complexity score
        #[arg(long, value_name = "SCORE")]
        complexity_block_at: Option<u64>,

        /// Disable the policy-file cyclomatic-complexity block threshold
        #[arg(long, conflicts_with = "complexity_block_at")]
        no_complexity_block: bool,

        /// Disable Python documentation coverage extraction and reporting
        #[arg(long, conflicts_with = "documentation_review_below")]
        no_documentation_coverage: bool,

        /// Include conventional Python test files in documentation coverage
        #[arg(long, conflicts_with = "no_documentation_coverage")]
        include_tests: bool,

        /// Require human review when Python documentation coverage is below this percentage
        #[arg(long, value_name = "PERCENT", value_parser = parse_percentage)]
        documentation_review_below: Option<u8>,

        /// Exit 0 for pass, 2 for review required, 3 for blocked (errors use 1)
        #[arg(long)]
        gate: bool,

        /// JSON profile; review requires --format json, full preserves all facts
        #[arg(long, value_enum, default_value_t = ReportProfile::Full)]
        profile: ReportProfile,

        /// Limit rankings and findings in compact output profiles
        #[arg(long, value_name = "COUNT", value_parser = parse_positive_usize)]
        top: Option<usize>,

        /// Output format
        #[arg(long, value_enum, default_value_t = AnalyzeOutputFormat::Terminal)]
        format: AnalyzeOutputFormat,
    },
    /// Report Python module, class, function, and method docstring coverage
    #[command(after_help = "Use --help for examples and configuration details.", after_long_help = concat!(include_str!("help/comments.txt"), include_str!("help/policy.txt")))]
    Comments {
        /// File or directory to analyze
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

        /// Select repository-relative paths with a full-match regular expression; may repeat
        #[arg(long = "match", value_name = "REGEX", action = clap::ArgAction::Append)]
        match_patterns: Vec<String>,

        /// Include files matching a repository-relative glob even when Git ignores them
        #[arg(long, value_name = "GLOB", action = clap::ArgAction::Append)]
        include_ignored: Vec<String>,

        /// Load an explicit review policy file
        #[arg(long, value_name = "PATH")]
        policy: Option<PathBuf>,

        /// Require human review when Python documentation coverage is below this percentage
        #[arg(long, value_name = "PERCENT", value_parser = parse_percentage)]
        documentation_review_below: Option<u8>,

        /// Include conventional Python test files in documentation coverage
        #[arg(long)]
        include_tests: bool,

        /// Limit missing and unavailable symbol details
        #[arg(long, value_name = "COUNT", value_parser = parse_positive_usize)]
        top: Option<usize>,

        /// Return 0 when the configured threshold passes and 2 when review is required
        #[arg(long)]
        gate: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t = CommentsOutputFormat::Terminal)]
        format: CommentsOutputFormat,
    },
    /// Resolve supported language dependencies and build language-pure graphs
    #[command(after_help = "Use --help for examples and configuration details.", after_long_help = include_str!("help/dependencies.txt"))]
    Dependencies {
        /// Project directory to analyze
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

        /// Analyze only this dependency language; may repeat
        #[arg(long, value_name = "LANGUAGE", action = clap::ArgAction::Append)]
        language: Vec<String>,

        /// C++ compilation database used for header search paths
        #[arg(long, value_name = "FILE")]
        compile_commands: Option<PathBuf>,

        /// Python interpreter used for standard-library and installed-package resolution
        #[arg(long, value_name = "EXECUTABLE", conflicts_with = "venv")]
        python: Option<PathBuf>,

        /// Virtual-environment directory used for installed-package resolution
        #[arg(long, value_name = "DIRECTORY", conflicts_with = "python")]
        venv: Option<PathBuf>,

        /// Focus the view on one local dependency unit; may repeat
        #[arg(
            long,
            value_name = "UNIT",
            action = clap::ArgAction::Append,
            conflicts_with_all = ["path_from", "path_to", "closure"]
        )]
        focus: Vec<String>,

        /// Traverse dependencies, dependents, or both from a focus or closure root
        #[arg(long, value_enum, conflicts_with_all = ["path_from", "path_to"])]
        direction: Option<DependencyDirectionArgument>,

        /// Traversal depth from focused units; zero shows only the focus and its cycle
        #[arg(long, value_name = "N", requires = "focus")]
        depth: Option<usize>,

        /// Find the shortest exact local dependency path from this unit
        #[arg(
            long,
            value_name = "UNIT",
            requires = "path_to",
            conflicts_with_all = ["focus", "depth", "cycles_only", "closure", "direction"]
        )]
        path_from: Option<String>,

        /// Find the shortest exact local dependency path to this unit
        #[arg(
            long,
            value_name = "UNIT",
            requires = "path_from",
            conflicts_with_all = ["focus", "depth", "cycles_only", "closure", "direction"]
        )]
        path_to: Option<String>,

        /// Show the complete exact local dependency or dependent closure
        #[arg(
            long,
            value_name = "UNIT",
            conflicts_with_all = ["focus", "depth", "cycles_only", "path_from", "path_to"]
        )]
        closure: Option<String>,

        /// Keep only exact dependency relations; remove inferred and uncertain relations
        #[arg(long)]
        exact_only: bool,

        /// Show repository-local units and relations, including inferred relations
        #[arg(long)]
        local_only: bool,

        /// Show only cyclic local strongly connected components
        #[arg(long)]
        cycles_only: bool,

        /// Exclude type-checking-only imports before graph construction
        #[arg(long)]
        exclude_type_only: bool,

        /// Exclude imports guarded by ImportError handling before graph construction
        #[arg(long)]
        exclude_optional: bool,

        /// Exclude imports inside functions, methods, or lambdas before graph construction
        #[arg(long)]
        exclude_callable_local: bool,

        /// Exclude conditionally executed imports before graph construction
        #[arg(long)]
        exclude_conditional: bool,

        /// Limit terminal fan-in and fan-out rankings
        #[arg(long, value_name = "COUNT", value_parser = parse_positive_usize)]
        top: Option<usize>,

        /// Output format
        #[arg(long, value_enum, default_value_t = DependencyOutputFormat::Terminal)]
        format: DependencyOutputFormat,

        /// Write an HTML bundle directory (default: codegraide-dependencies); requires --format html
        #[arg(long, value_name = "DIRECTORY")]
        output: Option<PathBuf>,

        /// Open the interactive HTML graph in the default browser
        #[arg(long)]
        open: bool,
    },
    /// Explore Python or C++ symbols and conservative written-call targets
    #[command(after_help = "Use --help for examples and configuration details.", after_long_help = include_str!("help/calls.txt"))]
    Calls {
        /// Project directory to analyze
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

        /// Analyze one call language; required when Python and C++ are both present
        #[arg(long, value_enum)]
        language: Option<CallLanguageArgument>,

        /// C++ compilation database used only for include visibility metadata
        #[arg(long, value_name = "FILE")]
        compile_commands: Option<PathBuf>,

        /// C++ architectural-group definition file
        #[arg(long, value_name = "FILE")]
        architecture: Option<PathBuf>,

        /// Python interpreter used for import-boundary resolution
        #[arg(long, value_name = "EXECUTABLE", conflicts_with = "venv")]
        python: Option<PathBuf>,

        /// Virtual-environment directory used for import-boundary resolution
        #[arg(long, value_name = "DIRECTORY", conflicts_with = "python")]
        venv: Option<PathBuf>,

        /// Focus on one symbol selector such as shop.service::Client.send; may repeat
        #[arg(long, value_name = "SYMBOL", action = clap::ArgAction::Append)]
        focus: Vec<String>,

        /// Traverse callers, callees, or both from focused symbols
        #[arg(long, value_enum, requires = "focus")]
        direction: Option<CallDirectionArgument>,

        /// Traversal depth from focused symbols
        #[arg(long, value_name = "N", requires = "focus")]
        depth: Option<usize>,

        /// Keep only exact call relations; remove inferred and other uncertain relations
        #[arg(long)]
        exact_only: bool,

        /// Show project symbols and local calls, including inferred calls
        #[arg(long)]
        local_only: bool,

        /// Show only recursive local strongly connected components
        #[arg(long)]
        cycles_only: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t = CallOutputFormat::Terminal)]
        format: CallOutputFormat,

        /// Write HTML to this file (default: codegraide-call-graph.html); requires --format html
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Open HTML output in the default browser
        #[arg(long)]
        open: bool,

        /// Embed function bodies and call-site context in HTML output
        #[arg(long)]
        include_source: bool,

        /// Nested expansion depth, 1-10 (default: 3); requires --include-source and HTML
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=10))]
        max_expansion_depth: Option<u8>,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum CallLanguageArgument {
    Python,
    Cpp,
}

impl CallLanguageArgument {
    fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Cpp => "cpp",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum InventoryOutputFormat {
    Terminal,
    Json,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum AnalyzeOutputFormat {
    Terminal,
    Json,
    Gate,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum CommentsOutputFormat {
    Terminal,
    Json,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum DependencyOutputFormat {
    Terminal,
    Json,
    Mermaid,
    Dot,
    Html,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum CallOutputFormat {
    Terminal,
    Json,
    Html,
    Mermaid,
    Dot,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum DependencyDirectionArgument {
    Dependencies,
    Dependents,
    Both,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum CallDirectionArgument {
    Callers,
    Callees,
    Both,
}

impl From<CallDirectionArgument> for CallDirection {
    fn from(value: CallDirectionArgument) -> Self {
        match value {
            CallDirectionArgument::Callers => Self::Callers,
            CallDirectionArgument::Callees => Self::Callees,
            CallDirectionArgument::Both => Self::Both,
        }
    }
}

impl From<DependencyDirectionArgument> for DependencyDirection {
    fn from(value: DependencyDirectionArgument) -> Self {
        match value {
            DependencyDirectionArgument::Dependencies => Self::Dependencies,
            DependencyDirectionArgument::Dependents => Self::Dependents,
            DependencyDirectionArgument::Both => Self::Both,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
enum ReportProfile {
    Full,
    Review,
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

fn parse_positive_usize(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| "COUNT must be a positive integer".to_owned())?;
    if parsed == 0 {
        return Err("COUNT must be a positive integer".to_owned());
    }
    Ok(parsed)
}

fn parse_percentage(value: &str) -> Result<u8, String> {
    let parsed = value
        .parse::<u8>()
        .map_err(|_| "PERCENT must be an integer between 1 and 100".to_owned())?;
    if !(1..=100).contains(&parsed) {
        return Err("PERCENT must be an integer between 1 and 100".to_owned());
    }
    Ok(parsed)
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
        Command::Analyze {
            path,
            match_patterns,
            include_ignored,
            diagnostics,
            details,
            policy,
            complexity_review_at,
            complexity_block_at,
            no_complexity_block,
            no_documentation_coverage,
            include_tests,
            documentation_review_below,
            gate,
            profile,
            top,
            format,
        } => run_analyze(&AnalyzeRequest {
            path,
            match_patterns,
            include_ignored,
            diagnostics,
            details,
            policy: policy.as_deref(),
            complexity_review_at: *complexity_review_at,
            complexity_block_at: *complexity_block_at,
            no_complexity_block: *no_complexity_block,
            no_documentation_coverage: *no_documentation_coverage,
            include_tests: *include_tests,
            documentation_review_below: *documentation_review_below,
            gate: *gate,
            profile: *profile,
            top: *top,
            format: *format,
        }),
        Command::Comments {
            path,
            match_patterns,
            include_ignored,
            policy,
            documentation_review_below,
            include_tests,
            top,
            gate,
            format,
        } => run_comments(&CommentsRequest {
            path,
            match_patterns,
            include_ignored,
            policy: policy.as_deref(),
            documentation_review_below: *documentation_review_below,
            include_tests: *include_tests,
            top: *top,
            gate: *gate,
            format: *format,
        }),
        Command::Dependencies {
            path,
            language,
            compile_commands,
            python,
            venv,
            focus,
            direction,
            depth,
            path_from,
            path_to,
            closure,
            exact_only,
            local_only,
            cycles_only,
            exclude_type_only,
            exclude_optional,
            exclude_callable_local,
            exclude_conditional,
            top,
            format,
            output,
            open,
        } => run_dependencies(&DependencyRequest {
            path,
            languages: language,
            compile_commands: compile_commands.as_deref(),
            python: python.as_deref(),
            venv: venv.as_deref(),
            focus,
            direction: *direction,
            depth: *depth,
            path_from: path_from.as_deref(),
            path_to: path_to.as_deref(),
            closure: closure.as_deref(),
            exact_only: *exact_only,
            local_only: *local_only,
            cycles_only: *cycles_only,
            exclusions: DependencyGraphInputExclusions {
                type_only: *exclude_type_only,
                optional: *exclude_optional,
                callable_local: *exclude_callable_local,
                conditional: *exclude_conditional,
            },
            top: *top,
            format: *format,
            output: output.as_deref(),
            open: *open,
        }),
        Command::ReviewContext(args) => review_context::run(args),
        Command::Calls {
            path,
            language,
            compile_commands,
            architecture,
            python,
            venv,
            focus,
            direction,
            depth,
            exact_only,
            local_only,
            cycles_only,
            format,
            output,
            open,
            include_source,
            max_expansion_depth,
        } => run_calls(&CallRequest {
            path,
            language: *language,
            compile_commands: compile_commands.as_deref(),
            architecture: architecture.as_deref(),
            python: python.as_deref(),
            venv: venv.as_deref(),
            focus,
            direction: *direction,
            depth: *depth,
            exact_only: *exact_only,
            local_only: *local_only,
            cycles_only: *cycles_only,
            format: *format,
            output: output.as_deref(),
            open: *open,
            include_source: *include_source,
            max_expansion_depth: *max_expansion_depth,
        }),
    }
}

fn run_inventory(
    path: &Path,
    include_ignored: &[String],
    config_path: Option<&Path>,
    audit_ignored: bool,
    no_warnings: bool,
    list_files: &[FileListSelection],
    format: InventoryOutputFormat,
) -> ExitCode {
    if format == InventoryOutputFormat::Json && !list_files.is_empty() {
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

    if format == InventoryOutputFormat::Json {
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

#[derive(Debug, Clone, Copy)]
struct DependencyRequest<'a> {
    path: &'a Path,
    languages: &'a [String],
    compile_commands: Option<&'a Path>,
    python: Option<&'a Path>,
    venv: Option<&'a Path>,
    focus: &'a [String],
    direction: Option<DependencyDirectionArgument>,
    depth: Option<usize>,
    path_from: Option<&'a str>,
    path_to: Option<&'a str>,
    closure: Option<&'a str>,
    exact_only: bool,
    local_only: bool,
    cycles_only: bool,
    exclusions: DependencyGraphInputExclusions,
    top: Option<usize>,
    format: DependencyOutputFormat,
    output: Option<&'a Path>,
    open: bool,
}

struct LanguageDependencyRun {
    language: String,
    resolver: DependencyResolverReport,
    environment: Option<DependencyEnvironmentReport>,
    summary_lines: Vec<String>,
    graph: DependencyGraphAnalysis,
    view: DependencyGraphView,
    query: Option<DependencyGraphQueryResult>,
    diagnostics: Vec<String>,
}

fn run_dependencies(request: &DependencyRequest<'_>) -> ExitCode {
    if request.output.is_some() && request.format != DependencyOutputFormat::Html {
        eprintln!("error: --output requires --format html");
        return ExitCode::FAILURE;
    }
    if request.open && request.format != DependencyOutputFormat::Html {
        eprintln!("error: --open requires --format html");
        return ExitCode::FAILURE;
    }
    if request.direction.is_some() && request.focus.is_empty() && request.closure.is_none() {
        eprintln!("error: --direction requires --focus or --closure");
        return ExitCode::FAILURE;
    }
    if request.closure.is_some() && request.direction == Some(DependencyDirectionArgument::Both) {
        eprintln!("error: --closure direction must be dependencies or dependents, not both");
        return ExitCode::FAILURE;
    }
    let python_environment = request
        .python
        .map(|path| PythonEnvironmentSelection::Interpreter(path.to_path_buf()))
        .or_else(|| {
            request
                .venv
                .map(|path| PythonEnvironmentSelection::VirtualEnvironment(path.to_path_buf()))
        });
    let mut dependency_registry = DependencyResolverRegistry::new();
    for resolver in [
        Box::new(CppDependencyResolver::new(CppResolutionOptions {
            compilation_database: request.compile_commands.map(Path::to_path_buf),
        })) as Box<dyn DependencyResolver>,
        Box::new(PythonDependencyResolver::new(PythonResolutionOptions {
            environment: python_environment,
        })) as Box<dyn DependencyResolver>,
    ] {
        if let Err(error) = dependency_registry.register(resolver) {
            eprintln!("error: could not register dependency resolver: {error}");
            return ExitCode::FAILURE;
        }
    }
    let supported = dependency_registry
        .languages()
        .map(|language| language.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let requested = request
        .languages
        .iter()
        .map(|language| language.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    if let Some(language) = requested
        .iter()
        .find(|language| !supported.contains(language.as_str()))
    {
        eprintln!(
            "error: dependency resolver {language:?} is not installed; installed resolvers: {}",
            supported.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        return ExitCode::FAILURE;
    }
    let metadata = match request.path.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!(
                "error: cannot access dependency project {}: {error}",
                request.path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    if !metadata.is_dir() {
        eprintln!(
            "error: dependency analysis requires a project directory: {}",
            request.path.display()
        );
        return ExitCode::FAILURE;
    }

    let mut registry = match build_builtin_analyzer_registry(BuiltinAnalyzerFeatures {
        documentation: false,
    }) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let analysis = match analyze_repository(
        &AnalysisOptions {
            target: request.path.to_path_buf(),
            ..AnalysisOptions::default()
        },
        &mut registry,
    ) {
        Ok(analysis) => analysis,
        Err(error) => {
            eprintln!(
                "error: failed to analyze dependency project {}: {error}",
                request.path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let present = analysis
        .analyzers
        .iter()
        .filter(|run| !run.files.is_empty())
        .map(|run| run.descriptor.language.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let selected = supported
        .iter()
        .filter(|language| {
            (requested.is_empty() && present.contains(*language)) || requested.contains(*language)
        })
        .cloned()
        .collect::<Vec<_>>();
    if request.compile_commands.is_some() && !selected.iter().any(|language| language == "cpp") {
        eprintln!("error: --compile-commands requires the C++ dependency resolver");
        return ExitCode::FAILURE;
    }
    if selected.len() > 1 && selectors_need_language(request) {
        eprintln!(
            "error: graph selectors in a multi-language run must use language:identity, such as python:shop.api or cpp:src/main.cpp"
        );
        return ExitCode::FAILURE;
    }

    let mut runs = Vec::new();
    for language in selected {
        let language_id = LanguageId::new(&language);
        let resolver = dependency_registry
            .get(&language_id)
            .expect("selected dependency resolver must be registered");
        let result = build_dependency_run(&analysis, request, resolver);
        match result {
            Ok(run) => runs.push(run),
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    runs.sort_by(|left, right| left.language.cmp(&right.language));
    for run in &runs {
        for diagnostic in &run.diagnostics {
            eprintln!("warning[{}]: {diagnostic}", run.language);
        }
        if matches!(
            request.format,
            DependencyOutputFormat::Mermaid | DependencyOutputFormat::Dot
        ) && run.view.nodes.len() > 200
        {
            eprintln!(
                "warning[{}]: graph contains {} nodes; use --language, --focus, --local-only, or --cycles-only for a more readable view",
                run.language,
                run.view.nodes.len()
            );
        }
    }
    let unavailable = analysis
        .inventory_languages
        .keys()
        .filter(|language| !supported.contains(language.as_str()))
        .map(|language| UnavailableDependencyLanguage {
            language: language.as_str().to_owned(),
            status: "resolver-not-installed",
            installation_hint: None,
        })
        .collect::<Vec<_>>();

    match request.format {
        DependencyOutputFormat::Terminal => {
            print_dependency_bundle_summary(request.path, &runs, &unavailable, request);
            ExitCode::SUCCESS
        }
        DependencyOutputFormat::Mermaid => {
            print!("{}", render_dependency_mermaid_bundle(&runs));
            ExitCode::SUCCESS
        }
        DependencyOutputFormat::Dot => {
            print!("{}", render_dependency_dot_bundle(&runs));
            ExitCode::SUCCESS
        }
        DependencyOutputFormat::Json => print_dependency_bundle_json(&runs, unavailable, request),
        DependencyOutputFormat::Html => emit_dependency_html_bundle(
            request.path,
            &runs,
            &unavailable,
            request.output,
            request.open,
        ),
    }
}

fn selectors_need_language(request: &DependencyRequest<'_>) -> bool {
    request
        .focus
        .iter()
        .map(String::as_str)
        .chain(request.path_from)
        .chain(request.path_to)
        .chain(request.closure)
        .any(|selector| !selector.contains(':'))
}

fn selector_for_language(selector: &str, language: &str) -> Option<String> {
    match selector.split_once(':') {
        Some((prefix, value)) => (prefix == language).then(|| value.to_owned()),
        None => Some(selector.to_owned()),
    }
}

fn graph_projection(
    language: &str,
    graph: &DependencyGraphAnalysis,
    request: &DependencyRequest<'_>,
) -> Result<(DependencyGraphView, Option<DependencyGraphQueryResult>), String> {
    let focus_modules = request
        .focus
        .iter()
        .filter_map(|selector| selector_for_language(selector, language))
        .collect::<Vec<_>>();
    let query = match (request.path_from, request.path_to, request.closure) {
        (Some(from), Some(to), _) => {
            let from = selector_for_language(from, language);
            let to = selector_for_language(to, language);
            match (from, to) {
                (Some(from), Some(to)) => Some(DependencyGraphQuery::ShortestPath { from, to }),
                (None, None) => None,
                _ => return Err("dependency path endpoints must use the same language".to_owned()),
            }
        }
        (_, _, Some(module)) => {
            selector_for_language(module, language).map(|module| DependencyGraphQuery::Closure {
                module,
                direction: match request.direction {
                    Some(DependencyDirectionArgument::Dependents) => {
                        DependencyQueryDirection::Dependents
                    }
                    _ => DependencyQueryDirection::Dependencies,
                },
            })
        }
        _ => None,
    };
    let query_result = query
        .as_ref()
        .map(|query| query_dependency_graph(graph, query))
        .transpose()
        .map_err(|error| format!("could not query {language} dependency graph: {error}"))?;
    let mut view = if let Some(result) = &query_result {
        dependency_query_view(graph, result)
    } else {
        filter_dependency_graph(
            graph,
            &DependencyGraphFilter {
                focus_modules,
                direction: request
                    .direction
                    .map(Into::into)
                    .unwrap_or(DependencyDirection::Both),
                depth: request.depth.unwrap_or(1),
                exact_only: request.exact_only,
                local_only: request.local_only,
                cycles_only: request.cycles_only,
            },
        )
        .map_err(|error| format!("could not filter {language} dependency graph: {error}"))?
    };
    for node in &mut view.nodes {
        node.id = format!("{language}_{}", node.id);
    }
    Ok((view, query_result))
}

fn build_dependency_run(
    analysis: &RepositoryAnalysis,
    request: &DependencyRequest<'_>,
    resolver: &dyn DependencyResolver,
) -> Result<LanguageDependencyRun, String> {
    let descriptor = resolver.descriptor();
    let language = descriptor.language.as_str();
    let resolution = resolver
        .resolve(analysis)
        .map_err(|error| format!("could not resolve {language} dependencies: {error}"))?;
    let included = resolution
        .resolutions
        .iter()
        .filter(|resolution| request.exclusions.retains(&resolution.reference))
        .cloned()
        .collect::<Vec<_>>();
    let graph = analyze_dependency_graph(&resolution.local_units, &included)
        .map_err(|error| format!("could not build {language} dependency graph: {error}"))?;
    let (view, query) = graph_projection(language, &graph, request)?;
    let environment_report = resolution
        .metadata
        .get("environment-selection")
        .map(|selection| DependencyEnvironmentReport {
            selection: selection.clone(),
            implementation: resolution.metadata["environment-implementation"].clone(),
            python_version: resolution.metadata["environment-version"].clone(),
            virtual_environment: resolution.metadata["environment-is-virtual"] == "true",
            distribution_count: resolution.metadata["environment-distributions"]
                .parse()
                .expect("resolver distribution count must be numeric"),
        });
    Ok(LanguageDependencyRun {
        language: language.to_owned(),
        resolver: DependencyResolverReport {
            id: descriptor.id.clone(),
            version: descriptor.version.clone(),
            definition_version: descriptor.definition_version.clone(),
            unit_kind: descriptor.local_unit_kind.as_str(),
            hierarchy_behavior: descriptor.hierarchy_behavior.clone(),
            resolution_capabilities: descriptor.resolution_capabilities.clone(),
            status: "available",
            context: resolution
                .context_coverage
                .iter()
                .map(|context| DependencyResolverContextReport {
                    kind: context.kind.clone(),
                    selected: context.selected,
                    total: context.total,
                    supported: context.supported,
                    unsupported: context.unsupported,
                })
                .collect(),
            limitations: descriptor.limitations.clone(),
        },
        environment: environment_report,
        summary_lines: resolution.summary_lines,
        graph,
        view,
        query,
        diagnostics: resolution.diagnostics,
    })
}

fn print_dependency_query(result: &DependencyGraphQueryResult) {
    match &result.query {
        DependencyGraphQuery::ShortestPath { from, to } => {
            if result.found {
                println!("Shortest dependency path ({from} -> {to}):");
                println!(
                    "  {}",
                    result
                        .nodes
                        .iter()
                        .map(dependency_node_name)
                        .collect::<Vec<_>>()
                        .join(" -> ")
                );
            } else {
                println!("No exact local dependency path found from {from} to {to}.");
            }
        }
        DependencyGraphQuery::Closure { module, direction } => {
            println!("{} closure for {module}:", direction.as_str());
            for node in &result.nodes {
                println!("  {}", dependency_node_name(node));
            }
        }
    }
}

fn print_dependency_bundle_json(
    runs: &[LanguageDependencyRun],
    unavailable: Vec<UnavailableDependencyLanguage>,
    request: &DependencyRequest<'_>,
) -> ExitCode {
    let languages = runs
        .iter()
        .map(|run| DependencyLanguageJsonReport {
            language: run.language.clone(),
            resolver: run.resolver.clone(),
            graph: DependencyJsonReport::from_analysis_with_query_and_exclusions(
                &run.graph,
                &run.view,
                run.environment.clone(),
                run.query.as_ref(),
                request.exclusions,
            ),
        })
        .collect();
    match serde_json::to_string_pretty(&DependencyBundleJsonReport::new(languages, unavailable)) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: could not serialize dependency report: {error}");
            ExitCode::FAILURE
        }
    }
}

fn render_dependency_mermaid_bundle(runs: &[LanguageDependencyRun]) -> String {
    let mut output = String::from("flowchart LR\n");
    for run in runs {
        let slug = language_slug(&run.language);
        output.push_str(&format!(
            "  subgraph language_{slug}[\"{}\"]\n",
            run.language
        ));
        let rendered = render_dependency_mermaid(&run.view)
            .strip_prefix("flowchart LR\n")
            .unwrap_or_default()
            .replace("cluster_cycle_", &format!("{slug}_cycle_"));
        for line in rendered.lines() {
            output.push_str("    ");
            output.push_str(line);
            output.push('\n');
        }
        output.push_str("  end\n");
    }
    output
}

fn render_dependency_dot_bundle(runs: &[LanguageDependencyRun]) -> String {
    let mut output = String::from(
        "digraph dependencies {\n  rankdir=LR;\n  graph [fontname=\"Helvetica\"];\n  node [fontname=\"Helvetica\", style=filled];\n  edge [fontname=\"Helvetica\"];\n",
    );
    for run in runs {
        let slug = language_slug(&run.language);
        output.push_str(&format!(
            "  subgraph cluster_language_{slug} {{\n    label=\"{}\";\n",
            run.language.replace('"', "\\\"")
        ));
        let rendered = render_dependency_dot(&run.view);
        let body = rendered
            .strip_prefix("digraph dependencies {\n")
            .and_then(|value| value.strip_suffix("}\n"))
            .unwrap_or_default()
            .replace("cluster_cycle_", &format!("{slug}_cycle_"));
        for line in body.lines().skip(4) {
            output.push_str("    ");
            output.push_str(line);
            output.push('\n');
        }
        output.push_str("  }\n");
    }
    output.push_str("}\n");
    output
}

fn emit_dependency_html_bundle(
    project: &Path,
    runs: &[LanguageDependencyRun],
    unavailable: &[UnavailableDependencyLanguage],
    output: Option<&Path>,
    open: bool,
) -> ExitCode {
    let output = output.unwrap_or_else(|| Path::new("codegraide-dependencies"));
    let mut pages = Vec::<(String, String)>::new();
    let mut used_names = BTreeSet::new();
    let language_pages = runs
        .iter()
        .map(|run| {
            let filename = format!("{}.html", language_slug(&run.language));
            if !used_names.insert(filename.clone()) {
                return Err(format!(
                    "dependency language filenames collide at {filename:?}"
                ));
            }
            Ok((run.language.clone(), filename))
        })
        .collect::<Result<Vec<_>, String>>();
    let language_pages = match language_pages {
        Ok(pages) => pages,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    for (run, (_, filename)) in runs.iter().zip(&language_pages) {
        let presentation = codegraide_core::DependencyExplorerPresentation::new(
            &run.language,
            run.resolver.unit_kind,
        );
        let rendered = codegraide_core::render_dependency_html_with_presentation(
            &run.view,
            run.query.as_ref(),
            &presentation,
        );
        let html = match rendered {
            Ok(html) => inject_dependency_navigation(&html, &run.language, &language_pages),
            Err(error) => {
                eprintln!(
                    "error: could not render {} dependency graph: {error}",
                    run.language
                );
                return ExitCode::FAILURE;
            }
        };
        pages.push((filename.clone(), html));
    }
    pages.push((
        "index.html".to_owned(),
        dependency_index_html(project, runs, unavailable, &language_pages),
    ));
    let generated_files = pages
        .iter()
        .map(|(name, _)| name.clone())
        .chain(std::iter::once(
            "codegraide-dependency-report.json".to_owned(),
        ))
        .collect::<Vec<_>>();
    let manifest = serde_json::json!({
        "format": "codegraide-dependency-html-bundle-v1",
        "generated_files": generated_files,
        "languages": language_pages.iter().map(|(language, file)| {
            serde_json::json!({"language": language, "file": file})
        }).collect::<Vec<_>>()
    });
    if let Err(error) = prepare_dependency_output_directory(output, &generated_files) {
        eprintln!(
            "error: could not prepare dependency report directory {}: {error}",
            output.display()
        );
        return ExitCode::FAILURE;
    }
    for (filename, contents) in pages {
        if let Err(error) = fs::write(output.join(&filename), contents) {
            eprintln!(
                "error: could not write dependency report file {}: {error}",
                output.join(filename).display()
            );
            return ExitCode::FAILURE;
        }
    }
    let manifest_path = output.join("codegraide-dependency-report.json");
    let manifest = serde_json::to_string_pretty(&manifest).expect("manifest serialization");
    if let Err(error) = fs::write(&manifest_path, manifest) {
        eprintln!(
            "error: could not write dependency report manifest {}: {error}",
            manifest_path.display()
        );
        return ExitCode::FAILURE;
    }
    eprintln!("wrote dependency report bundle to {}", output.display());
    if open {
        let index = output.join("index.html");
        if let Err(error) = open_in_default_browser(&index) {
            eprintln!(
                "error: could not open dependency report {}: {error}",
                index.display()
            );
            return ExitCode::FAILURE;
        }
        eprintln!("opened dependency report overview");
    }
    ExitCode::SUCCESS
}

fn prepare_dependency_output_directory(
    output: &Path,
    generated_files: &[String],
) -> io::Result<()> {
    if output.exists() && !output.is_dir() {
        return Err(io::Error::other(
            "output path exists and is not a directory",
        ));
    }
    fs::create_dir_all(output)?;
    let existing = fs::read_dir(output)?.collect::<Result<Vec<_>, _>>()?;
    if existing.is_empty() {
        return Ok(());
    }
    let manifest_path = output.join("codegraide-dependency-report.json");
    let manifest_source = fs::read_to_string(&manifest_path).map_err(|_| {
        io::Error::other("directory is nonempty and has no Codegraide dependency manifest")
    })?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_source)
        .map_err(|error| io::Error::other(format!("invalid dependency manifest: {error}")))?;
    if manifest["format"] != "codegraide-dependency-html-bundle-v1" {
        return Err(io::Error::other(
            "directory does not contain a recognized Codegraide dependency bundle",
        ));
    }
    let retained = generated_files.iter().collect::<BTreeSet<_>>();
    for filename in manifest["generated_files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        if filename.contains('/') || filename.contains('\\') || filename == "." || filename == ".."
        {
            return Err(io::Error::other(
                "dependency manifest contains an unsafe filename",
            ));
        }
        if !retained.contains(&filename.to_owned()) {
            let stale = output.join(filename);
            if stale.is_file() {
                fs::remove_file(stale)?;
            }
        }
    }
    Ok(())
}

fn language_slug(language: &str) -> String {
    let slug = language
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    if slug.is_empty() {
        "language".to_owned()
    } else {
        slug
    }
}

fn inject_dependency_navigation(html: &str, current: &str, pages: &[(String, String)]) -> String {
    if pages.len() == 1 {
        return html.to_owned();
    }
    let mut navigation = String::from(
        "<nav class=\"language-tabs\" aria-label=\"Dependency languages\"><a href=\"index.html\">Overview</a>",
    );
    for (language, filename) in pages {
        navigation.push_str(&format!(
            "<a href=\"{}\"{}>{}</a>",
            html_escape(filename),
            if language == current {
                " aria-current=\"page\""
            } else {
                ""
            },
            html_escape(language)
        ));
    }
    navigation.push_str("</nav>");
    let style = "<style>.language-tabs{display:flex;gap:.45rem;padding:.65rem 1rem;background:var(--surface);border-bottom:1px solid var(--border);position:sticky;top:0;z-index:20}.language-tabs a{color:var(--text);text-decoration:none;padding:.45rem .7rem;border:1px solid var(--border);border-radius:.45rem}.language-tabs a[aria-current=page]{background:var(--accent-soft);border-color:var(--accent);color:var(--accent)}</style>";
    html.replacen("</head>", &format!("{style}</head>"), 1)
        .replacen("<body>", &format!("<body>{navigation}"), 1)
}

fn dependency_index_html(
    project: &Path,
    runs: &[LanguageDependencyRun],
    unavailable: &[UnavailableDependencyLanguage],
    pages: &[(String, String)],
) -> String {
    let mut cards = String::new();
    for (run, (_, filename)) in runs.iter().zip(pages) {
        cards.push_str(&format!(
            "<a class=\"card\" href=\"{}\"><h2>{}</h2><p>{} nodes · {} relations · {} cycles</p><p>Resolution: {} exact, {} inferred, {} context-dependent, {} unresolved</p></a>",
            html_escape(filename),
            html_escape(&run.language),
            run.graph.nodes.len(),
            run.graph.relations.len(),
            run.graph.cycles.len(),
            run.graph.coverage.exact_references,
            run.graph.coverage.inferred_references,
            run.graph.coverage.context_dependent_references,
            run.graph.coverage.unresolved_references
        ));
    }
    for language in unavailable {
        cards.push_str(&format!(
            "<div class=\"card unavailable\"><h2>{}</h2><p>Dependency resolver not installed.</p></div>",
            html_escape(&language.language)
        ));
    }
    let repository_name = project
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repository");
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Codegraide Dependency Report</title><style>:root{{color-scheme:light dark;font-family:system-ui,sans-serif}}body{{margin:0;background:#f6f7fb;color:#18202b}}main{{max-width:960px;margin:auto;padding:2rem}}.tabs{{display:flex;gap:.5rem;margin:1rem 0 2rem}}.tabs a{{padding:.5rem .75rem;border:1px solid #cfd6e2;border-radius:.5rem;text-decoration:none;color:inherit}}.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:1rem}}.card{{display:block;padding:1rem 1.2rem;background:white;border:1px solid #d9dee7;border-radius:.8rem;color:inherit;text-decoration:none;box-shadow:0 8px 24px #18202b12}}.card h2{{text-transform:uppercase;font-size:1rem}}.unavailable{{opacity:.7}}@media(prefers-color-scheme:dark){{body{{background:#11151c;color:#edf1f7}}.card{{background:#181e27;border-color:#303947}}}}</style></head><body><main><h1>Codegraide Dependency Report</h1><p>Repository: <strong>{}</strong>. Each language graph is isolated and loads in its own page.</p><nav class=\"tabs\">{}</nav><section class=\"grid\">{cards}</section></main></body></html>",
        html_escape(repository_name),
        pages
            .iter()
            .map(|(language, file)| format!(
                "<a href=\"{}\">{}</a>",
                html_escape(file),
                html_escape(language)
            ))
            .collect::<String>()
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn open_in_default_browser(path: &Path) -> io::Result<()> {
    let path = path.canonicalize()?;
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg(&path);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]).arg(&path);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(&path);
        command
    };
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "browser opener exited with status {status}"
        )))
    }
}

fn print_dependency_bundle_summary(
    path: &Path,
    runs: &[LanguageDependencyRun],
    unavailable: &[UnavailableDependencyLanguage],
    request: &DependencyRequest<'_>,
) {
    println!("Dependency project: {}", path.display());
    println!(
        "Dependency languages: {}",
        if runs.is_empty() {
            "none".to_owned()
        } else {
            runs.iter()
                .map(|run| run.language.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    for language in unavailable {
        println!(
            "Unavailable language: {} ({})",
            language.language, language.status
        );
    }
    for run in runs {
        println!(
            "\n{} dependencies [{} {}]:",
            run.language.to_uppercase(),
            run.resolver.id,
            run.resolver.version
        );
        for line in &run.summary_lines {
            println!("  {line}");
        }
        if let Some(result) = &run.query {
            print_dependency_query(result);
            continue;
        }
        print_dependency_language_summary(run, request.exclusions, request.top);
    }
}

fn print_dependency_language_summary(
    run: &LanguageDependencyRun,
    exclusions: DependencyGraphInputExclusions,
    top: Option<usize>,
) {
    let graph = &run.graph;
    let view = &run.view;
    println!(
        "  Resolution coverage: total={} exact={} inferred={} ambiguous={} context-dependent={} unresolved={}",
        graph.coverage.total_references,
        graph.coverage.exact_references,
        graph.coverage.inferred_references,
        graph.coverage.ambiguous_references,
        graph.coverage.context_dependent_references,
        graph.coverage.unresolved_references
    );
    let contexts = graph
        .relations
        .iter()
        .flat_map(|relation| relation.evidence.iter())
        .filter_map(|evidence| {
            evidence
                .reference
                .as_import()
                .map(|reference| reference.context)
        })
        .collect::<Vec<_>>();
    if !contexts.is_empty() {
        println!(
            "  Import contexts: type-only={} optional={} callable-local={} conditional={}",
            contexts
                .iter()
                .filter(|context| context.usage.as_str() == "type-checking-only")
                .count(),
            contexts
                .iter()
                .filter(|context| context.requirement.as_str() == "optional")
                .count(),
            contexts
                .iter()
                .filter(|context| context.scope.as_str() == "callable")
                .count(),
            contexts
                .iter()
                .filter(|context| context.conditional)
                .count()
        );
    }
    let mut excluded = Vec::new();
    if exclusions.type_only {
        excluded.push("type-only");
    }
    if exclusions.optional {
        excluded.push("optional");
    }
    if exclusions.callable_local {
        excluded.push("callable-local");
    }
    if exclusions.conditional {
        excluded.push("conditional");
    }
    println!(
        "  Graph input exclusions: {}",
        if excluded.is_empty() {
            "none".to_owned()
        } else {
            excluded.join(", ")
        }
    );
    println!(
        "  Graph: nodes={} relations={} cycles={} | view: nodes={} relations={}",
        graph.nodes.len(),
        graph.relations.len(),
        graph.cycles.len(),
        view.nodes.len(),
        view.relations.len()
    );
    let limit = top.unwrap_or(10);
    if graph.coverage.inferred_references > 0 {
        print_local_structural_ranking(
            "Highest local fan-in (exact + inferred)",
            view,
            limit,
            true,
        );
        print_local_structural_ranking(
            "Highest local fan-out (exact + inferred)",
            view,
            limit,
            false,
        );
    }
    print_dependency_ranking("Highest exact fan-in", view, limit, true);
    print_dependency_ranking("Highest exact fan-out", view, limit, false);
    println!("\nCycles:");
    let visible = view
        .nodes
        .iter()
        .map(|node| &node.node)
        .collect::<BTreeSet<_>>();
    let explanations = explain_dependency_cycles(graph)
        .into_iter()
        .filter(|explanation| {
            explanation
                .members
                .iter()
                .all(|member| visible.contains(member))
        })
        .collect::<Vec<_>>();
    if explanations.is_empty() {
        println!("  (none)");
    } else {
        for explanation in explanations {
            println!(
                "  {} witness: {}",
                explanation.component_number,
                explanation
                    .witness_nodes
                    .iter()
                    .map(dependency_node_name)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
            println!("    recommended cuts (approximate):");
            for relation in explanation.recommended_cuts {
                println!(
                    "      {} -> {} ({} reference {})",
                    dependency_node_name(&relation.source),
                    dependency_node_name(&relation.target),
                    relation.evidence.len(),
                    if relation.evidence.len() == 1 {
                        "site"
                    } else {
                        "sites"
                    }
                );
                for evidence in relation.evidence {
                    match &evidence.reference {
                        codegraide_core::DependencyReference::Import(reference) => println!(
                            "        {}:{}:{} [{}; {}; {}{}]",
                            evidence.source_path.display(),
                            reference.span.start.line,
                            reference.span.start.column,
                            reference.context.scope.as_str(),
                            reference.context.usage.as_str(),
                            reference.context.requirement.as_str(),
                            if reference.context.conditional {
                                "; conditional"
                            } else {
                                ""
                            }
                        ),
                        codegraide_core::DependencyReference::Include(reference) => println!(
                            "        {}:{}:{} [{}{}]",
                            evidence.source_path.display(),
                            reference.span.start.line,
                            reference.span.start.column,
                            reference.delimiter.as_str(),
                            if reference.conditional {
                                "; conditional"
                            } else {
                                ""
                            }
                        ),
                    }
                }
            }
        }
    }
    let unresolved = view
        .nodes
        .iter()
        .filter(|node| node.node.kind() == DependencyNodeKind::Unresolved)
        .count();
    let ambiguous = view
        .nodes
        .iter()
        .filter(|node| node.node.kind() == DependencyNodeKind::Ambiguous)
        .count();
    let context_dependent = view
        .nodes
        .iter()
        .filter(|node| node.node.kind() == DependencyNodeKind::ContextDependent)
        .count();
    println!(
        "\nInvestigation nodes: ambiguous={ambiguous} context-dependent={context_dependent} unresolved={unresolved}"
    );
}

fn print_dependency_ranking(heading: &str, view: &DependencyGraphView, limit: usize, fan_in: bool) {
    let mut nodes = view
        .nodes
        .iter()
        .filter(|node| node.node.kind() == DependencyNodeKind::LocalModule)
        .filter(|node| if fan_in { node.fan_in } else { node.fan_out } > 0)
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        let left_value = if fan_in { left.fan_in } else { left.fan_out };
        let right_value = if fan_in { right.fan_in } else { right.fan_out };
        right_value
            .cmp(&left_value)
            .then_with(|| dependency_node_name(&left.node).cmp(&dependency_node_name(&right.node)))
    });
    println!("\n{heading}:");
    if nodes.is_empty() {
        println!("  (none)");
    } else {
        for node in nodes.into_iter().take(limit) {
            let value = if fan_in { node.fan_in } else { node.fan_out };
            println!("  {value:>4}  {}", dependency_node_name(&node.node));
        }
    }
}

fn print_local_structural_ranking(
    heading: &str,
    view: &DependencyGraphView,
    limit: usize,
    fan_in: bool,
) {
    let mut counts = BTreeMap::<DependencyNode, usize>::new();
    for relation in &view.relations {
        if !matches!(
            relation.kind,
            DependencyRelationKind::Exact | DependencyRelationKind::Inferred
        ) || relation.source.kind() != DependencyNodeKind::LocalModule
            || relation.target.kind() != DependencyNodeKind::LocalModule
        {
            continue;
        }
        let node = if fan_in {
            &relation.target
        } else {
            &relation.source
        };
        *counts.entry(node.clone()).or_default() += 1;
    }
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|(left_node, left_count), (right_node, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| dependency_node_name(left_node).cmp(&dependency_node_name(right_node)))
    });
    println!("\n{heading}:");
    if counts.is_empty() {
        println!("  (none)");
    } else {
        for (node, count) in counts.into_iter().take(limit) {
            println!("  {count:>4}  {}", dependency_node_name(&node));
        }
    }
}

fn dependency_node_name(node: &DependencyNode) -> String {
    match node {
        DependencyNode::LocalModule(module) => module.id.qualified_name().to_owned(),
        DependencyNode::StandardLibrary(module) => module.qualified_name().to_owned(),
        DependencyNode::InstalledDistribution {
            distribution_display_name,
            version,
            ..
        } => format!(
            "{}{}",
            distribution_display_name,
            version
                .as_ref()
                .map(|version| format!("=={version}"))
                .unwrap_or_default()
        ),
        DependencyNode::SystemHeader { name, .. } | DependencyNode::ExternalHeader { name, .. } => {
            name.clone()
        }
        DependencyNode::Ambiguous { requested, .. }
        | DependencyNode::Unresolved { requested, .. }
        | DependencyNode::ContextDependent { requested, .. } => requested.clone(),
    }
}

#[derive(Debug, Clone, Copy)]
struct CallRequest<'a> {
    path: &'a Path,
    language: Option<CallLanguageArgument>,
    compile_commands: Option<&'a Path>,
    architecture: Option<&'a Path>,
    python: Option<&'a Path>,
    venv: Option<&'a Path>,
    focus: &'a [String],
    direction: Option<CallDirectionArgument>,
    depth: Option<usize>,
    exact_only: bool,
    local_only: bool,
    cycles_only: bool,
    format: CallOutputFormat,
    output: Option<&'a Path>,
    open: bool,
    include_source: bool,
    max_expansion_depth: Option<u8>,
}

fn run_calls(request: &CallRequest<'_>) -> ExitCode {
    if request.output.is_some() && request.format != CallOutputFormat::Html {
        eprintln!("error: --output requires --format html");
        return ExitCode::FAILURE;
    }
    if request.open && request.format != CallOutputFormat::Html {
        eprintln!("error: --open requires --format html");
        return ExitCode::FAILURE;
    }
    if request.include_source && request.format != CallOutputFormat::Html {
        eprintln!("error: --include-source requires --format html");
        return ExitCode::FAILURE;
    }
    if request.max_expansion_depth.is_some()
        && (!request.include_source || request.format != CallOutputFormat::Html)
    {
        eprintln!("error: --max-expansion-depth requires --include-source and --format html");
        return ExitCode::FAILURE;
    }
    if !request.path.is_dir() {
        eprintln!(
            "error: call analysis requires a project directory: {}",
            request.path.display()
        );
        return ExitCode::FAILURE;
    }
    let mut registry = match build_builtin_analyzer_registry(BuiltinAnalyzerFeatures {
        documentation: false,
    }) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let analysis = match analyze_repository(
        &AnalysisOptions {
            target: request.path.to_path_buf(),
            ..AnalysisOptions::default()
        },
        &mut registry,
    ) {
        Ok(analysis) => analysis,
        Err(error) => {
            eprintln!("error: failed to analyze call project: {error}");
            return ExitCode::FAILURE;
        }
    };
    let present_languages = analysis
        .analyzers
        .iter()
        .filter(|run| {
            !run.files.is_empty()
                && run
                    .descriptor
                    .capabilities
                    .contains(&AnalyzerCapability::CallReferences)
        })
        .map(|run| run.descriptor.language.as_str())
        .collect::<BTreeSet<_>>();
    let language = match request.language {
        Some(language) if present_languages.contains(language.as_str()) => language,
        Some(language) => {
            eprintln!(
                "error: no {} files with call analysis support were found",
                language.as_str()
            );
            return ExitCode::FAILURE;
        }
        None if present_languages.len() == 1 => {
            if present_languages.contains("cpp") {
                CallLanguageArgument::Cpp
            } else {
                CallLanguageArgument::Python
            }
        }
        None if present_languages.len() > 1 => {
            eprintln!(
                "error: call analysis found Python and C++; select --language python or --language cpp"
            );
            return ExitCode::FAILURE;
        }
        None => {
            eprintln!("error: no files with call analysis support were found");
            return ExitCode::FAILURE;
        }
    };
    if language == CallLanguageArgument::Cpp && (request.python.is_some() || request.venv.is_some())
    {
        eprintln!("error: --python and --venv apply only to --language python");
        return ExitCode::FAILURE;
    }
    if language == CallLanguageArgument::Python
        && (request.compile_commands.is_some() || request.architecture.is_some())
    {
        eprintln!("error: --compile-commands and --architecture apply only to --language cpp");
        return ExitCode::FAILURE;
    }
    let (symbols, resolutions, diagnostics, language_modules) = match language {
        CallLanguageArgument::Python => {
            let environment = request
                .python
                .map(|path| PythonEnvironmentSelection::Interpreter(path.to_path_buf()))
                .or_else(|| {
                    request.venv.map(|path| {
                        PythonEnvironmentSelection::VirtualEnvironment(path.to_path_buf())
                    })
                });
            let dependencies = match resolve_python_dependencies(
                &analysis,
                &PythonResolutionOptions { environment },
            ) {
                Ok(resolution) => resolution,
                Err(error) => {
                    eprintln!("error: could not resolve Python imports for call analysis: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let resolution = resolve_python_calls(&analysis, &dependencies);
            let diagnostics: Vec<String> = dependencies
                .diagnostics
                .iter()
                .chain(resolution.diagnostics.iter())
                .cloned()
                .collect();
            (
                resolution.symbols,
                resolution.resolutions,
                diagnostics,
                Vec::new(),
            )
        }
        CallLanguageArgument::Cpp => {
            let dependencies = match resolve_cpp_dependencies(
                &analysis,
                &CppResolutionOptions {
                    compilation_database: request.compile_commands.map(Path::to_path_buf),
                },
            ) {
                Ok(resolution) => resolution,
                Err(error) => {
                    eprintln!("error: could not resolve C++ includes for call analysis: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let mut resolution = resolve_cpp_calls(&analysis, &dependencies);
            if let Some(path) = request.architecture
                && let Err(error) = apply_architecture_to_resolution(path, &mut resolution)
            {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
            let diagnostics: Vec<String> = dependencies
                .diagnostics
                .iter()
                .chain(resolution.diagnostics.iter())
                .cloned()
                .collect();
            (
                resolution.symbols,
                resolution.resolutions,
                diagnostics,
                resolution.modules,
            )
        }
    };
    let graph = analyze_call_graph_with_modules(&symbols, &resolutions, language_modules);
    let filter = CallGraphFilter {
        focus_symbols: request.focus.to_vec(),
        direction: request
            .direction
            .map(Into::into)
            .unwrap_or(CallDirection::Both),
        depth: request.depth.unwrap_or(1),
        exact_only: request.exact_only,
        local_only: request.local_only,
        cycles_only: request.cycles_only,
    };
    let view = match filter_call_graph(&graph, &filter) {
        Ok(view) => view,
        Err(error) => {
            eprintln!("error: could not filter call graph: {error}");
            return ExitCode::FAILURE;
        }
    };
    for diagnostic in &diagnostics {
        eprintln!("warning: {diagnostic}");
    }
    if matches!(
        request.format,
        CallOutputFormat::Mermaid | CallOutputFormat::Dot
    ) && view.nodes.len() > 200
    {
        eprintln!(
            "warning: call graph contains {} nodes; use --focus, --local-only, or --cycles-only",
            view.nodes.len()
        );
    }
    match request.format {
        CallOutputFormat::Terminal => {
            print_call_summary(request.path, &graph, &view);
            ExitCode::SUCCESS
        }
        CallOutputFormat::Json => {
            match serde_json::to_string_pretty(&CallJsonReport::from_analysis_for_language(
                language.as_str(),
                &graph,
                &view,
            )) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("error: could not serialize call report: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        CallOutputFormat::Mermaid => {
            print!("{}", render_call_mermaid(&view));
            ExitCode::SUCCESS
        }
        CallOutputFormat::Dot => {
            print!("{}", render_call_dot(&view));
            ExitCode::SUCCESS
        }
        CallOutputFormat::Html => emit_call_html(
            &view,
            request.path,
            request.include_source,
            request.max_expansion_depth.unwrap_or(3),
            request.output,
            request.open,
        ),
    }
}

fn print_call_summary(path: &Path, graph: &CallGraphAnalysis, view: &CallGraphView) {
    println!("Call project: {}", path.display());
    println!(
        "Resolution coverage: total={} exact={} inferred={} external={} ambiguous={} unresolved={} unavailable={}",
        graph.coverage.total_calls,
        graph.coverage.exact_calls,
        graph.coverage.inferred_calls,
        graph.coverage.external_calls,
        graph.coverage.ambiguous_calls,
        graph.coverage.unresolved_calls,
        graph.coverage.unavailable_calls
    );
    println!(
        "Graph: nodes={} calls={} recursive-groups={} | view: nodes={} calls={}",
        graph.nodes.len(),
        graph.relations.len(),
        graph.cycles.len(),
        view.nodes.len(),
        view.relations.len()
    );
    print_call_ranking("Highest caller fan-in", view, true);
    print_call_ranking("Highest callee fan-out", view, false);
    println!("\nRecursive groups:");
    let cycles = view
        .strongly_connected_components
        .iter()
        .filter(|component| component.cyclic)
        .collect::<Vec<_>>();
    if cycles.is_empty() {
        println!("  (none)");
    } else {
        for (index, cycle) in cycles.iter().enumerate() {
            println!(
                "  {}: {}",
                index + 1,
                cycle
                    .members
                    .iter()
                    .map(call_node_name)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
    }
}

fn print_call_ranking(heading: &str, view: &CallGraphView, fan_in: bool) {
    let mut nodes = view
        .nodes
        .iter()
        .filter(|node| matches!(node.node, codegraide_core::CallNode::LocalSymbol(_)))
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        let left_value = if fan_in { left.fan_in } else { left.fan_out };
        let right_value = if fan_in { right.fan_in } else { right.fan_out };
        right_value
            .cmp(&left_value)
            .then_with(|| call_node_name(&left.node).cmp(&call_node_name(&right.node)))
    });
    println!("\n{heading}:");
    for node in nodes.into_iter().take(10) {
        let value = if fan_in { node.fan_in } else { node.fan_out };
        println!("  {value:>4}  {}", call_node_name(&node.node));
    }
}

fn emit_call_html(
    view: &CallGraphView,
    project_root: &Path,
    include_source: bool,
    max_expansion_depth: u8,
    output: Option<&Path>,
    open: bool,
) -> ExitCode {
    let html = match include_source {
        true => render_call_html_with_source(view, project_root, max_expansion_depth)
            .map_err(|error| error.to_string()),
        false => render_call_html(view).map_err(|error| error.to_string()),
    };
    let html = match html {
        Ok(html) => html,
        Err(error) => {
            eprintln!("error: could not render interactive call graph: {error}");
            return ExitCode::FAILURE;
        }
    };
    if output.is_none() && !open {
        print!("{html}");
        return ExitCode::SUCCESS;
    }
    let output = output.unwrap_or_else(|| Path::new("codegraide-call-graph.html"));
    if let Err(error) = fs::write(output, html) {
        eprintln!(
            "error: could not write call graph {}: {error}",
            output.display()
        );
        return ExitCode::FAILURE;
    }
    eprintln!("wrote interactive call graph to {}", output.display());
    if open && let Err(error) = open_in_default_browser(output) {
        eprintln!(
            "error: could not open call graph {}: {error}",
            output.display()
        );
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[derive(Debug, Clone, Copy)]
struct CommentsRequest<'a> {
    path: &'a Path,
    match_patterns: &'a [String],
    include_ignored: &'a [String],
    policy: Option<&'a Path>,
    documentation_review_below: Option<u8>,
    include_tests: bool,
    top: Option<usize>,
    gate: bool,
    format: CommentsOutputFormat,
}

fn run_comments(request: &CommentsRequest<'_>) -> ExitCode {
    let mut registry = match build_builtin_analyzer_registry(BuiltinAnalyzerFeatures {
        documentation: true,
    }) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let analysis = match analyze_repository(
        &AnalysisOptions {
            target: request.path.to_path_buf(),
            match_patterns: request.match_patterns.to_vec(),
            include_ignored: request.include_ignored.to_vec(),
            documentation_coverage: true,
            documentation_include_tests: request.include_tests,
            review: ReviewOptions {
                policy_path: request.policy.map(Path::to_path_buf),
                documentation_review_below: request.documentation_review_below,
                ..ReviewOptions::default()
            },
        },
        &mut registry,
    ) {
        Ok(analysis) => analysis,
        Err(error) => {
            eprintln!(
                "error: failed to analyze documentation coverage for {}: {error}",
                request.path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    if request.gate && analysis.review.policy.documentation_review_below.is_none() {
        eprintln!(
            "error: comments --gate requires --documentation-review-below or a policy threshold"
        );
        return ExitCode::FAILURE;
    }

    let output_status = match request.format {
        CommentsOutputFormat::Terminal => {
            print_documentation_summary(request.path, &analysis, request.top.unwrap_or(20));
            ExitCode::SUCCESS
        }
        CommentsOutputFormat::Json => {
            match serde_json::to_string_pretty(&DocumentationJsonReport::from_analysis(
                &analysis,
                request.top,
            )) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("error: could not serialize documentation report: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    };
    if output_status != ExitCode::SUCCESS {
        return output_status;
    }
    if request.gate {
        review_exit_code(documentation_review_status(&analysis))
    } else {
        ExitCode::SUCCESS
    }
}

fn documentation_review_status(analysis: &RepositoryAnalysis) -> ReviewStatus {
    if analysis.review.findings.iter().any(|finding| {
        finding
            .rule_id
            .starts_with("python-documentation-coverage-")
            && finding.required_action == codegraide_core::RequiredAction::HumanReview
    }) {
        ReviewStatus::HumanReviewRequired
    } else {
        ReviewStatus::Pass
    }
}

fn print_documentation_summary(path: &Path, analysis: &RepositoryAnalysis, top: usize) {
    let coverage = &analysis.documentation_coverage;
    println!("Documentation target: {}", path.display());
    println!("Status: {}.", human_documentation_status(coverage.status));
    println!(
        "Files: applicable: {}, skipped tests: {}, unsupported: {}.",
        coverage.applicable_files, coverage.skipped_test_files, coverage.unsupported_selected_files
    );
    println!(
        "Coverage: documented: {}/{}, missing: {}, unavailable: {}, coverage: {}.",
        coverage.counts.documented,
        coverage.counts.measured(),
        coverage.counts.missing,
        coverage.counts.unavailable,
        format_documentation_percentage(coverage.counts.coverage_basis_points())
    );
    println!("\nBy symbol kind:");
    if coverage.by_kind.is_empty() {
        println!("  (none)");
    } else {
        for (kind, counts) in &coverage.by_kind {
            println!(
                "  {}: documented: {}/{}, missing: {}, unavailable: {}, coverage: {}",
                kind.as_str(),
                counts.documented,
                counts.measured(),
                counts.missing,
                counts.unavailable,
                format_documentation_percentage(counts.coverage_basis_points())
            );
        }
    }
    println!("\nFiles:");
    if coverage.files.is_empty() {
        println!("  (none)");
    } else {
        for file in &coverage.files {
            println!(
                "  {}: documented: {}/{}, missing: {}, unavailable: {}, coverage: {}",
                file.path.display(),
                file.counts.documented,
                file.counts.measured(),
                file.counts.missing,
                file.counts.unavailable,
                format_documentation_percentage(file.counts.coverage_basis_points())
            );
        }
    }
    println!("\nMissing documentation:");
    if coverage.missing_symbols.is_empty() {
        println!("  (none)");
    } else {
        for symbol in coverage.missing_symbols.iter().take(top) {
            println!(
                "  {}:{} {} {}",
                symbol.path.display(),
                symbol.span.start.line,
                symbol.kind.as_str(),
                symbol.qualified_name
            );
        }
        if coverage.missing_symbols.len() > top {
            println!(
                "  ... {} more (use --top to change the limit)",
                coverage.missing_symbols.len() - top
            );
        }
    }
    if !coverage.unavailable_symbols.is_empty() {
        println!("\nUnavailable documentation evidence:");
        for symbol in coverage.unavailable_symbols.iter().take(top) {
            println!(
                "  {}:{} {} {}: {}",
                symbol.path.display(),
                symbol.span.start.line,
                symbol.kind.as_str(),
                symbol.qualified_name,
                symbol.reason.as_deref().unwrap_or("unavailable")
            );
        }
    }
    for finding in analysis.review.findings.iter().filter(|finding| {
        finding
            .rule_id
            .starts_with("python-documentation-coverage-")
    }) {
        println!("\nReview: {}", finding.message);
    }
}

fn format_documentation_percentage(basis_points: Option<u16>) -> String {
    basis_points.map_or_else(
        || "not available".to_owned(),
        |value| format!("{}.{:02}%", value / 100, value % 100),
    )
}

#[derive(Debug, Clone, Copy)]
struct AnalyzeRequest<'a> {
    path: &'a Path,
    match_patterns: &'a [String],
    include_ignored: &'a [String],
    diagnostics: &'a [String],
    details: &'a [String],
    policy: Option<&'a Path>,
    complexity_review_at: Option<u64>,
    complexity_block_at: Option<u64>,
    no_complexity_block: bool,
    no_documentation_coverage: bool,
    include_tests: bool,
    documentation_review_below: Option<u8>,
    gate: bool,
    profile: ReportProfile,
    top: Option<usize>,
    format: AnalyzeOutputFormat,
}

fn run_analyze(request: &AnalyzeRequest<'_>) -> ExitCode {
    let AnalyzeRequest {
        path,
        match_patterns,
        include_ignored,
        diagnostics,
        details,
        policy,
        complexity_review_at,
        complexity_block_at,
        no_complexity_block,
        no_documentation_coverage,
        include_tests,
        documentation_review_below,
        gate,
        profile,
        top,
        format,
    } = *request;
    let diagnostic_request = match parse_diagnostic_request(diagnostics) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let detail_request = match parse_detail_request(details) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    if format != AnalyzeOutputFormat::Terminal && diagnostic_request.is_some() {
        if detail_request.is_some() {
            eprintln!("error: --diagnostics and --details are only available with terminal output");
        } else {
            eprintln!("error: --diagnostics is only available with terminal output");
        }
        return ExitCode::FAILURE;
    }
    if format != AnalyzeOutputFormat::Terminal && detail_request.is_some() {
        eprintln!("error: --details is only available with terminal output");
        return ExitCode::FAILURE;
    }
    if format != AnalyzeOutputFormat::Json && profile != ReportProfile::Full {
        eprintln!("error: --profile review requires --format json");
        return ExitCode::FAILURE;
    }

    let documentation = !no_documentation_coverage;
    let mut registry =
        match build_builtin_analyzer_registry(BuiltinAnalyzerFeatures { documentation }) {
            Ok(registry) => registry,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        };

    let analysis = match analyze_repository(
        &AnalysisOptions {
            target: path.to_path_buf(),
            match_patterns: match_patterns.to_vec(),
            include_ignored: include_ignored.to_vec(),
            documentation_coverage: documentation,
            documentation_include_tests: include_tests,
            review: ReviewOptions {
                policy_path: policy.map(Path::to_path_buf),
                complexity_review_at,
                complexity_block_at,
                no_complexity_block,
                documentation_review_below,
            },
        },
        &mut registry,
    ) {
        Ok(analysis) => analysis,
        Err(error) => {
            eprintln!("error: failed to analyze {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let output_status = match format {
        AnalyzeOutputFormat::Json => print_analysis_json(&analysis, profile, top),
        AnalyzeOutputFormat::Gate => print_gate_json(&analysis, top),
        AnalyzeOutputFormat::Terminal => {
            print_analysis_summary(path, &analysis, top);
            if let Some(request) = detail_request {
                if let Err(message) = print_details(&analysis, request) {
                    eprintln!("error: {message}");
                    return ExitCode::FAILURE;
                }
            }
            if let Some(request) = diagnostic_request {
                if let Err(message) = print_diagnostics(&analysis, request) {
                    eprintln!("error: {message}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
    };
    if output_status != ExitCode::SUCCESS {
        return output_status;
    }
    if gate {
        review_exit_code(analysis.review.status)
    } else {
        ExitCode::SUCCESS
    }
}

#[derive(Debug, Clone)]
enum DiagnosticRequest {
    All,
    Files(Vec<PathBuf>),
}

fn parse_diagnostic_request(values: &[String]) -> Result<Option<DiagnosticRequest>, String> {
    if values.is_empty() {
        return Ok(None);
    }
    let all_count = values
        .iter()
        .filter(|value| value.as_str() == "__ALL_DIAGNOSTICS__")
        .count();
    if all_count > 0 {
        if values.len() != all_count {
            return Err("--diagnostics cannot mix a bare request with file paths".to_owned());
        }
        return Ok(Some(DiagnosticRequest::All));
    }
    Ok(Some(DiagnosticRequest::Files(
        values.iter().map(PathBuf::from).collect(),
    )))
}

fn parse_detail_request(values: &[String]) -> Result<Option<DiagnosticRequest>, String> {
    if values.is_empty() {
        return Ok(None);
    }
    let all_count = values
        .iter()
        .filter(|value| value.as_str() == "__ALL_DETAILS__")
        .count();
    if all_count > 0 {
        if values.len() != all_count {
            return Err("--details cannot mix a bare request with file paths".to_owned());
        }
        return Ok(Some(DiagnosticRequest::All));
    }
    Ok(Some(DiagnosticRequest::Files(
        values.iter().map(PathBuf::from).collect(),
    )))
}

fn print_analysis_json(
    analysis: &RepositoryAnalysis,
    profile: ReportProfile,
    top: Option<usize>,
) -> ExitCode {
    let json = match profile {
        ReportProfile::Full => {
            serde_json::to_string_pretty(&AnalysisJsonReport::from_analysis_with_top(analysis, top))
        }
        ReportProfile::Review => serde_json::to_string_pretty(&ReviewJsonReport::from_analysis(
            analysis,
            Some(top.unwrap_or(20)),
        )),
    };
    match json {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: could not serialize analysis report: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_gate_json(analysis: &RepositoryAnalysis, top: Option<usize>) -> ExitCode {
    match serde_json::to_string_pretty(&GateJsonReport::from_analysis(analysis, top)) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: could not serialize gate report: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_analysis_summary(path: &Path, analysis: &RepositoryAnalysis, top: Option<usize>) {
    println!("Analysis target: {}", path.display());
    println!("Inventoried files: {}", analysis.inventoried_files);
    println!(
        "Selected files: {}",
        analysis.selection.selected_files.len()
    );
    println!(
        "Review status: {}",
        human_review_status(analysis.review.status)
    );
    println!(
        "Review coverage: Measured callables: {}/{}, unavailable callables: {}, unsupported files: {}.",
        analysis.review.coverage.measured_callables,
        analysis.review.coverage.eligible_callables,
        analysis.review.coverage.unavailable_callables,
        analysis.review.coverage.unsupported_selected_files
    );
    let documentation = &analysis.documentation_coverage;
    println!(
        "Documentation coverage: {}. Applicable files: {}, skipped test files: {}, documented: {}/{}, missing: {}, unavailable: {}, coverage: {}.",
        human_documentation_status(documentation.status),
        documentation.applicable_files,
        documentation.skipped_test_files,
        documentation.counts.documented,
        documentation.counts.measured(),
        documentation.counts.missing,
        documentation.counts.unavailable,
        format_documentation_percentage(documentation.counts.coverage_basis_points())
    );
    println!("\nLanguages:");
    if analysis.inventory_languages.is_empty() {
        println!("  (none)");
    } else {
        for (language, count) in &analysis.inventory_languages {
            let marker = if analysis.inventory_only_languages.contains_key(language) {
                " (inventory only)"
            } else {
                ""
            };
            println!("  {}: {count}{marker}", language.as_str());
        }
    }
    println!("\nAnalyzers:");
    if analysis.analyzers.is_empty() {
        println!("  (none)");
    }
    for run in &analysis.analyzers {
        println!(
            "  {} [{}]: analyzed: {}, successful: {}, partial: {}, failed: {}",
            run.descriptor.language.as_str(),
            run.descriptor.id,
            run.counts.analyzed,
            run.counts.successful,
            run.counts.partial,
            run.counts.failed
        );
        print_diagnostic_summary(run);
        let symbol_counts = run
            .files
            .iter()
            .flat_map(|file| file.facts.symbols.iter())
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, symbol| {
                *counts.entry(symbol.kind.as_str()).or_default() += 1;
                counts
            });
        let dependency_counts = run
            .files
            .iter()
            .flat_map(|file| file.facts.dependencies.iter())
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, dependency| {
                *counts.entry(dependency.kind().as_str()).or_default() += 1;
                counts
            });
        let facts = symbol_counts
            .into_iter()
            .chain(dependency_counts)
            .map(|(kind, count)| format!("{}: {count}", fact_count_label(kind)))
            .collect::<Vec<_>>();
        println!(
            "    facts: {}",
            if facts.is_empty() {
                "(none)".to_owned()
            } else {
                facts.join(", ")
            }
        );
        let explicit_export_counts =
            run.files
                .iter()
                .fold(([0usize; 4], 0usize), |(mut statuses, mut names), file| {
                    if let Some(exports) = &file.facts.explicit_exports {
                        match exports.status {
                            codegraide_core::ExplicitExportStatus::NotDeclared => statuses[0] += 1,
                            codegraide_core::ExplicitExportStatus::Complete => statuses[1] += 1,
                            codegraide_core::ExplicitExportStatus::Partial => statuses[2] += 1,
                            codegraide_core::ExplicitExportStatus::Unavailable => statuses[3] += 1,
                        }
                        names += exports.names.len();
                    }
                    (statuses, names)
                });
        if explicit_export_counts.0.iter().sum::<usize>() > 0 {
            println!(
                "    explicit exports: complete: {}, partial: {}, unavailable: {}, not declared: {}, names: {}",
                explicit_export_counts.0[1],
                explicit_export_counts.0[2],
                explicit_export_counts.0[3],
                explicit_export_counts.0[0],
                explicit_export_counts.1
            );
        }
        print_top_measurements(
            run,
            MeasurementConcept::DeclarationPhysicalLines,
            "longest declarations",
        );
        print_top_measurements(
            run,
            MeasurementConcept::MaxControlFlowNesting,
            "deepest nesting",
        );
        print_top_measurements(
            run,
            MeasurementConcept::CyclomaticComplexity,
            "highest cyclomatic complexity",
        );
    }
    if !analysis.review.findings.is_empty() {
        println!("\nReview findings:");
        for finding in analysis.review.findings.iter().take(top.unwrap_or(5)) {
            let location = finding
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "repository".to_owned());
            println!("  {}", human_review_finding(finding, &location));
        }
    }
    for diagnostic in &analysis.diagnostics {
        println!(
            "\n{}[{}]: {}",
            diagnostic.severity.as_str(),
            diagnostic.code,
            diagnostic.message
        );
    }
}

fn print_diagnostic_summary(run: &codegraide_core::AnalyzerRun) {
    let mut total = 0usize;
    let mut file_counts = BTreeMap::<PathBuf, usize>::new();
    let mut groups = BTreeMap::<(String, String, String), (usize, BTreeSet<PathBuf>)>::new();
    for file in &run.files {
        if file.diagnostics.is_empty() {
            continue;
        }
        total += file.diagnostics.len();
        file_counts.insert(file.path.clone(), file.diagnostics.len());
        for diagnostic in &file.diagnostics {
            let group = groups
                .entry((
                    diagnostic.severity.as_str().to_owned(),
                    diagnostic.code.clone(),
                    diagnostic.message.clone(),
                ))
                .or_default();
            group.0 += 1;
            group.1.insert(file.path.clone());
        }
    }
    if total == 0 {
        return;
    }

    println!(
        "    diagnostics: {total} total across {} {}",
        file_counts.len(),
        file_word(file_counts.len())
    );
    let mut ranked_groups = groups.into_iter().collect::<Vec<_>>();
    ranked_groups
        .sort_by(|left, right| right.1.0.cmp(&left.1.0).then_with(|| left.0.cmp(&right.0)));
    for ((severity, code, message), (count, paths)) in ranked_groups.iter().take(5) {
        println!(
            "      {severity}[{code}]: {count} in {} {} — {message}",
            paths.len(),
            file_word(paths.len())
        );
    }
    if ranked_groups.len() > 5 {
        println!(
            "      ... {} more diagnostic groups",
            ranked_groups.len() - 5
        );
    }

    let mut ranked_files = file_counts.into_iter().collect::<Vec<_>>();
    ranked_files.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    println!("      files with the most diagnostics:");
    for (path, count) in ranked_files.iter().take(5) {
        println!("        {}: {count}", path.display());
    }
    if ranked_files.len() > 5 {
        println!("        ... {} more files", ranked_files.len() - 5);
    }
    println!("      Full details: use --diagnostics [FILE].");
}

fn file_word(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn human_review_status(status: ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Pass => "Passed.",
        ReviewStatus::HumanReviewRequired => "Human review required.",
        ReviewStatus::Blocked => "Blocked.",
    }
}

fn human_documentation_status(
    status: codegraide_core::DocumentationCoverageStatus,
) -> &'static str {
    match status {
        codegraide_core::DocumentationCoverageStatus::Disabled => "Disabled",
        codegraide_core::DocumentationCoverageStatus::NotApplicable => "Not applicable",
        codegraide_core::DocumentationCoverageStatus::Complete => "Complete",
        codegraide_core::DocumentationCoverageStatus::Partial => "Partial",
    }
}

fn human_review_finding(finding: &codegraide_core::ReviewFinding, location: &str) -> String {
    let action = match finding.required_action {
        codegraide_core::RequiredAction::None => "No action required",
        codegraide_core::RequiredAction::HumanReview => "Human review required",
        codegraide_core::RequiredAction::Block => "Blocked",
    };
    let risk = match finding.risk {
        codegraide_core::RiskLevel::Low => "low risk",
        codegraide_core::RiskLevel::Moderate => "moderate risk",
        codegraide_core::RiskLevel::High => "high risk",
        codegraide_core::RiskLevel::Critical => "critical risk",
        codegraide_core::RiskLevel::Unknown => "unknown risk",
    };
    format!("{action} for {location} ({risk}): {}", finding.message)
}

fn fact_count_label(kind: &str) -> &str {
    match kind {
        "class" => "classes",
        "function" => "functions",
        "import" => "imports",
        "include" => "includes",
        "lambda" => "lambdas",
        "method" => "methods",
        "module" => "modules",
        "namespace" => "namespaces",
        "struct" => "structs",
        _ => kind,
    }
}

fn print_top_measurements(
    run: &codegraide_core::AnalyzerRun,
    concept: MeasurementConcept,
    label: &str,
) {
    let Some(metric) = run
        .descriptor
        .measurements
        .iter()
        .find(|metric| metric.concept == concept)
    else {
        return;
    };
    let mut values = run
        .files
        .iter()
        .flat_map(|file| {
            file.facts.symbols.iter().filter_map(|symbol| {
                if !matches!(
                    symbol.kind,
                    codegraide_core::SymbolKind::Function
                        | codegraide_core::SymbolKind::Method
                        | codegraide_core::SymbolKind::Lambda
                ) {
                    return None;
                }
                let value = symbol
                    .measurements
                    .iter()
                    .find(|measurement| measurement.id == metric.id)
                    .and_then(|measurement| measurement.value)?;
                Some((value, file.path.clone(), symbol.qualified_name.clone()))
            })
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    if values.is_empty() {
        return;
    }
    println!("    {label}:");
    for (value, path, qualified_name) in values.into_iter().take(3) {
        println!("      {value:>4}  {}::{qualified_name}", path.display());
    }
}

fn print_diagnostics(
    analysis: &RepositoryAnalysis,
    request: DiagnosticRequest,
) -> Result<(), String> {
    let requested_paths = match request {
        DiagnosticRequest::All => None,
        DiagnosticRequest::Files(paths) => Some(
            paths
                .into_iter()
                .map(|path| normalize_diagnostic_path(analysis, &path))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    println!("\nDiagnostics:");
    let mut printed_any = false;
    for run in &analysis.analyzers {
        for file in &run.files {
            if requested_paths
                .as_ref()
                .is_some_and(|paths| !paths.iter().any(|path| path == &file.path))
            {
                continue;
            }
            printed_any = true;
            println!("  {}:", file.path.display());
            if file.diagnostics.is_empty() {
                println!("    (none)");
                continue;
            }
            for diagnostic in &file.diagnostics {
                print_one_diagnostic(diagnostic);
            }
        }
    }
    if let Some(paths) = requested_paths {
        for path in paths {
            if !analysis
                .analyzers
                .iter()
                .flat_map(|run| run.files.iter())
                .any(|file| file.path == path)
            {
                return Err(format!(
                    "diagnostic path {} was not selected or is not supported",
                    path.display()
                ));
            }
        }
    }
    if !printed_any {
        println!("  (none)");
    }
    Ok(())
}

fn print_details(analysis: &RepositoryAnalysis, request: DiagnosticRequest) -> Result<(), String> {
    let requested_paths = match request {
        DiagnosticRequest::All => None,
        DiagnosticRequest::Files(paths) => Some(
            paths
                .into_iter()
                .map(|path| normalize_diagnostic_path(analysis, &path))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    println!("\nDetails:");
    let mut printed_any = false;
    for run in &analysis.analyzers {
        for file in &run.files {
            if requested_paths
                .as_ref()
                .is_some_and(|paths| !paths.iter().any(|path| path == &file.path))
            {
                continue;
            }
            printed_any = true;
            println!("  {}:", file.path.display());
            for symbol in &file.facts.symbols {
                println!(
                    "    {} {} [{}]",
                    symbol.kind.as_str(),
                    symbol.qualified_name,
                    symbol.completeness.as_str()
                );
                for measurement in &symbol.measurements {
                    println!(
                        "      metric {}: {}",
                        measurement.id,
                        measurement
                            .value
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "unavailable".to_owned())
                    );
                }
                if let Some(documentation) = &symbol.documentation {
                    println!(
                        "      documentation: {}{}",
                        documentation.status.as_str(),
                        documentation
                            .span
                            .map(|span| format!(" at {}:{}", span.start.line, span.start.column))
                            .unwrap_or_default()
                    );
                }
                for event in &symbol.decision_events {
                    println!(
                        "      decision {}: {}:{}-{}:{}",
                        event.kind.as_str(),
                        event.span.start.line,
                        event.span.start.column,
                        event.span.end.line,
                        event.span.end.column
                    );
                }
            }
            for dependency in &file.facts.dependencies {
                match dependency {
                    codegraide_core::DependencyReference::Import(dependency) => println!(
                        "    import {}{}",
                        dependency.module.as_deref().unwrap_or("."),
                        dependency
                            .imported_name
                            .as_ref()
                            .map(|name| format!("::{name}"))
                            .unwrap_or_default()
                    ),
                    codegraide_core::DependencyReference::Include(dependency) => println!(
                        "    include {} [{}{}]",
                        dependency.target,
                        dependency.delimiter.as_str(),
                        if dependency.conditional {
                            "; conditional"
                        } else {
                            ""
                        }
                    ),
                }
            }
            if let Some(exports) = &file.facts.explicit_exports {
                println!("    explicit exports [{}]", exports.status.as_str());
                if let Some(span) = exports.declaration_span {
                    println!(
                        "      declaration: {}:{}-{}:{}",
                        span.start.line, span.start.column, span.end.line, span.end.column
                    );
                }
                for name in &exports.names {
                    println!(
                        "      {:?}: {}:{}-{}:{}",
                        name.name,
                        name.span.start.line,
                        name.span.start.column,
                        name.span.end.line,
                        name.span.end.column
                    );
                }
                if let Some(reason) = &exports.reason {
                    println!("      reason: {reason}");
                }
            }
        }
    }
    if let Some(paths) = &requested_paths {
        for path in paths {
            if !analysis
                .analyzers
                .iter()
                .flat_map(|run| run.files.iter())
                .any(|file| file.path == *path)
            {
                return Err(format!(
                    "details path {} was not selected or is not supported",
                    path.display()
                ));
            }
        }
    }
    if !printed_any {
        println!("  (none)");
    }
    Ok(())
}

fn review_exit_code(status: ReviewStatus) -> ExitCode {
    ExitCode::from(review_status_code(status))
}

fn normalize_diagnostic_path(
    analysis: &RepositoryAnalysis,
    path: &Path,
) -> Result<PathBuf, String> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        analysis.selection.root.join(path)
    };
    candidate
        .strip_prefix(&analysis.selection.root)
        .map(Path::to_path_buf)
        .map_err(|_| {
            format!(
                "diagnostic path {} is outside the analysis root",
                path.display()
            )
        })
}

fn print_one_diagnostic(diagnostic: &codegraide_core::AnalysisDiagnostic) {
    let location = diagnostic.span.map(|span| {
        format!(
            "{}:{}-{}:{}",
            span.start.line, span.start.column, span.end.line, span.end.column
        )
    });
    match location {
        Some(location) => println!(
            "    {}[{}] {location}: {}",
            diagnostic.severity.as_str(),
            diagnostic.code,
            diagnostic.message
        ),
        None => println!(
            "    {}[{}]: {}",
            diagnostic.severity.as_str(),
            diagnostic.code,
            diagnostic.message
        ),
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
            "  {}: {}",
            category.as_str(),
            inventory.category_count(category)
        );
    }

    println!("\nLanguages:");
    if inventory.files_by_language.is_empty() {
        println!("  (none)");
    } else {
        for (language, count) in &inventory.files_by_language {
            println!("  {}: {count}", language.as_str());
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
                "    {}: files: {}, source: {}, comment: {}, blank: {}",
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
            println!("  {}: {count}", extension.as_str());
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
        } = args.command
        else {
            panic!("expected inventory command");
        };

        assert_eq!(path, PathBuf::from("example"));
        assert_eq!(include_ignored, ["generated/**", "vendor/**"]);
        assert_eq!(config, Some(PathBuf::from("full.json")));
        assert!(audit_ignored);
        assert!(no_warnings);
        assert_eq!(
            list_files,
            [FileListSelection::Source, FileListSelection::Uncategorized]
        );
        assert_eq!(format, InventoryOutputFormat::Terminal);
    }

    #[test]
    fn dependency_language_slugs_are_deterministic_and_filesystem_safe() {
        assert_eq!(language_slug("C++"), "c--");
        assert_eq!(language_slug("Objective C"), "objective-c");
        assert_eq!(language_slug(""), "language");
    }
}
