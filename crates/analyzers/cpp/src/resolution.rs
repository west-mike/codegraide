use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use codegraide_core::{
    DependencyReference, DependencyResolutionContextCoverage, DependencyResolutionOutcome,
    DependencyResolver, DependencyResolverDescriptor, DependencyResolverError, DependencyTarget,
    DependencyUnitKind, IncludeDelimiter, InferredDependencyBasis, LanguageId, LocalModule,
    ModuleId, ProjectDependencyResolution, RepositoryAnalysis, ResolvedProjectDependencies,
    UnresolvedDependencyReason,
};
use serde::Deserialize;

pub const CPP_HEADER_RESOLUTION_DEFINITION_VERSION: &str = "cpp-header-resolution-v1";

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct CppResolutionOptions {
    pub compilation_database: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct CppResolutionSummary {
    pub compilation_database_selected: bool,
    pub total_commands: usize,
    pub supported_commands: usize,
    pub unsupported_commands: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CppDependencyResolution {
    pub local_modules: Vec<LocalModule>,
    pub resolutions: Vec<ProjectDependencyResolution>,
    pub summary: CppResolutionSummary,
    pub diagnostics: Vec<String>,
}

pub struct CppDependencyResolver {
    descriptor: DependencyResolverDescriptor,
    options: CppResolutionOptions,
}

impl CppDependencyResolver {
    pub fn new(options: CppResolutionOptions) -> Self {
        Self {
            descriptor: DependencyResolverDescriptor {
                id: "cpp-header-resolver".to_owned(),
                language: LanguageId::new("cpp"),
                version: "0.1.0".to_owned(),
                definition_version: CPP_HEADER_RESOLUTION_DEFINITION_VERSION.to_owned(),
                local_unit_kind: DependencyUnitKind::File,
                hierarchy_behavior: "repository-directory-segments".to_owned(),
                resolution_capabilities: vec![
                    "same-directory-quoted-include".to_owned(),
                    "standard-library-header-classification".to_owned(),
                    "unique-repository-suffix-inference".to_owned(),
                    "gcc-clang-compilation-database".to_owned(),
                    "context-propagation-through-local-headers".to_owned(),
                    "sysroot-substitution".to_owned(),
                ],
                limitations: vec![
                    "Without a compilation database, only same-directory quoted includes are exact; unique repository suffix matches are labeled inferred and ambiguous suffixes remain unresolved. GCC/Clang include flags are supported; compiler execution, MSVC flags, response files, macro expansion, and implicit system paths are not."
                        .to_owned(),
                ],
            },
            options,
        }
    }
}

impl DependencyResolver for CppDependencyResolver {
    fn descriptor(&self) -> &DependencyResolverDescriptor {
        &self.descriptor
    }

    fn resolve(
        &self,
        analysis: &RepositoryAnalysis,
    ) -> Result<ResolvedProjectDependencies, DependencyResolverError> {
        let resolution = resolve_cpp_dependencies(analysis, &self.options)
            .map_err(|error| DependencyResolverError::new(error.to_string()))?;
        let summary = resolution.summary;
        Ok(ResolvedProjectDependencies {
            local_units: resolution.local_modules,
            resolutions: resolution.resolutions,
            context_coverage: vec![DependencyResolutionContextCoverage {
                kind: "compilation-database".to_owned(),
                selected: summary.compilation_database_selected,
                total: summary.total_commands,
                supported: summary.supported_commands,
                unsupported: summary.unsupported_commands,
            }],
            summary_lines: vec![format!(
                "Compilation database: selected={}, commands={}, supported={}, unsupported={}",
                summary.compilation_database_selected,
                summary.total_commands,
                summary.supported_commands,
                summary.unsupported_commands
            )],
            metadata: BTreeMap::new(),
            diagnostics: resolution.diagnostics,
        })
    }
}

#[derive(Debug)]
pub enum CppResolutionError {
    InvalidProject(String),
    Io { context: String, source: io::Error },
    InvalidCompilationDatabase(String),
}

impl fmt::Display for CppResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProject(message) | Self::InvalidCompilationDatabase(message) => {
                formatter.write_str(message)
            }
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
        }
    }
}

