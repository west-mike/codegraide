use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use codegraide_core::{
    DependencyReference, DependencyResolutionContextCoverage, DependencyResolutionOutcome,
    DependencyResolver, DependencyResolverDescriptor, DependencyResolverError, DependencyTarget,
    DependencyUnitKind, ImportReference, LanguageId, LocalModule, ModuleId,
    ProjectDependencyResolution, RepositoryAnalysis, ResolvedProjectDependencies,
    UnresolvedDependencyReason,
};
use serde::Deserialize;

const PROBE_SCHEMA_VERSION: &str = "codegraide-python-environment-v1";
pub const PYTHON_IMPORT_RESOLUTION_DEFINITION_VERSION: &str = "python-import-resolution-v1";
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const PYTHON_PROBE: &str = include_str!("environment_probe.txt");

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PythonEnvironmentSelection {
    Interpreter(PathBuf),
    VirtualEnvironment(PathBuf),
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct PythonResolutionOptions {
    pub environment: Option<PythonEnvironmentSelection>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PythonEnvironmentSummary {
    pub selection_kind: &'static str,
    pub executable: PathBuf,
    pub implementation: String,
    pub version: String,
    pub is_virtual_environment: bool,
    pub distribution_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PythonDependencyResolution {
    pub package_roots: Vec<PathBuf>,
    pub local_modules: Vec<LocalModule>,
    pub resolutions: Vec<ProjectDependencyResolution>,
    pub environment: Option<PythonEnvironmentSummary>,
    pub diagnostics: Vec<String>,
}

pub struct PythonDependencyResolver {
    descriptor: DependencyResolverDescriptor,
    options: PythonResolutionOptions,
}

impl PythonDependencyResolver {
    pub fn new(options: PythonResolutionOptions) -> Self {
        Self {
            descriptor: DependencyResolverDescriptor {
                id: "python-import-resolver".to_owned(),
                language: LanguageId::new("python"),
                version: "0.1.0".to_owned(),
                definition_version: PYTHON_IMPORT_RESOLUTION_DEFINITION_VERSION.to_owned(),
                local_unit_kind: DependencyUnitKind::Module,
                hierarchy_behavior: "qualified-module-segments".to_owned(),
                resolution_capabilities: vec![
                    "repository-package-roots".to_owned(),
                    "relative-imports".to_owned(),
                    "standard-library-probe".to_owned(),
                    "installed-distribution-probe".to_owned(),
                ],
                limitations: vec![
                    "Dynamic imports, runtime import hooks, and framework re-exports are not resolved."
                        .to_owned(),
                ],
            },
            options,
        }
    }
}

impl DependencyResolver for PythonDependencyResolver {
    fn descriptor(&self) -> &DependencyResolverDescriptor {
        &self.descriptor
    }

    fn resolve(
        &self,
        analysis: &RepositoryAnalysis,
    ) -> Result<ResolvedProjectDependencies, DependencyResolverError> {
        let resolution = resolve_python_dependencies(analysis, &self.options)
            .map_err(|error| DependencyResolverError::new(error.to_string()))?;
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "package-roots".to_owned(),
            resolution
                .package_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
        if let Some(environment) = &resolution.environment {
            metadata.insert(
                "environment-selection".to_owned(),
                environment.selection_kind.to_owned(),
            );
            metadata.insert(
                "environment-implementation".to_owned(),
                environment.implementation.clone(),
            );
            metadata.insert(
                "environment-version".to_owned(),
                environment.version.clone(),
            );
            metadata.insert(
                "environment-is-virtual".to_owned(),
                environment.is_virtual_environment.to_string(),
            );
            metadata.insert(
                "environment-distributions".to_owned(),
                environment.distribution_count.to_string(),
            );
        }
        let summary_lines = vec![
            format!("Package roots: {}", metadata["package-roots"]),
            match &resolution.environment {
                Some(environment) => format!(
                    "Python environment: {} {} ({}, packages={})",
                    environment.implementation,
                    environment.version,
                    environment.selection_kind,
                    environment.distribution_count
                ),
                None => "Python environment: not selected (external imports remain unresolved)"
                    .to_owned(),
            },
        ];
        Ok(ResolvedProjectDependencies {
            local_units: resolution.local_modules,
            resolutions: resolution.resolutions,
            context_coverage: vec![DependencyResolutionContextCoverage {
                kind: "python-environment".to_owned(),
                selected: resolution.environment.is_some(),
                total: usize::from(resolution.environment.is_some()),
                supported: usize::from(resolution.environment.is_some()),
                unsupported: 0,
            }],
            summary_lines,
            metadata,
            diagnostics: resolution.diagnostics,
        })
    }
}

#[derive(Debug)]
pub enum PythonResolutionError {
    InvalidProject(String),
    Io { context: String, source: io::Error },
    InvalidEnvironment(String),
    Probe(String),
}

impl fmt::Display for PythonResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProject(message) | Self::InvalidEnvironment(message) => {
                formatter.write_str(message)
            }
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::Probe(message) => write!(formatter, "Python environment probe failed: {message}"),
        }
    }
}

