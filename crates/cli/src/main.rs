use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use codegraide_analyzer_python::{
    PythonAnalyzer, PythonEnvironmentSelection, PythonResolutionOptions, resolve_python_calls,
    resolve_python_dependencies,
};
use codegraide_core::{
    AnalysisJsonReport, AnalysisOptions, AnalyzerRegistry, CallDirection, CallGraphAnalysis,
    CallGraphFilter, CallGraphView, CallJsonReport, DependencyDirection,
    DependencyEnvironmentReport, DependencyGraphAnalysis, DependencyGraphFilter,
    DependencyGraphInputExclusions, DependencyGraphQuery, DependencyGraphQueryResult,
    DependencyGraphView, DependencyJsonReport, DependencyNode, DependencyNodeKind,
    DependencyQueryDirection, DocumentationJsonReport, FileCategory, GateJsonReport,
    InventoryJsonReport, InventoryOptions, RepositoryAnalysis, RepositoryInventory,
    ReviewJsonReport, ReviewOptions, ReviewStatus, analyze_call_graph, analyze_dependency_graph,
    analyze_repository, call_node_name, dependency_query_view, explain_dependency_cycles,
    filter_call_graph, filter_dependency_graph, inventory_repository_with_options,
    query_dependency_graph, render_call_dot, render_call_html, render_call_mermaid,
    render_dependency_dot, render_dependency_html, render_dependency_html_with_query,
    render_dependency_mermaid, review_status_code,
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
        #[arg(long, value_enum, default_value_t = InventoryOutputFormat::Terminal)]
        format: InventoryOutputFormat,
    },
    /// Parse supported source files and report syntax recovery diagnostics
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

        /// Return gate-specific exit codes for review status
        #[arg(long)]
        gate: bool,

        /// Select the JSON report profile; full preserves the complete analysis report
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
    /// Resolve Python imports and build a dependency graph
    Dependencies {
        /// Project directory to analyze
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

        /// Python interpreter used for standard-library and installed-package resolution
        #[arg(long, value_name = "EXECUTABLE", conflicts_with = "venv")]
        python: Option<PathBuf>,

        /// Virtual-environment directory used for installed-package resolution
        #[arg(long, value_name = "DIRECTORY", conflicts_with = "python")]
        venv: Option<PathBuf>,

        /// Focus the view on one local module; may repeat
        #[arg(
            long,
            value_name = "MODULE",
            action = clap::ArgAction::Append,
            conflicts_with_all = ["path_from", "path_to", "closure"]
        )]
        focus: Vec<String>,

        /// Traverse dependencies, dependents, or both from a focus or closure root
        #[arg(long, value_enum, conflicts_with_all = ["path_from", "path_to"])]
        direction: Option<DependencyDirectionArgument>,

        /// Traversal depth from focused modules; zero shows only the focus and its cycle
        #[arg(long, value_name = "N", requires = "focus")]
        depth: Option<usize>,

        /// Find the shortest exact local dependency path from this module
        #[arg(
            long,
            value_name = "MODULE",
            requires = "path_to",
            conflicts_with_all = ["focus", "depth", "cycles_only", "closure", "direction"]
        )]
        path_from: Option<String>,

        /// Find the shortest exact local dependency path to this module
        #[arg(
            long,
            value_name = "MODULE",
            requires = "path_from",
            conflicts_with_all = ["focus", "depth", "cycles_only", "closure", "direction"]
        )]
        path_to: Option<String>,

        /// Show the complete exact local dependency or dependent closure
        #[arg(
            long,
            value_name = "MODULE",
            conflicts_with_all = ["focus", "depth", "cycles_only", "path_from", "path_to"]
        )]
        closure: Option<String>,

        /// Hide ambiguous and unresolved relations
        #[arg(long)]
        exact_only: bool,

        /// Show repository-local modules and exact local relations only
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

        /// Write an interactive HTML graph to this file
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Open the interactive HTML graph in the default browser
        #[arg(long)]
        open: bool,
    },
    /// Resolve conservative Python call targets and build a symbol call graph
    Calls {
        /// Project directory to analyze
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,

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

        /// Hide ambiguous, unresolved, and external call boundaries
        #[arg(long)]
        exact_only: bool,

        /// Show project symbols and exact local calls only
        #[arg(long)]
        local_only: bool,

        /// Show only recursive local strongly connected components
        #[arg(long)]
        cycles_only: bool,

        /// Output format
        #[arg(long, value_enum, default_value_t = CallOutputFormat::Terminal)]
        format: CallOutputFormat,

        /// Write HTML output to this file
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Open HTML output in the default browser
        #[arg(long)]
        open: bool,
    },
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
        Command::Calls {
            path,
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
        } => run_calls(&CallRequest {
            path,
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

    let mut registry = AnalyzerRegistry::new();
    let analyzer = match PythonAnalyzer::without_documentation() {
        Ok(analyzer) => analyzer,
        Err(error) => {
            eprintln!("error: could not initialize Python analyzer: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = registry.register(Box::new(analyzer)) {
        eprintln!("error: could not register Python analyzer: {error}");
        return ExitCode::FAILURE;
    }
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
    let environment = request
        .python
        .map(|path| PythonEnvironmentSelection::Interpreter(path.to_path_buf()))
        .or_else(|| {
            request
                .venv
                .map(|path| PythonEnvironmentSelection::VirtualEnvironment(path.to_path_buf()))
        });
    let resolution =
        match resolve_python_dependencies(&analysis, &PythonResolutionOptions { environment }) {
            Ok(resolution) => resolution,
            Err(error) => {
                eprintln!("error: could not resolve Python dependencies: {error}");
                return ExitCode::FAILURE;
            }
        };
    let included_resolutions = resolution
        .resolutions
        .iter()
        .filter(|resolution| request.exclusions.retains(&resolution.reference))
        .cloned()
        .collect::<Vec<_>>();
    let graph = match analyze_dependency_graph(&resolution.local_modules, &included_resolutions) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("error: could not build dependency graph: {error}");
            return ExitCode::FAILURE;
        }
    };
    let filter = DependencyGraphFilter {
        focus_modules: request.focus.to_vec(),
        direction: request
            .direction
            .map(Into::into)
            .unwrap_or(DependencyDirection::Both),
        depth: request.depth.unwrap_or(1),
        exact_only: request.exact_only,
        local_only: request.local_only,
        cycles_only: request.cycles_only,
    };
    let query = request
        .path_from
        .zip(request.path_to)
        .map(|(from, to)| DependencyGraphQuery::ShortestPath {
            from: from.to_owned(),
            to: to.to_owned(),
        })
        .or_else(|| {
            request.closure.map(|module| DependencyGraphQuery::Closure {
                module: module.to_owned(),
                direction: match request.direction {
                    Some(DependencyDirectionArgument::Dependents) => {
                        DependencyQueryDirection::Dependents
                    }
                    _ => DependencyQueryDirection::Dependencies,
                },
            })
        });
    let query_result = match query
        .as_ref()
        .map(|query| query_dependency_graph(&graph, query))
        .transpose()
    {
        Ok(result) => result,
        Err(error) => {
            eprintln!("error: could not query dependency graph: {error}");
            return ExitCode::FAILURE;
        }
    };
    let view = if let Some(result) = &query_result {
        dependency_query_view(&graph, result)
    } else {
        match filter_dependency_graph(&graph, &filter) {
            Ok(view) => view,
            Err(error) => {
                eprintln!("error: could not filter dependency graph: {error}");
                return ExitCode::FAILURE;
            }
        }
    };
    for diagnostic in &resolution.diagnostics {
        eprintln!("warning: {diagnostic}");
    }
    if matches!(
        request.format,
        DependencyOutputFormat::Mermaid | DependencyOutputFormat::Dot
    ) && view.nodes.len() > 200
    {
        eprintln!(
            "warning: graph contains {} nodes; use --focus, --local-only, or --cycles-only for a more readable view",
            view.nodes.len()
        );
    }

    match request.format {
        DependencyOutputFormat::Terminal => {
            if let Some(result) = &query_result {
                print_dependency_query(result);
            } else {
                print_dependency_summary(
                    request.path,
                    &resolution,
                    &graph,
                    &view,
                    request.exclusions,
                    request.top,
                );
            }
            ExitCode::SUCCESS
        }
        DependencyOutputFormat::Mermaid => {
            print!("{}", render_dependency_mermaid(&view));
            ExitCode::SUCCESS
        }
        DependencyOutputFormat::Dot => {
            print!("{}", render_dependency_dot(&view));
            ExitCode::SUCCESS
        }
        DependencyOutputFormat::Json => {
            let environment =
                resolution
                    .environment
                    .as_ref()
                    .map(|environment| DependencyEnvironmentReport {
                        selection: environment.selection_kind,
                        implementation: environment.implementation.clone(),
                        python_version: environment.version.clone(),
                        virtual_environment: environment.is_virtual_environment,
                        distribution_count: environment.distribution_count,
                    });
            match serde_json::to_string_pretty(
                &DependencyJsonReport::from_analysis_with_query_and_exclusions(
                    &graph,
                    &view,
                    environment,
                    query_result.as_ref(),
                    request.exclusions,
                ),
            ) {
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
        DependencyOutputFormat::Html => {
            emit_dependency_html(&view, query_result.as_ref(), request.output, request.open)
        }
    }
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

fn emit_dependency_html(
    view: &DependencyGraphView,
    query: Option<&DependencyGraphQueryResult>,
    output: Option<&Path>,
    open: bool,
) -> ExitCode {
    let html = match if query.is_some() {
        render_dependency_html_with_query(view, query)
    } else {
        render_dependency_html(view)
    } {
        Ok(html) => html,
        Err(error) => {
            eprintln!("error: could not render interactive dependency graph: {error}");
            return ExitCode::FAILURE;
        }
    };
    if output.is_none() && !open {
        print!("{html}");
        return ExitCode::SUCCESS;
    }

    let output = output.unwrap_or_else(|| Path::new("codegraide-dependency-graph.html"));
    if let Err(error) = fs::write(output, html) {
        eprintln!(
            "error: could not write interactive dependency graph {}: {error}",
            output.display()
        );
        return ExitCode::FAILURE;
    }
    eprintln!("wrote interactive dependency graph to {}", output.display());

    if open {
        if let Err(error) = open_in_default_browser(output) {
            eprintln!(
                "error: could not open interactive dependency graph {}: {error}",
                output.display()
            );
            return ExitCode::FAILURE;
        }
        eprintln!("opened interactive dependency graph in the default browser");
    }
    ExitCode::SUCCESS
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

fn print_dependency_summary(
    path: &Path,
    resolution: &codegraide_analyzer_python::PythonDependencyResolution,
    graph: &DependencyGraphAnalysis,
    view: &DependencyGraphView,
    exclusions: DependencyGraphInputExclusions,
    top: Option<usize>,
) {
    println!("Dependency project: {}", path.display());
    println!(
        "Package roots: {}",
        resolution
            .package_roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    match &resolution.environment {
        Some(environment) => println!(
            "Python environment: {} {} ({}, packages={})\n  executable: {}",
            environment.implementation,
            environment.version,
            environment.selection_kind,
            environment.distribution_count,
            environment.executable.display()
        ),
        None => println!("Python environment: not selected (external imports remain unresolved)"),
    }
    println!(
        "Resolution coverage: total={} exact={} ambiguous={} unresolved={}",
        graph.coverage.total_references,
        graph.coverage.exact_references,
        graph.coverage.ambiguous_references,
        graph.coverage.unresolved_references
    );
    let contexts = graph
        .relations
        .iter()
        .flat_map(|relation| {
            relation
                .evidence
                .iter()
                .map(|evidence| evidence.reference.context)
        })
        .collect::<Vec<_>>();
    println!(
        "Import contexts: type-only={} optional={} callable-local={} conditional={}",
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
        "Graph input exclusions: {}",
        if excluded.is_empty() {
            "none".to_owned()
        } else {
            excluded.join(", ")
        }
    );
    println!(
        "Graph: nodes={} relations={} cycles={} | view: nodes={} relations={}",
        graph.nodes.len(),
        graph.relations.len(),
        graph.cycles.len(),
        view.nodes.len(),
        view.relations.len()
    );
    let limit = top.unwrap_or(10);
    print_dependency_ranking("Highest fan-in", view, limit, true);
    print_dependency_ranking("Highest fan-out", view, limit, false);
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
                    "      {} -> {} ({} import {})",
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
                    println!(
                        "        {}:{}:{} [{}; {}; {}{}]",
                        evidence.source_path.display(),
                        evidence.reference.span.start.line,
                        evidence.reference.span.start.column,
                        evidence.reference.context.scope.as_str(),
                        evidence.reference.context.usage.as_str(),
                        evidence.reference.context.requirement.as_str(),
                        if evidence.reference.context.conditional {
                            "; conditional"
                        } else {
                            ""
                        }
                    );
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
    println!("\nInvestigation nodes: ambiguous={ambiguous} unresolved={unresolved}");
}

fn print_dependency_ranking(heading: &str, view: &DependencyGraphView, limit: usize, fan_in: bool) {
    let mut nodes = view
        .nodes
        .iter()
        .filter(|node| node.node.kind() == DependencyNodeKind::LocalModule)
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
        DependencyNode::Ambiguous { requested, .. }
        | DependencyNode::Unresolved { requested, .. } => requested.clone(),
    }
}

#[derive(Debug, Clone, Copy)]
struct CallRequest<'a> {
    path: &'a Path,
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
    if !request.path.is_dir() {
        eprintln!(
            "error: call analysis requires a project directory: {}",
            request.path.display()
        );
        return ExitCode::FAILURE;
    }
    let mut registry = AnalyzerRegistry::new();
    let analyzer = match PythonAnalyzer::without_documentation() {
        Ok(analyzer) => analyzer,
        Err(error) => {
            eprintln!("error: could not initialize Python analyzer: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = registry.register(Box::new(analyzer)) {
        eprintln!("error: could not register Python analyzer: {error}");
        return ExitCode::FAILURE;
    }
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
    let environment = request
        .python
        .map(|path| PythonEnvironmentSelection::Interpreter(path.to_path_buf()))
        .or_else(|| {
            request
                .venv
                .map(|path| PythonEnvironmentSelection::VirtualEnvironment(path.to_path_buf()))
        });
    let dependencies =
        match resolve_python_dependencies(&analysis, &PythonResolutionOptions { environment }) {
            Ok(resolution) => resolution,
            Err(error) => {
                eprintln!("error: could not resolve Python imports for call analysis: {error}");
                return ExitCode::FAILURE;
            }
        };
    let resolution = resolve_python_calls(&analysis, &dependencies);
    let graph = analyze_call_graph(&resolution.symbols, &resolution.resolutions);
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
    for diagnostic in dependencies
        .diagnostics
        .iter()
        .chain(resolution.diagnostics.iter())
    {
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
            match serde_json::to_string_pretty(&CallJsonReport::from_analysis(&graph, &view)) {
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
        CallOutputFormat::Html => emit_call_html(&view, request.output, request.open),
    }
}

fn print_call_summary(path: &Path, graph: &CallGraphAnalysis, view: &CallGraphView) {
    println!("Call project: {}", path.display());
    println!(
        "Resolution coverage: total={} exact={} external={} ambiguous={} unresolved={}",
        graph.coverage.total_calls,
        graph.coverage.exact_calls,
        graph.coverage.external_calls,
        graph.coverage.ambiguous_calls,
        graph.coverage.unresolved_calls
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

fn emit_call_html(view: &CallGraphView, output: Option<&Path>, open: bool) -> ExitCode {
    let html = match render_call_html(view) {
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
    let mut registry = AnalyzerRegistry::new();
    let analyzer = match PythonAnalyzer::new() {
        Ok(analyzer) => analyzer,
        Err(error) => {
            eprintln!("error: could not initialize Python analyzer: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = registry.register(Box::new(analyzer)) {
        eprintln!("error: could not register Python analyzer: {error}");
        return ExitCode::FAILURE;
    }
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
    println!("Status: {}", coverage.status.as_str());
    println!(
        "Files: applicable={} skipped_tests={} unsupported={}",
        coverage.applicable_files, coverage.skipped_test_files, coverage.unsupported_selected_files
    );
    println!(
        "Coverage: documented={}/{} missing={} unavailable={} ({})",
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
                "  {}: documented={}/{} missing={} unavailable={} ({})",
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
                "  {}: documented={}/{} missing={} unavailable={} ({})",
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

    let mut registry = AnalyzerRegistry::new();
    let analyzer = match if no_documentation_coverage {
        PythonAnalyzer::without_documentation()
    } else {
        PythonAnalyzer::new()
    } {
        Ok(analyzer) => analyzer,
        Err(error) => {
            eprintln!("error: could not initialize Python analyzer: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = registry.register(Box::new(analyzer)) {
        eprintln!("error: could not register Python analyzer: {error}");
        return ExitCode::FAILURE;
    }

    let analysis = match analyze_repository(
        &AnalysisOptions {
            target: path.to_path_buf(),
            match_patterns: match_patterns.to_vec(),
            include_ignored: include_ignored.to_vec(),
            documentation_coverage: !no_documentation_coverage,
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
    println!("Review status: {}", analysis.review.status.as_str());
    println!(
        "Review coverage: measured={}/{} unavailable={} unsupported-files={}",
        analysis.review.coverage.measured_callables,
        analysis.review.coverage.eligible_callables,
        analysis.review.coverage.unavailable_callables,
        analysis.review.coverage.unsupported_selected_files
    );
    let documentation = &analysis.documentation_coverage;
    println!(
        "Documentation coverage: status={} files={} skipped-tests={} documented={}/{} missing={} unavailable={} ({})",
        documentation.status.as_str(),
        documentation.applicable_files,
        documentation.skipped_test_files,
        documentation.counts.documented,
        documentation.counts.measured(),
        documentation.counts.missing,
        documentation.counts.unavailable,
        format_documentation_percentage(documentation.counts.coverage_basis_points())
    );
    println!("\nInventory languages:");
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
            "  {} [{}]: analyzed={} successful={} partial={} failed={}",
            run.descriptor.language.as_str(),
            run.descriptor.id,
            run.counts.analyzed,
            run.counts.successful,
            run.counts.partial,
            run.counts.failed
        );
        for file in &run.files {
            if !file.diagnostics.is_empty() {
                println!(
                    "    diagnostics: {} ({})",
                    file.path.display(),
                    file.diagnostics.len()
                );
            }
        }
        let symbol_counts = run
            .files
            .iter()
            .flat_map(|file| file.facts.symbols.iter())
            .fold([0usize; 5], |mut counts, symbol| {
                match symbol.kind {
                    codegraide_core::SymbolKind::Module => counts[0] += 1,
                    codegraide_core::SymbolKind::Class => counts[1] += 1,
                    codegraide_core::SymbolKind::Function => counts[2] += 1,
                    codegraide_core::SymbolKind::Method => counts[3] += 1,
                    codegraide_core::SymbolKind::Lambda => counts[4] += 1,
                }
                counts
            });
        let import_count = run
            .files
            .iter()
            .map(|file| file.facts.dependencies.len())
            .sum::<usize>();
        println!(
            "    facts: modules={} classes={} functions={} methods={} lambdas={} imports={import_count}",
            symbol_counts[0],
            symbol_counts[1],
            symbol_counts[2],
            symbol_counts[3],
            symbol_counts[4]
        );
        print_top_measurements(
            run,
            "function-declaration-physical-lines",
            "longest declarations",
        );
        print_top_measurements(run, "python-max-control-flow-nesting", "deepest nesting");
        print_top_measurements(
            run,
            "python-cyclomatic-complexity",
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
            println!(
                "  {} {} {}: {}",
                finding.risk.as_str(),
                finding.required_action.as_str(),
                location,
                finding.message
            );
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

fn print_top_measurements(run: &codegraide_core::AnalyzerRun, metric_id: &str, label: &str) {
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
                    .find(|measurement| measurement.id == metric_id)
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
                println!(
                    "    import {}{}",
                    dependency.module.as_deref().unwrap_or("."),
                    dependency
                        .imported_name
                        .as_ref()
                        .map(|name| format!("::{name}"))
                        .unwrap_or_default()
                );
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
}