impl std::error::Error for CppResolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CompilationDatabaseEntry {
    directory: PathBuf,
    file: PathBuf,
    arguments: Option<Vec<String>>,
    command: Option<String>,
    #[allow(dead_code)]
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SearchKind {
    Quote,
    Include,
    System,
    After,
}

#[derive(Debug, Clone)]
struct SearchDirectory {
    kind: SearchKind,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct CompileContext {
    source: PathBuf,
    search: Vec<SearchDirectory>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum ContextResolution {
    Target(DependencyTarget),
    Inferred(DependencyTarget),
    Unresolved(UnresolvedDependencyReason),
}

pub fn resolve_cpp_dependencies(
    analysis: &RepositoryAnalysis,
    options: &CppResolutionOptions,
) -> Result<CppDependencyResolution, CppResolutionError> {
    resolve_dependencies(analysis, options, false)
}

/// Resolve committed include facts without filesystem or compilation-database access.
pub fn resolve_cpp_snapshot_dependencies(
    analysis: &RepositoryAnalysis,
) -> Result<CppDependencyResolution, CppResolutionError> {
    resolve_dependencies(analysis, &CppResolutionOptions::default(), true)
}

fn resolve_dependencies(
    analysis: &RepositoryAnalysis,
    options: &CppResolutionOptions,
    snapshot: bool,
) -> Result<CppDependencyResolution, CppResolutionError> {
    if !matches!(
        analysis.selection.target_kind,
        codegraide_core::AnalysisTargetKind::Directory
    ) {
        return Err(CppResolutionError::InvalidProject(
            "C++ dependency analysis requires a project directory".to_owned(),
        ));
    }

    let root = &analysis.selection.root;
    let repository_files = analysis.selection.selected_files.clone();
    let cpp_files = analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
        .flat_map(|run| run.files.iter())
        .map(|file| (file.path.clone(), file.facts.dependencies.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut catalog = cpp_files
        .keys()
        .cloned()
        .map(|path| (path.clone(), local_file(path, true)))
        .collect::<BTreeMap<_, _>>();

    let (contexts, summary, mut diagnostics) = match &options.compilation_database {
        Some(path) => load_compilation_database(path, root)?,
        None => (
            Vec::new(),
            CppResolutionSummary::default(),
            vec![
                "C++ compilation database not selected; exact resolution is limited to same-directory quoted includes, while unique repository suffixes are shown as inferred local relations"
                    .to_owned(),
            ],
        ),
    };

    let mut context_by_file = BTreeMap::<PathBuf, BTreeSet<usize>>::new();
    let mut queue = VecDeque::new();
    for (index, context) in contexts.iter().enumerate() {
        if cpp_files.contains_key(&context.source)
            && context_by_file
                .entry(context.source.clone())
                .or_default()
                .insert(index)
        {
            queue.push_back((context.source.clone(), index));
        }
    }

    while let Some((source_path, context_index)) = queue.pop_front() {
        let Some(references) = cpp_files.get(&source_path) else {
            continue;
        };
        for reference in references
            .iter()
            .filter_map(DependencyReference::as_include)
        {
            let ContextResolution::Target(DependencyTarget::LocalModule(target)) = resolve_include(
                root,
                &source_path,
                reference,
                Some(&contexts[context_index]),
                &repository_files,
                &mut catalog,
                snapshot,
            ) else {
                continue;
            };
            if cpp_files.contains_key(&target.path)
                && context_by_file
                    .entry(target.path.clone())
                    .or_default()
                    .insert(context_index)
            {
                queue.push_back((target.path, context_index));
            }
        }
    }

    let mut resolutions = Vec::new();
    for (source_path, references) in &cpp_files {
        let source = catalog
            .get(source_path)
            .expect("every analyzed C++ file is in the dependency catalog")
            .clone();
        for reference in references {
            let Some(include) = reference.as_include() else {
                continue;
            };
            let context_ids = context_by_file.get(source_path);
            let outcomes = match context_ids {
                Some(ids) if !ids.is_empty() => ids
                    .iter()
                    .map(|index| {
                        resolve_include(
                            root,
                            source_path,
                            include,
                            Some(&contexts[*index]),
                            &repository_files,
                            &mut catalog,
                            snapshot,
                        )
                    })
                    .collect::<Vec<_>>(),
                _ => vec![resolve_include(
                    root,
                    source_path,
                    include,
                    None,
                    &repository_files,
                    &mut catalog,
                    snapshot,
                )],
            };
            resolutions.push(ProjectDependencyResolution::new(
                source_path.clone(),
                source.id.clone(),
                reference.clone(),
                aggregate_outcomes(&include.target, outcomes),
            ));
        }
    }
    resolutions.sort_by(|left, right| {
        left.source_path.cmp(&right.source_path).then_with(|| {
            left.reference
                .span()
                .start_byte
                .cmp(&right.reference.span().start_byte)
        })
    });
    diagnostics.sort();
    diagnostics.dedup();

    Ok(CppDependencyResolution {
        local_modules: catalog.into_values().collect(),
        resolutions,
        summary,
        diagnostics,
    })
}

fn load_compilation_database(
    path: &Path,
    root: &Path,
) -> Result<(Vec<CompileContext>, CppResolutionSummary, Vec<String>), CppResolutionError> {
    let database_path = fs::canonicalize(path).map_err(|source| CppResolutionError::Io {
        context: format!("cannot access compilation database {}", path.display()),
        source,
    })?;
    let source = fs::read_to_string(&database_path).map_err(|source| CppResolutionError::Io {
        context: format!("cannot read compilation database {}", path.display()),
        source,
    })?;
    let entries =
        serde_json::from_str::<Vec<CompilationDatabaseEntry>>(&source).map_err(|error| {
            CppResolutionError::InvalidCompilationDatabase(format!(
                "cannot parse compilation database {}: {error}",
                path.display()
            ))
        })?;
    let database_directory = database_path
        .parent()
        .expect("a canonical file has a parent directory");
    let mut contexts = Vec::new();
    let mut diagnostics = Vec::new();
    let mut summary = CppResolutionSummary {
        compilation_database_selected: true,
        total_commands: entries.len(),
        ..CppResolutionSummary::default()
    };

    for (index, entry) in entries.into_iter().enumerate() {
        match compile_context(entry, database_directory, root) {
            Ok(Some(context)) => {
                summary.supported_commands += 1;
                contexts.push(context);
            }
            Ok(None) => {}
            Err(message) => {
                summary.unsupported_commands += 1;
                diagnostics.push(format!(
                    "compilation command {} is unsupported: {message}",
                    index + 1
                ));
            }
        }
    }
    contexts.sort_by(|left, right| left.source.cmp(&right.source));
    Ok((contexts, summary, diagnostics))
}

fn compile_context(
    entry: CompilationDatabaseEntry,
    database_directory: &Path,
    root: &Path,
) -> Result<Option<CompileContext>, String> {
    let directory = if entry.directory.is_absolute() {
        entry.directory
    } else {
        database_directory.join(entry.directory)
    };
    let source = if entry.file.is_absolute() {
        entry.file
    } else {
        directory.join(entry.file)
    };
    let source = fs::canonicalize(&source)
        .map_err(|error| format!("source file is unavailable: {error}"))?;
    let Ok(source) = source.strip_prefix(root) else {
        return Ok(None);
    };
    let arguments = match (entry.arguments, entry.command) {
        (Some(arguments), _) => arguments,
        (None, Some(command)) => split_command(&command)?,
        (None, None) => return Err("missing both arguments and command".to_owned()),
    };
    if arguments.iter().any(|argument| argument.starts_with('@')) {
        return Err("response files are not supported".to_owned());
    }
    if arguments.iter().any(|argument| {
        argument == "/I" || argument.starts_with("/I") || argument.starts_with("/external:I")
    }) {
        return Err("MSVC include flags are not supported".to_owned());
    }
    let sysroot = option_value(&arguments, "--sysroot")
        .or_else(|| option_value(&arguments, "-isysroot"))
        .map(|value| absolute_search_path(&directory, Path::new(&value), None));
    let mut search = Vec::new();
    collect_search_paths(
        &arguments,
        "-iquote",
        SearchKind::Quote,
        &directory,
        sysroot.as_deref(),
        &mut search,
    );
    collect_search_paths(
        &arguments,
        "-I",
        SearchKind::Include,
        &directory,
        sysroot.as_deref(),
        &mut search,
    );
    collect_search_paths(
        &arguments,
        "-isystem",
        SearchKind::System,
        &directory,
        sysroot.as_deref(),
        &mut search,
    );
    collect_search_paths(
        &arguments,
        "-idirafter",
        SearchKind::After,
        &directory,
        sysroot.as_deref(),
        &mut search,
    );
    Ok(Some(CompileContext {
        source: source.to_path_buf(),
        search,
    }))
}

fn option_value(arguments: &[String], option: &str) -> Option<String> {
    for (index, argument) in arguments.iter().enumerate() {
        if argument == option {
            return arguments.get(index + 1).cloned();
        }
        if let Some(value) = argument.strip_prefix(&format!("{option}=")) {
            return Some(value.to_owned());
        }
    }
    None
}

fn collect_search_paths(
    arguments: &[String],
    option: &str,
    kind: SearchKind,
    directory: &Path,
    sysroot: Option<&Path>,
    output: &mut Vec<SearchDirectory>,
) {
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        let value = if argument == option {
            index += 1;
            arguments.get(index).cloned()
        } else {
            argument
                .strip_prefix(option)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        if let Some(value) = value {
            output.push(SearchDirectory {
                kind,
                path: absolute_search_path(directory, Path::new(&value), sysroot),
            });
        }
        index += 1;
    }
}

fn absolute_search_path(directory: &Path, path: &Path, sysroot: Option<&Path>) -> PathBuf {
    if let Some(value) = path.to_str().and_then(|value| value.strip_prefix('=')) {
        return sysroot
            .unwrap_or(directory)
            .join(value.trim_start_matches('/'));
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        directory.join(path)
    }
}

fn split_command(command: &str) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                arguments.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if escaped || quote.is_some() {
        return Err("command contains unterminated quoting or escaping".to_owned());
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    Ok(arguments)
}

fn resolve_include(
    root: &Path,
    source_path: &Path,
    include: &codegraide_core::IncludeReference,
    context: Option<&CompileContext>,
    repository_files: &[PathBuf],
    catalog: &mut BTreeMap<PathBuf, LocalModule>,
    snapshot: bool,
) -> ContextResolution {
    if include.delimiter == IncludeDelimiter::Macro {
        return ContextResolution::Unresolved(UnresolvedDependencyReason::MacroInclude);
    }
    let source_directory = root
        .join(source_path)
        .parent()
        .expect("an analyzed file has a parent directory")
        .to_path_buf();
    let mut search = Vec::<(PathBuf, SearchKind)>::new();
    if include.delimiter == IncludeDelimiter::Quote {
        search.push((source_directory, SearchKind::Quote));
    }
    if let Some(context) = context {
        for kind in [
            SearchKind::Quote,
            SearchKind::Include,
            SearchKind::System,
            SearchKind::After,
        ] {
            if kind == SearchKind::Quote && include.delimiter != IncludeDelimiter::Quote {
                continue;
            }
            search.extend(
                context
                    .search
                    .iter()
                    .filter(|directory| directory.kind == kind)
                    .map(|directory| (directory.path.clone(), kind)),
            );
        }
    }
    if snapshot && include.delimiter == IncludeDelimiter::Quote {
        let relative = source_path
            .parent()
            .unwrap_or(Path::new(""))
            .join(&include.target);
        let mut normalized = PathBuf::new();
        let valid = relative.components().all(|part| match part {
            std::path::Component::Normal(name) => {
                normalized.push(name);
                true
            }
            std::path::Component::CurDir => true,
            std::path::Component::ParentDir => normalized.pop(),
            _ => false,
        });
        if valid && repository_files.contains(&normalized) {
            let module = catalog
                .entry(normalized.clone())
                .or_insert_with(|| local_file(normalized, false))
                .clone();
            return ContextResolution::Target(DependencyTarget::LocalModule(module));
        }
    }
    for (directory, kind) in search {
        if snapshot {
            continue;
        }
        let candidate = directory.join(&include.target);
        if !candidate.is_file() {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(&candidate) else {
            continue;
        };
        if let Ok(relative) = canonical.strip_prefix(root) {
            let relative = relative.to_path_buf();
            let module = catalog
                .entry(relative.clone())
                .or_insert_with(|| local_file(relative, false))
                .clone();
            return ContextResolution::Target(DependencyTarget::LocalModule(module));
        }
        let language = LanguageId::new("cpp");
        return ContextResolution::Target(
            if matches!(kind, SearchKind::System | SearchKind::After) {
                DependencyTarget::SystemHeader {
                    language,
                    name: include.target.clone(),
                }
            } else {
                DependencyTarget::ExternalHeader {
                    language,
                    name: include.target.clone(),
                }
            },
        );
    }
    if context.is_none() {
        if include.delimiter == IncludeDelimiter::Angle && is_cpp_standard_header(&include.target) {
            return ContextResolution::Target(DependencyTarget::SystemHeader {
                language: LanguageId::new("cpp"),
                name: include.target.clone(),
            });
        }
        let suffix = Path::new(&include.target);
        if !suffix.is_absolute()
            && !suffix
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            let mut matches = repository_files
                .iter()
                .filter(|path| path.ends_with(suffix))
                .cloned()
                .collect::<Vec<_>>();
            matches.sort();
            matches.dedup();
            if let [relative] = matches.as_slice() {
                let module = catalog
                    .entry(relative.clone())
                    .or_insert_with(|| local_file(relative.clone(), false))
                    .clone();
                return ContextResolution::Inferred(DependencyTarget::LocalModule(module));
            }
        }
    }
    ContextResolution::Unresolved(if context.is_none() {
        UnresolvedDependencyReason::BuildContextUnavailable
    } else if include.delimiter == IncludeDelimiter::Angle {
        UnresolvedDependencyReason::ImplicitSearchPathUnavailable
    } else {
        UnresolvedDependencyReason::HeaderNotFound
    })
}

fn aggregate_outcomes(
    requested: &str,
    outcomes: Vec<ContextResolution>,
) -> DependencyResolutionOutcome {
    if let [ContextResolution::Inferred(target)] = outcomes.as_slice() {
        return DependencyResolutionOutcome::inferred(
            target.clone(),
            InferredDependencyBasis::UniqueRepositorySuffix,
        );
    }
    let mut targets = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            ContextResolution::Target(target) | ContextResolution::Inferred(target) => {
                Some(target.clone())
            }
            ContextResolution::Unresolved(_) => None,
        })
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    let mut reasons = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            ContextResolution::Unresolved(reason) => Some(*reason),
            ContextResolution::Target(_) | ContextResolution::Inferred(_) => None,
        })
        .collect::<Vec<_>>();
    reasons.sort();
    reasons.dedup();
    match (targets.as_slice(), reasons.as_slice()) {
        ([target], []) => DependencyResolutionOutcome::exact(target.clone()),
        ([], [reason]) => DependencyResolutionOutcome::unresolved(requested, *reason),
        _ => DependencyResolutionOutcome::context_dependent(requested, targets, reasons),
    }
}

fn is_cpp_standard_header(target: &str) -> bool {
    matches!(
        target,
        "algorithm"
            | "any"
            | "array"
            | "atomic"
            | "bit"
            | "bitset"
            | "cassert"
            | "cctype"
            | "cerrno"
            | "charconv"
            | "chrono"
            | "cinttypes"
            | "climits"
            | "cmath"
            | "compare"
            | "complex"
            | "concepts"
            | "condition_variable"
            | "coroutine"
            | "cstddef"
            | "cstdint"
            | "cstdio"
            | "cstdlib"
            | "cstring"
            | "ctime"
            | "deque"
            | "exception"
            | "execution"
            | "expected"
            | "filesystem"
            | "format"
            | "forward_list"
            | "fstream"
            | "functional"
            | "future"
            | "initializer_list"
            | "iomanip"
            | "ios"
            | "iosfwd"
            | "iostream"
            | "istream"
            | "iterator"
            | "limits"
            | "list"
            | "locale"
            | "map"
            | "memory"
            | "mutex"
            | "new"
            | "numbers"
            | "numeric"
            | "optional"
            | "ostream"
            | "queue"
            | "random"
            | "ranges"
            | "ratio"
            | "regex"
            | "set"
            | "shared_mutex"
            | "source_location"
            | "span"
            | "sstream"
            | "stack"
            | "stdexcept"
            | "streambuf"
            | "string"
            | "string_view"
            | "system_error"
            | "thread"
            | "tuple"
            | "type_traits"
            | "typeindex"
            | "typeinfo"
            | "unordered_map"
            | "unordered_set"
            | "utility"
            | "valarray"
            | "variant"
            | "vector"
            | "version"
    )
}

fn local_file(path: PathBuf, outgoing_dependencies_analyzed: bool) -> LocalModule {
    let name = path.to_string_lossy().replace('\\', "/");
    LocalModule::new(ModuleId::new(LanguageId::new("cpp"), name), path)
        .with_outgoing_dependencies_analyzed(outgoing_dependencies_analyzed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_standard_command_strings_without_expansion() {
        assert_eq!(
            split_command("clang++ -I\"include dir\" -DVALUE=\\\"hello\\\" file.cpp").unwrap(),
            ["clang++", "-Iinclude dir", "-DVALUE=\"hello\"", "file.cpp"]
        );
    }

    #[test]
    fn aggregates_divergent_contexts_as_context_dependent() {
        let first =
            DependencyTarget::LocalModule(local_file(PathBuf::from("debug/config.h"), true));
        let second =
            DependencyTarget::LocalModule(local_file(PathBuf::from("release/config.h"), true));
        let outcome = aggregate_outcomes(
            "config.h",
            vec![
                ContextResolution::Target(first.clone()),
                ContextResolution::Target(second.clone()),
            ],
        );
        assert_eq!(
            outcome,
            DependencyResolutionOutcome::context_dependent(
                "config.h",
                vec![first, second],
                Vec::new()
            )
        );
    }

    #[test]
    fn recognizes_representative_standard_headers_without_matching_project_files() {
        assert!(is_cpp_standard_header("algorithm"));
        assert!(is_cpp_standard_header("string_view"));
        assert!(!is_cpp_standard_header("argparse/argparse.hpp"));
        assert!(!is_cpp_standard_header("doctest.hpp"));
    }
}