impl std::error::Error for PythonResolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct InstalledDistribution {
    normalized_name: String,
    display_name: String,
    version: Option<String>,
}

#[derive(Debug, Clone)]
struct PythonEnvironment {
    summary: PythonEnvironmentSummary,
    stdlib_names: BTreeSet<String>,
    import_providers: BTreeMap<String, Vec<InstalledDistribution>>,
}

#[derive(Debug, Deserialize)]
struct ProbePayload {
    schema_version: String,
    implementation: String,
    version: [u64; 3],
    is_virtual_environment: bool,
    stdlib_names: Vec<String>,
    distributions: Vec<ProbeDistribution>,
}

#[derive(Debug, Deserialize)]
struct ProbeDistribution {
    normalized_name: String,
    display_name: String,
    version: Option<String>,
    import_names: Vec<String>,
}

pub fn resolve_python_dependencies(
    analysis: &RepositoryAnalysis,
    options: &PythonResolutionOptions,
) -> Result<PythonDependencyResolution, PythonResolutionError> {
    if !matches!(
        analysis.selection.target_kind,
        codegraide_core::AnalysisTargetKind::Directory
    ) {
        return Err(PythonResolutionError::InvalidProject(
            "dependency analysis requires a project directory".to_owned(),
        ));
    }
    let package_roots = discover_package_roots(&analysis.selection.root)?;
    let environment = options
        .environment
        .as_ref()
        .map(probe_environment)
        .transpose()?;
    Ok(resolve_with_context(analysis, package_roots, environment))
}

/// Resolve only committed package metadata; never inspect the working directory.
pub fn resolve_python_snapshot_dependencies(
    analysis: &RepositoryAnalysis,
    git: &codegraide_core::git_snapshot::GitSnapshot,
) -> Result<PythonDependencyResolution, PythonResolutionError> {
    let metadata = git.files.get(Path::new("pyproject.toml"));
    if metadata.is_none() && git.entries.contains_key("pyproject.toml") {
        return Err(PythonResolutionError::InvalidProject(
            "committed pyproject.toml is unreadable".into(),
        ));
    }
    let roots = package_roots(metadata.map(|file| file.source.as_str()), |path| {
        path == Path::new(".")
            || git.entries.keys().any(|entry| {
                Path::new(entry)
                    .strip_prefix(path)
                    .is_ok_and(|suffix| !suffix.as_os_str().is_empty())
            })
    })?;
    Ok(resolve_with_context(analysis, roots, None))
}

