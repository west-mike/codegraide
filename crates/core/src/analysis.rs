use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::analyzer::{
    AnalysisDiagnostic, AnalysisInput, AnalyzerDescriptor, AnalyzerRegistry, AnalyzerRegistryError,
    FileAnalysis, FileAnalysisStatus,
};
use crate::error::InventoryError;
use crate::inventory::{InventoryOptions, detect_language, inventory_repository_with_options};
use crate::report::{FileCategory, LanguageId};
use crate::review::{
    ReviewEvaluation, ReviewOptions, ReviewPolicy, ReviewPolicyError, evaluate_review,
};

#[derive(Debug, Clone)]
pub struct AnalysisOptions {
    pub target: PathBuf,
    pub match_patterns: Vec<String>,
    pub include_ignored: Vec<String>,
    pub review: ReviewOptions,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            target: PathBuf::from("."),
            match_patterns: Vec::new(),
            include_ignored: Vec::new(),
            review: ReviewOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AnalysisTargetKind {
    Directory,
    File,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AnalysisSelection {
    pub root: PathBuf,
    pub target_kind: AnalysisTargetKind,
    pub match_patterns: Vec<String>,
    pub selected_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct LanguageAnalysisCounts {
    pub analyzed: usize,
    pub successful: usize,
    pub partial: usize,
    pub failed: usize,
}

impl LanguageAnalysisCounts {
    fn record(&mut self, status: FileAnalysisStatus) {
        self.analyzed += 1;
        match status {
            FileAnalysisStatus::Successful => self.successful += 1,
            FileAnalysisStatus::Partial => self.partial += 1,
            FileAnalysisStatus::Failed => self.failed += 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnalyzerRun {
    pub descriptor: AnalyzerDescriptor,
    pub counts: LanguageAnalysisCounts,
    pub files: Vec<FileAnalysis>,
}

#[derive(Debug, Clone)]
pub struct RepositoryAnalysis {
    pub selection: AnalysisSelection,
    pub inventoried_files: usize,
    pub inventory_languages: BTreeMap<LanguageId, usize>,
    pub inventory_only_languages: BTreeMap<LanguageId, usize>,
    pub analyzers: Vec<AnalyzerRun>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
    pub review: ReviewEvaluation,
}

#[derive(Debug)]
pub enum AnalysisError {
    Io { context: String, source: io::Error },
    InvalidInput { message: String },
    Inventory(InventoryError),
    Registry(AnalyzerRegistryError),
    ReviewPolicy(ReviewPolicyError),
}

impl AnalysisError {
    fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::InvalidInput { message } => write!(formatter, "invalid input: {message}"),
            Self::Inventory(error) => write!(formatter, "inventory failed: {error}"),
            Self::Registry(error) => write!(formatter, "analyzer registry failed: {error}"),
            Self::ReviewPolicy(error) => write!(formatter, "review policy failed: {error}"),
        }
    }
}

impl std::error::Error for AnalysisError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Inventory(error) => Some(error),
            Self::Registry(error) => Some(error),
            Self::ReviewPolicy(error) => Some(error),
            Self::InvalidInput { .. } => None,
        }
    }
}

pub fn analyze_repository(
    options: &AnalysisOptions,
    registry: &mut AnalyzerRegistry,
) -> Result<RepositoryAnalysis, AnalysisError> {
    let target = fs::canonicalize(&options.target).map_err(|error| {
        AnalysisError::io(
            format!("cannot access analysis target {}", options.target.display()),
            error,
        )
    })?;
    let metadata = fs::metadata(&target).map_err(|error| {
        AnalysisError::io(
            format!("cannot inspect analysis target {}", target.display()),
            error,
        )
    })?;

    if metadata.is_dir() {
        analyze_directory(&target, options, registry)
    } else if metadata.is_file() {
        analyze_file(&target, options, registry)
    } else {
        Err(AnalysisError::invalid_input(format!(
            "analysis target {} is neither a regular file nor a directory",
            target.display()
        )))
    }
}

fn analyze_directory(
    root: &Path,
    options: &AnalysisOptions,
    registry: &mut AnalyzerRegistry,
) -> Result<RepositoryAnalysis, AnalysisError> {
    let inventory = inventory_repository_with_options(
        root,
        &InventoryOptions {
            include_ignored: options.include_ignored.clone(),
            ..InventoryOptions::default()
        },
    )
    .map_err(AnalysisError::Inventory)?;

    let matchers = compile_matchers(&options.match_patterns)?;
    let all_files = all_inventory_files(&inventory);
    let mut selected_files = all_files
        .into_iter()
        .filter(|path| {
            matchers.is_empty()
                || matchers
                    .iter()
                    .any(|matcher| matcher.is_match(&path_string(path)))
        })
        .collect::<Vec<_>>();
    selected_files.sort();

    if !matchers.is_empty() && selected_files.is_empty() {
        return Err(AnalysisError::invalid_input(
            "--match did not select any inventoried files",
        ));
    }

    let selection = AnalysisSelection {
        root: root.to_path_buf(),
        target_kind: AnalysisTargetKind::Directory,
        match_patterns: options.match_patterns.clone(),
        selected_files: selected_files.clone(),
    };
    let inventory_languages = inventory.files_by_language.clone();
    let mut sources = Vec::new();
    for relative_path in selected_files {
        let Some(language) = detect_language(&relative_path) else {
            continue;
        };
        if !registry.analyzer_for(&language) {
            continue;
        }
        let path = root.join(&relative_path);
        let source = fs::read(&path).map_err(|error| {
            AnalysisError::io(
                format!("cannot read selected file {}", path.display()),
                error,
            )
        })?;
        sources.push((relative_path, language, source));
    }

    let mut result = run_analyzers(
        selection,
        inventory.inventoried_files,
        inventory_languages,
        sources,
        registry,
    );
    if result
        .analyzers
        .iter()
        .all(|analyzer| analyzer.counts.analyzed == 0)
    {
        result.diagnostics.push(AnalysisDiagnostic {
            severity: crate::analyzer::DiagnosticSeverity::Warning,
            code: "no-supported-files".to_owned(),
            message: "no selected files have a registered syntax analyzer".to_owned(),
            span: None,
        });
    }
    finalize_analysis(&mut result, &options.review).map_err(AnalysisError::ReviewPolicy)?;
    Ok(result)
}

fn analyze_file(
    path: &Path,
    options: &AnalysisOptions,
    registry: &mut AnalyzerRegistry,
) -> Result<RepositoryAnalysis, AnalysisError> {
    if !options.match_patterns.is_empty() {
        return Err(AnalysisError::invalid_input(
            "--match can only be used when the analysis target is a directory",
        ));
    }
    if options
        .include_ignored
        .iter()
        .any(|pattern| !pattern.is_empty())
    {
        return Err(AnalysisError::invalid_input(
            "--include-ignored can only be used when the analysis target is a directory",
        ));
    }
    if path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(is_builtin_directory_name)
    {
        return Err(AnalysisError::invalid_input(format!(
            "explicit file {} is inside a built-in excluded directory",
            path.display()
        )));
    }

    let language = detect_language(path).ok_or_else(|| {
        AnalysisError::invalid_input(format!(
            "explicit file {} has no supported language",
            path.display()
        ))
    })?;
    if !registry.analyzer_for(&language) {
        return Err(AnalysisError::invalid_input(format!(
            "no analyzer is registered for {} files",
            language.as_str()
        )));
    }

    let root = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let relative_path = path
        .strip_prefix(&root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf());
    let source = fs::read(path).map_err(|error| {
        AnalysisError::io(
            format!("cannot read selected file {}", path.display()),
            error,
        )
    })?;
    let selection = AnalysisSelection {
        root,
        target_kind: AnalysisTargetKind::File,
        match_patterns: Vec::new(),
        selected_files: vec![relative_path.clone()],
    };

    let mut result = run_analyzers(
        selection,
        1,
        [(language.clone(), 1)].into_iter().collect(),
        vec![(relative_path, language, source)],
        registry,
    );
    finalize_analysis(&mut result, &options.review).map_err(AnalysisError::ReviewPolicy)?;
    Ok(result)
}

fn finalize_analysis(
    analysis: &mut RepositoryAnalysis,
    options: &ReviewOptions,
) -> Result<(), ReviewPolicyError> {
    let policy = ReviewPolicy::resolve(options)?;
    analysis.review = evaluate_review(
        &analysis.selection.selected_files,
        &analysis.analyzers,
        policy,
    );
    Ok(())
}

fn run_analyzers(
    selection: AnalysisSelection,
    inventoried_files: usize,
    inventory_languages: BTreeMap<LanguageId, usize>,
    sources: Vec<(PathBuf, LanguageId, Vec<u8>)>,
    registry: &mut AnalyzerRegistry,
) -> RepositoryAnalysis {
    let inventory_only_languages = inventory_languages
        .iter()
        .filter(|(language, _)| !registry.analyzer_for(language))
        .map(|(language, count)| (language.clone(), *count))
        .collect();

    let descriptors = registry.descriptors().cloned().collect::<Vec<_>>();
    let mut grouped = descriptors
        .into_iter()
        .map(|descriptor| {
            (
                descriptor.language.clone(),
                AnalyzerRun {
                    descriptor,
                    counts: LanguageAnalysisCounts::default(),
                    files: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (path, language, source) in sources {
        let Some(file) = registry.analyze(
            &language,
            AnalysisInput {
                path: &path,
                source: &source,
            },
        ) else {
            continue;
        };
        let run = grouped
            .get_mut(&language)
            .expect("analyzer descriptor exists after analysis");
        run.counts.record(file.status);
        run.files.push(file);
    }

    let mut analyzers = grouped.into_values().collect::<Vec<_>>();
    for run in &mut analyzers {
        run.files.sort_by(|left, right| left.path.cmp(&right.path));
    }

    RepositoryAnalysis {
        selection,
        inventoried_files,
        inventory_languages,
        inventory_only_languages,
        analyzers,
        diagnostics: Vec::new(),
        review: ReviewEvaluation {
            status: crate::review::ReviewStatus::HumanReviewRequired,
            policy: ReviewPolicy::resolve(&ReviewOptions::default())
                .expect("built-in review policy is valid"),
            coverage: crate::review::ReviewCoverage {
                selected_language_files: 0,
                unsupported_selected_files: 0,
                analyzed_files: 0,
                successful_files: 0,
                partial_files: 0,
                failed_files: 0,
                eligible_callables: 0,
                measured_callables: 0,
                unavailable_callables: 0,
            },
            rankings: Vec::new(),
            findings: Vec::new(),
        },
    }
}

fn all_inventory_files(inventory: &crate::report::RepositoryInventory) -> Vec<PathBuf> {
    let mut files = BTreeSet::new();
    for category in FileCategory::ALL {
        files.extend(inventory.category_files(category).iter().cloned());
    }
    files.into_iter().collect()
}

fn compile_matchers(patterns: &[String]) -> Result<Vec<Regex>, AnalysisError> {
    patterns
        .iter()
        .map(|pattern| {
            if pattern.is_empty() {
                return Err(AnalysisError::invalid_input(
                    "--match patterns cannot be empty",
                ));
            }
            Regex::new(&format!("^(?:{pattern})$")).map_err(|error| {
                AnalysisError::invalid_input(format!("invalid --match regex {pattern:?}: {error}"))
            })
        })
        .collect()
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_builtin_directory_name(value: &std::ffi::OsStr) -> bool {
    matches!(value.to_str(), Some(".git" | "target" | "__pycache__"))
}
