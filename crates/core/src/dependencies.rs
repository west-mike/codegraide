//! Domain types for dependency resolution.
//!
//! `DependencyReference` records what a language analyzer saw in one source
//! file. The types in this module record what a later project-resolution stage
//! concluded about that reference. Keeping those stages separate preserves
//! source evidence when graph construction eventually deduplicates edges.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use crate::analysis::RepositoryAnalysis;
use crate::analyzer::{DependencyReference, ImportRequirement, ImportScope, ImportUsage};
use crate::report::LanguageId;

pub const DEPENDENCY_GRAPH_DEFINITION_VERSION: &str = "dependency-graph-v2";
pub const DEPENDENCY_FAN_IN_DEFINITION_VERSION: &str = "dependency-fan-in-v2";
pub const DEPENDENCY_FAN_OUT_DEFINITION_VERSION: &str = "dependency-fan-out-v2";
pub const DEPENDENCY_SCC_DEFINITION_VERSION: &str = "dependency-scc-v2";
pub const DEPENDENCY_CYCLE_DEFINITION_VERSION: &str = "dependency-cycle-v2";

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyUnitKind {
    Module,
    File,
}

impl DependencyUnitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyResolverDescriptor {
    pub id: String,
    pub language: LanguageId,
    pub version: String,
    pub definition_version: String,
    pub local_unit_kind: DependencyUnitKind,
    pub hierarchy_behavior: String,
    pub resolution_capabilities: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyResolutionContextCoverage {
    pub kind: String,
    pub selected: bool,
    pub total: usize,
    pub supported: usize,
    pub unsupported: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ResolvedProjectDependencies {
    pub local_units: Vec<LocalModule>,
    pub resolutions: Vec<ProjectDependencyResolution>,
    pub context_coverage: Vec<DependencyResolutionContextCoverage>,
    pub summary_lines: Vec<String>,
    pub metadata: BTreeMap<String, String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyResolverError {
    message: String,
}

impl DependencyResolverError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DependencyResolverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DependencyResolverError {}

pub trait DependencyResolver {
    fn descriptor(&self) -> &DependencyResolverDescriptor;

    fn resolve(
        &self,
        analysis: &RepositoryAnalysis,
    ) -> Result<ResolvedProjectDependencies, DependencyResolverError>;
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DependencyResolverRegistryError {
    DuplicateLanguage(LanguageId),
    UnavailableLanguage(LanguageId),
}

impl fmt::Display for DependencyResolverRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateLanguage(language) => write!(
                formatter,
                "dependency resolver for language {:?} is already registered",
                language.as_str()
            ),
            Self::UnavailableLanguage(language) => write!(
                formatter,
                "dependency resolver for language {:?} is not installed",
                language.as_str()
            ),
        }
    }
}

impl std::error::Error for DependencyResolverRegistryError {}

#[derive(Default)]
pub struct DependencyResolverRegistry {
    resolvers: BTreeMap<LanguageId, Box<dyn DependencyResolver>>,
}

impl DependencyResolverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        resolver: Box<dyn DependencyResolver>,
    ) -> Result<(), DependencyResolverRegistryError> {
        let language = resolver.descriptor().language.clone();
        if self.resolvers.contains_key(&language) {
            return Err(DependencyResolverRegistryError::DuplicateLanguage(language));
        }
        self.resolvers.insert(language, resolver);
        Ok(())
    }

    pub fn languages(&self) -> impl Iterator<Item = &LanguageId> {
        self.resolvers.keys()
    }

    pub fn get(&self, language: &LanguageId) -> Option<&dyn DependencyResolver> {
        self.resolvers.get(language).map(Box::as_ref)
    }

    pub fn resolve(
        &self,
        language: &LanguageId,
        analysis: &RepositoryAnalysis,
    ) -> Result<ResolvedProjectDependencies, DependencyResolverError> {
        let resolver = self.get(language).ok_or_else(|| {
            DependencyResolverError::new(
                DependencyResolverRegistryError::UnavailableLanguage(language.clone()).to_string(),
            )
        })?;
        resolver.resolve(analysis)
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct DependencyGraphInputExclusions {
    pub type_only: bool,
    pub optional: bool,
    pub callable_local: bool,
    pub conditional: bool,
}

impl DependencyGraphInputExclusions {
    pub fn retains(self, reference: &DependencyReference) -> bool {
        if let Some(reference) = reference.as_include() {
            return !(self.conditional && reference.conditional);
        }
        let Some(reference) = reference.as_import() else {
            return true;
        };
        !(self.type_only && reference.context.usage == ImportUsage::TypeCheckingOnly
            || self.optional && reference.context.requirement == ImportRequirement::Optional
            || self.callable_local && reference.context.scope == ImportScope::Callable
            || self.conditional && reference.context.conditional)
    }
}

/// Stable importable identity for a module in a language.
///
/// Python uses the qualified import name, such as `shop.models`, rather than
/// a repository-relative path. A language component keeps the identity safe
/// for the future multi-language graph even when two languages use the same
/// module spelling.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ModuleId {
    language: LanguageId,
    qualified_name: String,
}