fn resolve_with_context(
    analysis: &RepositoryAnalysis,
    package_roots: Vec<PathBuf>,
    environment: Option<PythonEnvironment>,
) -> PythonDependencyResolution {
    let (local_modules, by_name, diagnostics) = build_module_catalog(analysis, &package_roots);
    let mut resolutions = Vec::new();
    for run in &analysis.analyzers {
        if run.descriptor.language.as_str() != "python" {
            continue;
        }
        for file in &run.files {
            let Some(source) = local_modules.iter().find(|module| module.path == file.path) else {
                continue;
            };
            for reference in &file.facts.dependencies {
                let Some(import) = reference.as_import() else {
                    continue;
                };
                resolutions.push(resolve_reference(
                    source,
                    reference,
                    import,
                    &by_name,
                    environment.as_ref(),
                ));
            }
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
    PythonDependencyResolution {
        package_roots,
        local_modules,
        resolutions,
        environment: environment.map(|value| value.summary),
        diagnostics,
    }
}

fn discover_package_roots(root: &Path) -> Result<Vec<PathBuf>, PythonResolutionError> {
    let pyproject = root.join("pyproject.toml");
    let source = if pyproject.is_file() {
        Some(
            fs::read_to_string(&pyproject).map_err(|source| PythonResolutionError::Io {
                context: format!("cannot read {}", pyproject.display()),
                source,
            })?,
        )
    } else {
        None
    };
    package_roots(source.as_deref(), |path| root.join(path).is_dir())
}

fn package_roots(
    source: Option<&str>,
    is_dir: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, PythonResolutionError> {
    let mut roots = BTreeSet::new();
    if let Some(source) = source {
        let document = source.parse::<toml::Value>().map_err(|error| {
            PythonResolutionError::InvalidProject(format!("cannot parse pyproject.toml: {error}"))
        })?;
        if let Some(value) = document
            .get("tool")
            .and_then(|value| value.get("setuptools"))
            .and_then(|value| value.get("package-dir"))
            .and_then(|value| value.get(""))
            .and_then(toml::Value::as_str)
        {
            insert_root(&mut roots, value);
        }
        if let Some(values) = document
            .get("tool")
            .and_then(|value| value.get("setuptools"))
            .and_then(|value| value.get("packages"))
            .and_then(|value| value.get("find"))
            .and_then(|value| value.get("where"))
        {
            if let Some(value) = values.as_str() {
                insert_root(&mut roots, value);
            } else if let Some(values) = values.as_array() {
                for value in values.iter().filter_map(toml::Value::as_str) {
                    insert_root(&mut roots, value);
                }
            }
        }
        if let Some(packages) = document
            .get("tool")
            .and_then(|value| value.get("poetry"))
            .and_then(|value| value.get("packages"))
            .and_then(toml::Value::as_array)
        {
            for package in packages {
                if let Some(value) = package.get("from").and_then(toml::Value::as_str) {
                    insert_root(&mut roots, value);
                }
            }
        }
    }
    if roots.is_empty() {
        if is_dir(Path::new("src")) {
            roots.insert(PathBuf::from("src"));
        } else {
            roots.insert(PathBuf::from("."));
        }
    }
    roots.retain(|candidate| is_dir(candidate));
    if roots.is_empty() {
        return Err(PythonResolutionError::InvalidProject(
            "configured Python package roots do not exist".to_owned(),
        ));
    }
    let mut roots = roots.into_iter().collect::<Vec<_>>();
    roots.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.cmp(right))
    });
    Ok(roots)
}

fn insert_root(roots: &mut BTreeSet<PathBuf>, value: &str) {
    let path = PathBuf::from(value);
    if !path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        roots.insert(if path.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            path
        });
    }
}

