//! Domain types for dependency resolution.
//!
//! `DependencyReference` records what a language analyzer saw in one source
//! file. The types in this module record what a later project-resolution stage
//! concluded about that reference. Keeping those stages separate preserves
//! source evidence when graph construction eventually deduplicates edges.

use std::path::PathBuf;

use crate::analyzer::{DependencyReference, ImportRequirement, ImportScope, ImportUsage};
use crate::report::LanguageId;

pub const DEPENDENCY_GRAPH_DEFINITION_VERSION: &str = "dependency-graph-v1";
pub const DEPENDENCY_FAN_IN_DEFINITION_VERSION: &str = "dependency-fan-in-v1";
pub const DEPENDENCY_FAN_OUT_DEFINITION_VERSION: &str = "dependency-fan-out-v1";
pub const DEPENDENCY_SCC_DEFINITION_VERSION: &str = "dependency-scc-v1";
pub const DEPENDENCY_CYCLE_DEFINITION_VERSION: &str = "dependency-cycle-v1";

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct DependencyGraphInputExclusions {
    pub type_only: bool,
    pub optional: bool,
    pub callable_local: bool,
    pub conditional: bool,
}

impl DependencyGraphInputExclusions {
    pub fn retains(self, reference: &DependencyReference) -> bool {
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
}

impl LocalModule {
    pub fn new(id: ModuleId, path: impl Into<PathBuf>) -> Self {
        Self {
            id,
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyTargetKind {
    LocalModule,
    StandardLibrary,
    InstalledDistribution,
}

impl DependencyTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalModule => "local-module",
            Self::StandardLibrary => "standard-library",
            Self::InstalledDistribution => "installed-distribution",
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
}

impl DependencyTarget {
    pub fn kind(&self) -> DependencyTargetKind {
        match self {
            Self::LocalModule(_) => DependencyTargetKind::LocalModule,
            Self::StandardLibrary(_) => DependencyTargetKind::StandardLibrary,
            Self::InstalledDistribution { .. } => DependencyTargetKind::InstalledDistribution,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum UnresolvedDependencyReason {
    ModuleNotFound,
    RelativeImportBeyondRoot,
    MissingPackageContext,
    EnvironmentUnavailable,
}

impl UnresolvedDependencyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModuleNotFound => "module-not-found",
            Self::RelativeImportBeyondRoot => "relative-import-beyond-root",
            Self::MissingPackageContext => "missing-package-context",
            Self::EnvironmentUnavailable => "environment-unavailable",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DependencyResolutionOutcome {
    Exact(DependencyTarget),
    Ambiguous {
        requested: String,
        candidates: Vec<DependencyTarget>,
    },
    Unresolved {
        requested: String,
        reason: UnresolvedDependencyReason,
    },
}

impl DependencyResolutionOutcome {
    pub fn exact(target: DependencyTarget) -> Self {
        Self::Exact(target)
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

    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact(_))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Exact(_) => "exact",
            Self::Ambiguous { .. } => "ambiguous",
            Self::Unresolved { .. } => "unresolved",
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
    use crate::analyzer::{DependencyKind, ResolutionLevel, SourcePosition, SourceSpan};

    fn module(language: &str, name: &str) -> ModuleId {
        ModuleId::new(LanguageId::new(language), name)
    }

    fn local_module(name: &str, path: &str) -> LocalModule {
        LocalModule::new(module("python", name), path)
    }

    fn reference(module_name: &str) -> DependencyReference {
        DependencyReference {
            kind: DependencyKind::Import,
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
        }
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
    }

    #[test]
    fn dependency_definition_versions_are_stable() {
        assert_eq!(DEPENDENCY_GRAPH_DEFINITION_VERSION, "dependency-graph-v1");
        assert_eq!(DEPENDENCY_FAN_IN_DEFINITION_VERSION, "dependency-fan-in-v1");
        assert_eq!(
            DEPENDENCY_FAN_OUT_DEFINITION_VERSION,
            "dependency-fan-out-v1"
        );
        assert_eq!(DEPENDENCY_SCC_DEFINITION_VERSION, "dependency-scc-v1");
        assert_eq!(DEPENDENCY_CYCLE_DEFINITION_VERSION, "dependency-cycle-v1");
    }
}