impl ModuleId {
    pub fn new(language: LanguageId, qualified_name: impl Into<String>) -> Self {
        Self {
            language,
            qualified_name: qualified_name.into(),
        }
    }

    pub fn language(&self) -> &LanguageId {
        &self.language
    }

    pub fn qualified_name(&self) -> &str {
        &self.qualified_name
    }
}

/// A local module identity and its repository-relative source path.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct LocalModule {
    pub id: ModuleId,
    pub path: PathBuf,
    pub outgoing_dependencies_analyzed: bool,
}

impl LocalModule {
    pub fn new(id: ModuleId, path: impl Into<PathBuf>) -> Self {
        Self {
            id,
            path: path.into(),
            outgoing_dependencies_analyzed: true,
        }
    }

    pub fn with_outgoing_dependencies_analyzed(mut self, analyzed: bool) -> Self {
        self.outgoing_dependencies_analyzed = analyzed;
        self
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyTargetKind {
    LocalModule,
    StandardLibrary,
    InstalledDistribution,
    SystemHeader,
    ExternalHeader,
}

impl DependencyTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalModule => "local-module",
            Self::StandardLibrary => "standard-library",
            Self::InstalledDistribution => "installed-distribution",
            Self::SystemHeader => "system-header",
            Self::ExternalHeader => "external-header",
        }
    }
}

/// An exact dependency target. Ambiguous and unresolved references are
/// represented by `DependencyResolutionOutcome` rather than by this enum.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyTarget {
    LocalModule(LocalModule),
    StandardLibrary(ModuleId),
    InstalledDistribution {
        import_module: ModuleId,
        distribution_name: String,
        distribution_display_name: String,
        version: Option<String>,
    },
    SystemHeader {
        language: LanguageId,
        name: String,
    },
    ExternalHeader {
        language: LanguageId,
        name: String,
    },
}

impl DependencyTarget {
    pub fn kind(&self) -> DependencyTargetKind {
        match self {
            Self::LocalModule(_) => DependencyTargetKind::LocalModule,
            Self::StandardLibrary(_) => DependencyTargetKind::StandardLibrary,
            Self::InstalledDistribution { .. } => DependencyTargetKind::InstalledDistribution,
            Self::SystemHeader { .. } => DependencyTargetKind::SystemHeader,
            Self::ExternalHeader { .. } => DependencyTargetKind::ExternalHeader,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum UnresolvedDependencyReason {
    ModuleNotFound,
    RelativeImportBeyondRoot,
    MissingPackageContext,
    EnvironmentUnavailable,
    BuildContextUnavailable,
    HeaderNotFound,
    MacroInclude,
    UnsupportedCompileCommand,
    ImplicitSearchPathUnavailable,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum InferredDependencyBasis {
    UniqueRepositorySuffix,
}

impl InferredDependencyBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UniqueRepositorySuffix => "unique-repository-suffix",
        }
    }
}

impl UnresolvedDependencyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModuleNotFound => "module-not-found",
            Self::RelativeImportBeyondRoot => "relative-import-beyond-root",
            Self::MissingPackageContext => "missing-package-context",
            Self::EnvironmentUnavailable => "environment-unavailable",
            Self::BuildContextUnavailable => "build-context-unavailable",
            Self::HeaderNotFound => "header-not-found",
            Self::MacroInclude => "macro-include",
            Self::UnsupportedCompileCommand => "unsupported-compile-command",
            Self::ImplicitSearchPathUnavailable => "implicit-search-path-unavailable",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DependencyResolutionOutcome {
    Exact(DependencyTarget),
    Inferred {
        target: DependencyTarget,
        basis: InferredDependencyBasis,
    },
    Ambiguous {
        requested: String,
        candidates: Vec<DependencyTarget>,
    },
    Unresolved {
        requested: String,
        reason: UnresolvedDependencyReason,
    },
    ContextDependent {
        requested: String,
        candidates: Vec<DependencyTarget>,
        unresolved_reasons: Vec<UnresolvedDependencyReason>,
    },
}

impl DependencyResolutionOutcome {
    pub fn exact(target: DependencyTarget) -> Self {
        Self::Exact(target)
    }