fn build_module_catalog(
    analysis: &RepositoryAnalysis,
    package_roots: &[PathBuf],
) -> (
    Vec<LocalModule>,
    BTreeMap<String, Vec<LocalModule>>,
    Vec<String>,
) {
    let mut modules = Vec::new();
    let mut diagnostics = Vec::new();
    for run in &analysis.analyzers {
        if run.descriptor.language.as_str() != "python" {
            continue;
        }
        for file in &run.files {
            let Some(module_name) = module_name_for_path(&file.path, package_roots) else {
                diagnostics.push(format!(
                    "Python file {} is outside the discovered package roots",
                    file.path.display()
                ));
                continue;
            };
            if module_name.is_empty() {
                diagnostics.push(format!(
                    "Python file {} does not produce an importable module name",
                    file.path.display()
                ));
                continue;
            }
            modules.push(LocalModule::new(
                ModuleId::new(LanguageId::new("python"), module_name),
                file.path.clone(),
            ));
        }
    }
    modules.sort();
    let mut by_name = BTreeMap::<String, Vec<LocalModule>>::new();
    for module in &modules {
        by_name
            .entry(module.id.qualified_name().to_owned())
            .or_default()
            .push(module.clone());
    }
    for (name, candidates) in &by_name {
        if candidates.len() > 1 {
            diagnostics.push(format!(
                "module {name} has {} candidate files and imports remain ambiguous",
                candidates.len()
            ));
        }
    }
    (modules, by_name, diagnostics)
}

fn module_name_for_path(path: &Path, roots: &[PathBuf]) -> Option<String> {
    let relative = roots.iter().find_map(|root| {
        if root == Path::new(".") {
            Some(path)
        } else {
            path.strip_prefix(root).ok()
        }
    })?;
    if relative.extension().and_then(|value| value.to_str()) != Some("py") {
        return None;
    }
    let mut parts = relative
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_owned),
            _ => None,
        })
        .collect::<Vec<_>>();
    let stem = relative.file_stem()?.to_str()?;
    if stem != "__init__" {
        parts.push(stem.to_owned());
    }
    if parts.iter().any(|part| !is_python_identifier(part)) {
        return None;
    }
    Some(parts.join("."))
}

fn is_python_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn resolve_reference(
    source: &LocalModule,
    reference: &DependencyReference,
    import: &ImportReference,
    local: &BTreeMap<String, Vec<LocalModule>>,
    environment: Option<&PythonEnvironment>,
) -> ProjectDependencyResolution {
    let requested = requested_module(source, import);
    let outcome = match requested {
        Err(reason) => DependencyResolutionOutcome::unresolved(reference_text(import), reason),
        Ok(requested) => resolve_requested(&requested, import, local, environment),
    };
    ProjectDependencyResolution::new(
        source.path.clone(),
        source.id.clone(),
        reference.clone(),
        outcome,
    )
}

fn requested_module(
    source: &LocalModule,
    reference: &ImportReference,
) -> Result<String, UnresolvedDependencyReason> {
    if reference.relative_level == 0 {
        return reference
            .module
            .clone()
            .or_else(|| reference.imported_name.clone())
            .ok_or(UnresolvedDependencyReason::ModuleNotFound);
    }
    let is_package =
        source.path.file_name().and_then(|value| value.to_str()) == Some("__init__.py");
    let mut package = source
        .id
        .qualified_name()
        .split('.')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !is_package {
        package.pop();
    }
    let levels_up = reference.relative_level.saturating_sub(1);
    if levels_up > 0 && levels_up >= package.len() {
        return Err(UnresolvedDependencyReason::RelativeImportBeyondRoot);
    }
    package.truncate(package.len() - levels_up);
    if let Some(module) = &reference.module {
        package.extend(module.split('.').map(str::to_owned));
    }
    if package.is_empty() {
        return Err(UnresolvedDependencyReason::MissingPackageContext);
    }
    Ok(package.join("."))
}

