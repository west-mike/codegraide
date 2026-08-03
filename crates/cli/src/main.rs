use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

use codegraide_analyzer_python::PythonAnalyzer;
use codegraide_core::{
    AnalysisJsonReport, AnalysisOptions, AnalyzerRegistry, FileCategory, GateJsonReport,
    InventoryJsonReport, InventoryOptions, RepositoryAnalysis, RepositoryInventory,
    ReviewJsonReport, ReviewOptions, ReviewStatus, analyze_repository,
    inventory_repository_with_options, review_status_code,
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
            gate: *gate,
            profile: *profile,
            top: *top,
            format: *format,
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
            target: path.to_path_buf(),
            match_patterns: match_patterns.to_vec(),
            include_ignored: include_ignored.to_vec(),
            review: ReviewOptions {
                policy_path: policy.map(Path::to_path_buf),
                complexity_review_at,
                complexity_block_at,
                no_complexity_block,
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