    pub fn inferred(target: DependencyTarget, basis: InferredDependencyBasis) -> Self {
        Self::Inferred { target, basis }
    }

    pub fn ambiguous(requested: impl Into<String>, mut candidates: Vec<DependencyTarget>) -> Self {
        candidates.sort();
        candidates.dedup();
        Self::Ambiguous {
            requested: requested.into(),
            candidates,
        }
    }

    pub fn unresolved(requested: impl Into<String>, reason: UnresolvedDependencyReason) -> Self {
        Self::Unresolved {
            requested: requested.into(),
            reason,
        }
    }

    pub fn context_dependent(
        requested: impl Into<String>,
        mut candidates: Vec<DependencyTarget>,
        mut unresolved_reasons: Vec<UnresolvedDependencyReason>,
    ) -> Self {
        candidates.sort();
        candidates.dedup();
        unresolved_reasons.sort();
        unresolved_reasons.dedup();
        Self::ContextDependent {
            requested: requested.into(),
            candidates,
            unresolved_reasons,
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact(_))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Exact(_) => "exact",
            Self::Inferred { .. } => "inferred",
            Self::Ambiguous { .. } => "ambiguous",
            Self::Unresolved { .. } => "unresolved",
            Self::ContextDependent { .. } => "context-dependent",
        }
    }
}

/// The project-resolution result for one syntax-derived import reference.
///
/// `source_path` is repository-relative. `reference` remains the original
/// syntax evidence, including its source span and import spelling.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProjectDependencyResolution {
    pub source_path: PathBuf,
    pub source_module: ModuleId,
    pub reference: DependencyReference,
    pub outcome: DependencyResolutionOutcome,
}