fn resolve_requested(
    requested: &str,
    reference: &ImportReference,
    local: &BTreeMap<String, Vec<LocalModule>>,
    environment: Option<&PythonEnvironment>,
) -> DependencyResolutionOutcome {
    let child = reference.imported_name.as_ref().and_then(|name| {
        let candidate = format!("{requested}.{name}");
        local.contains_key(&candidate).then_some(candidate)
    });
    let local_name = child.as_deref().unwrap_or(requested);
    if let Some(candidates) = local.get(local_name) {
        let targets = candidates
            .iter()
            .cloned()
            .map(DependencyTarget::LocalModule)
            .collect::<Vec<_>>();
        return if targets.len() == 1 {
            DependencyResolutionOutcome::exact(targets[0].clone())
        } else {
            DependencyResolutionOutcome::ambiguous(local_name, targets)
        };
    }
    if reference.relative_level > 0 {
        return DependencyResolutionOutcome::unresolved(
            requested,
            UnresolvedDependencyReason::ModuleNotFound,
        );
    }
    let Some(environment) = environment else {
        return DependencyResolutionOutcome::unresolved(
            requested,
            UnresolvedDependencyReason::EnvironmentUnavailable,
        );
    };
    let top_level = requested.split('.').next().unwrap_or(requested);
    let module = ModuleId::new(LanguageId::new("python"), requested);
    if environment.stdlib_names.contains(top_level) {
        return DependencyResolutionOutcome::exact(DependencyTarget::StandardLibrary(module));
    }
    if let Some(providers) = environment.import_providers.get(top_level) {
        let targets = providers
            .iter()
            .map(|provider| DependencyTarget::InstalledDistribution {
                import_module: module.clone(),
                distribution_name: provider.normalized_name.clone(),
                distribution_display_name: provider.display_name.clone(),
                version: provider.version.clone(),
            })
            .collect::<Vec<_>>();
        return if targets.len() == 1 {
            DependencyResolutionOutcome::exact(targets[0].clone())
        } else {
            DependencyResolutionOutcome::ambiguous(requested, targets)
        };
    }
    DependencyResolutionOutcome::unresolved(requested, UnresolvedDependencyReason::ModuleNotFound)
}

fn reference_text(reference: &ImportReference) -> String {
    let dots = ".".repeat(reference.relative_level);
    let module = reference.module.as_deref().unwrap_or_default();
    let imported = reference
        .imported_name
        .as_ref()
        .map(|value| format!("::{value}"))
        .unwrap_or_default();
    format!("{dots}{module}{imported}")
}

fn probe_environment(
    selection: &PythonEnvironmentSelection,
) -> Result<PythonEnvironment, PythonResolutionError> {
    let (selection_kind, executable) = match selection {
        PythonEnvironmentSelection::Interpreter(path) => ("interpreter", path.clone()),
        PythonEnvironmentSelection::VirtualEnvironment(path) => {
            if !path.join("pyvenv.cfg").is_file() {
                return Err(PythonResolutionError::InvalidEnvironment(format!(
                    "virtual environment {} does not contain pyvenv.cfg",
                    path.display()
                )));
            }
            #[cfg(windows)]
            let executable = path.join("Scripts").join("python.exe");
            #[cfg(not(windows))]
            let executable = path.join("bin").join("python");
            ("virtual-environment", executable)
        }
    };
    if !executable.is_file() {
        return Err(PythonResolutionError::InvalidEnvironment(format!(
            "Python executable {} does not exist or is not a regular file",
            executable.display()
        )));
    }
    let output = run_probe(&executable)?;
    let payload: ProbePayload = serde_json::from_slice(&output).map_err(|error| {
        PythonResolutionError::Probe(format!("returned malformed JSON: {error}"))
    })?;
    if payload.schema_version != PROBE_SCHEMA_VERSION {
        return Err(PythonResolutionError::Probe(format!(
            "returned unsupported schema {:?}",
            payload.schema_version
        )));
    }
    if payload.version[0] != 3 || payload.version[1] < 8 {
        return Err(PythonResolutionError::InvalidEnvironment(format!(
            "Python 3.8 or newer is required; selected interpreter reported {}.{}.{}",
            payload.version[0], payload.version[1], payload.version[2]
        )));
    }
    let distribution_count = payload.distributions.len();
    let mut import_providers = BTreeMap::<String, Vec<InstalledDistribution>>::new();
    for distribution in payload.distributions {
        let provider = InstalledDistribution {
            normalized_name: distribution.normalized_name,
            display_name: distribution.display_name,
            version: distribution.version,
        };
        for import_name in distribution.import_names {
            import_providers
                .entry(import_name)
                .or_default()
                .push(provider.clone());
        }
    }
    for providers in import_providers.values_mut() {
        providers.sort();
        providers.dedup();
    }
    let executable = fs::canonicalize(&executable).unwrap_or(executable);
    Ok(PythonEnvironment {
        summary: PythonEnvironmentSummary {
            selection_kind,
            executable,
            implementation: payload.implementation,
            version: format!(
                "{}.{}.{}",
                payload.version[0], payload.version[1], payload.version[2]
            ),
            is_virtual_environment: payload.is_virtual_environment,
            distribution_count,
        },
        stdlib_names: payload.stdlib_names.into_iter().collect(),
        import_providers,
    })
}

fn run_probe(executable: &Path) -> Result<Vec<u8>, PythonResolutionError> {
    run_probe_with_timeout(executable, PROBE_TIMEOUT)
}

fn run_probe_with_timeout(
    executable: &Path,
    timeout: Duration,
) -> Result<Vec<u8>, PythonResolutionError> {
    let output = codegraide_core::process::run(
        Command::new(executable)
            .args(["-I", "-c", PYTHON_PROBE])
            .stdin(Stdio::null()),
        Instant::now() + timeout,
        PROBE_OUTPUT_LIMIT,
    )
    .map_err(|source| {
        if source.kind() == io::ErrorKind::TimedOut {
            PythonResolutionError::Probe(format!(
                "timed out after {} seconds",
                timeout.as_secs_f64()
            ))
        } else {
            PythonResolutionError::Io {
                context: format!("cannot probe Python interpreter {}", executable.display()),
                source,
            }
        }
    })?;
    if !output.status.success() {
        return Err(PythonResolutionError::Probe(format!(
            "interpreter exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_src_paths_and_package_initializers_to_module_names() {
        let roots = vec![PathBuf::from("src")];
        assert_eq!(
            module_name_for_path(Path::new("src/shop/models.py"), &roots).as_deref(),
            Some("shop.models")
        );
        assert_eq!(
            module_name_for_path(Path::new("src/shop/__init__.py"), &roots).as_deref(),
            Some("shop")
        );
        assert_eq!(
            module_name_for_path(Path::new("shop/models.py"), &[PathBuf::from(".")]).as_deref(),
            Some("shop.models")
        );
    }

    #[test]
    fn rejects_files_outside_roots_and_invalid_module_components() {
        let roots = vec![PathBuf::from("src")];
        assert!(module_name_for_path(Path::new("tests/test.py"), &roots).is_none());
        assert!(module_name_for_path(Path::new("src/bad-name.py"), &roots).is_none());
    }

    #[test]
    fn relative_imports_cannot_escape_the_top_level_package() {
        let source = LocalModule::new(
            ModuleId::new(LanguageId::new("python"), "shop.api"),
            "src/shop/api.py",
        );
        let reference = codegraide_core::ImportReference {
            module: Some("outside".to_owned()),
            imported_name: None,
            alias: None,
            relative_level: 2,
            wildcard: false,
            resolution: codegraide_core::ResolutionLevel::Syntactic,
            enclosing_symbol: None,
            context: codegraide_core::ImportContext::default(),
            span: codegraide_core::SourceSpan {
                start_byte: 0,
                end_byte: 1,
                start: codegraide_core::SourcePosition { line: 1, column: 0 },
                end: codegraide_core::SourcePosition { line: 1, column: 1 },
            },
        };

        assert_eq!(
            requested_module(&source, &reference),
            Err(UnresolvedDependencyReason::RelativeImportBeyondRoot)
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_timeout_kills_a_stalled_interpreter() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let executable = directory.path().join("stalled-python");
        fs::write(&executable, "#!/bin/sh\nsleep 2\n").expect("probe fixture");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("executable fixture");

        let error = run_probe_with_timeout(&executable, Duration::from_millis(20))
            .expect_err("stalled probe should time out");
        assert!(error.to_string().contains("timed out"));
    }
}