impl ProjectDependencyResolution {
    pub fn new(
        source_path: PathBuf,
        source_module: ModuleId,
        reference: DependencyReference,
        outcome: DependencyResolutionOutcome,
    ) -> Self {
        Self {
            source_path,
            source_module,
            reference,
            outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{ResolutionLevel, SourcePosition, SourceSpan};

    fn module(language: &str, name: &str) -> ModuleId {
        ModuleId::new(LanguageId::new(language), name)
    }

    fn local_module(name: &str, path: &str) -> LocalModule {
        LocalModule::new(module("python", name), path)
    }

    fn reference(module_name: &str) -> DependencyReference {
        DependencyReference::Import(crate::ImportReference {
            module: Some(module_name.to_owned()),
            imported_name: None,
            alias: None,
            relative_level: 0,
            wildcard: false,
            resolution: ResolutionLevel::Syntactic,
            enclosing_symbol: None,
            context: crate::ImportContext::default(),
            span: SourceSpan {
                start_byte: 0,
                end_byte: module_name.len(),
                start: SourcePosition { line: 1, column: 0 },
                end: SourcePosition {
                    line: 1,
                    column: module_name.len(),
                },
            },
        })
    }

    #[test]
    fn module_ids_include_language_and_qualified_name() {
        let python = module("python", "shop.models");
        let rust = module("rust", "shop.models");

        assert_eq!(python.language().as_str(), "python");
        assert_eq!(python.qualified_name(), "shop.models");
        assert_ne!(python, rust);
    }

    #[test]
    fn target_kinds_have_stable_names() {
        assert_eq!(DependencyTargetKind::LocalModule.as_str(), "local-module");
        assert_eq!(
            DependencyTargetKind::StandardLibrary.as_str(),
            "standard-library"
        );
        assert_eq!(
            DependencyTargetKind::InstalledDistribution.as_str(),
            "installed-distribution"
        );
        assert_eq!(DependencyTargetKind::SystemHeader.as_str(), "system-header");
        assert_eq!(
            DependencyTargetKind::ExternalHeader.as_str(),
            "external-header"
        );
    }

    #[test]
    fn ambiguous_candidates_are_sorted_and_deduplicated() {
        let first =
            DependencyTarget::LocalModule(local_module("shop.models", "src/shop/models.py"));
        let second = DependencyTarget::LocalModule(local_module(
            "shop.models_alt",
            "src/shop/models_alt.py",
        ));
        let outcome = DependencyResolutionOutcome::ambiguous(
            "shop.models",
            vec![second.clone(), first.clone(), first.clone()],
        );

        assert_eq!(
            outcome,
            DependencyResolutionOutcome::Ambiguous {
                requested: "shop.models".to_owned(),
                candidates: vec![first, second],
            }
        );
        assert_eq!(outcome.kind(), "ambiguous");
        assert!(!outcome.is_exact());
    }

    #[test]
    fn inferred_outcomes_are_distinct_from_exact_resolution() {
        let target =
            DependencyTarget::LocalModule(local_module("include/shop.hpp", "include/shop.hpp"));
        let outcome = DependencyResolutionOutcome::inferred(
            target.clone(),
            InferredDependencyBasis::UniqueRepositorySuffix,
        );

        assert_eq!(outcome.kind(), "inferred");
        assert!(!outcome.is_exact());
        assert_eq!(
            outcome,
            DependencyResolutionOutcome::Inferred {
                target,
                basis: InferredDependencyBasis::UniqueRepositorySuffix,
            }
        );
        assert_eq!(
            InferredDependencyBasis::UniqueRepositorySuffix.as_str(),
            "unique-repository-suffix"
        );
    }

    #[test]
    fn resolution_preserves_original_reference_and_source_context() {
        let source_module = module("python", "shop.api");
        let original = reference("shop.models");
        let outcome = DependencyResolutionOutcome::exact(DependencyTarget::LocalModule(
            local_module("shop.models", "src/shop/models.py"),
        ));
        let resolution = ProjectDependencyResolution::new(
            PathBuf::from("src/shop/api.py"),
            source_module.clone(),
            original.clone(),
            outcome,
        );

        assert_eq!(resolution.source_path, PathBuf::from("src/shop/api.py"));
        assert_eq!(resolution.source_module, source_module);
        assert_eq!(resolution.reference, original);
        assert!(resolution.outcome.is_exact());
    }

    #[test]
    fn unresolved_reasons_have_stable_names() {
        assert_eq!(
            UnresolvedDependencyReason::ModuleNotFound.as_str(),
            "module-not-found"
        );
        assert_eq!(
            UnresolvedDependencyReason::RelativeImportBeyondRoot.as_str(),
            "relative-import-beyond-root"
        );
        assert_eq!(
            UnresolvedDependencyReason::MissingPackageContext.as_str(),
            "missing-package-context"
        );
        assert_eq!(
            UnresolvedDependencyReason::EnvironmentUnavailable.as_str(),
            "environment-unavailable"
        );
        assert_eq!(
            UnresolvedDependencyReason::BuildContextUnavailable.as_str(),
            "build-context-unavailable"
        );
        assert_eq!(
            UnresolvedDependencyReason::HeaderNotFound.as_str(),
            "header-not-found"
        );
    }

    #[test]
    fn dependency_definition_versions_are_stable() {
        assert_eq!(DEPENDENCY_GRAPH_DEFINITION_VERSION, "dependency-graph-v2");
        assert_eq!(DEPENDENCY_FAN_IN_DEFINITION_VERSION, "dependency-fan-in-v2");
        assert_eq!(
            DEPENDENCY_FAN_OUT_DEFINITION_VERSION,
            "dependency-fan-out-v2"
        );
        assert_eq!(DEPENDENCY_SCC_DEFINITION_VERSION, "dependency-scc-v2");
        assert_eq!(DEPENDENCY_CYCLE_DEFINITION_VERSION, "dependency-cycle-v2");
    }

    struct TestResolver {
        descriptor: DependencyResolverDescriptor,
    }

    impl TestResolver {
        fn new(language: &str) -> Self {
            Self {
                descriptor: DependencyResolverDescriptor {
                    id: format!("{language}-test-resolver"),
                    language: LanguageId::new(language),
                    version: "0.1.0".to_owned(),
                    definition_version: format!("{language}-test-resolution-v1"),
                    local_unit_kind: DependencyUnitKind::Module,
                    hierarchy_behavior: "test".to_owned(),
                    resolution_capabilities: Vec::new(),
                    limitations: Vec::new(),
                },
            }
        }
    }

    impl DependencyResolver for TestResolver {
        fn descriptor(&self) -> &DependencyResolverDescriptor {
            &self.descriptor
        }

        fn resolve(
            &self,
            _analysis: &RepositoryAnalysis,
        ) -> Result<ResolvedProjectDependencies, DependencyResolverError> {
            unreachable!("registry registration test does not resolve projects")
        }
    }

    #[test]
    fn resolver_registry_sorts_languages_and_rejects_duplicates() {
        let mut registry = DependencyResolverRegistry::new();
        registry
            .register(Box::new(TestResolver::new("python")))
            .expect("first resolver");
        registry
            .register(Box::new(TestResolver::new("cpp")))
            .expect("second resolver");

        assert_eq!(
            registry
                .languages()
                .map(LanguageId::as_str)
                .collect::<Vec<_>>(),
            ["cpp", "python"]
        );
        assert!(matches!(
            registry.register(Box::new(TestResolver::new("cpp"))),
            Err(DependencyResolverRegistryError::DuplicateLanguage(language))
                if language.as_str() == "cpp"
        ));
    }
}
