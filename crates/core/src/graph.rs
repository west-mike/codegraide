//! Dependency graph construction and graph-derived measurements.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::analyzer::DependencyReference;
use crate::dependencies::{
    DependencyResolutionOutcome, DependencyTarget, LocalModule, ModuleId,
    ProjectDependencyResolution, UnresolvedDependencyReason,
};
use crate::report::LanguageId;

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyNode {
    LocalModule(LocalModule),
    StandardLibrary(ModuleId),
    InstalledDistribution {
        language: LanguageId,
        distribution_name: String,
        distribution_display_name: String,
        version: Option<String>,
    },
    Ambiguous {
        source_module: ModuleId,
        requested: String,
        candidates: Vec<DependencyTarget>,
    },
    Unresolved {
        source_module: ModuleId,
        requested: String,
        reason: UnresolvedDependencyReason,
    },
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyNodeKind {
    LocalModule,
    StandardLibrary,
    InstalledDistribution,
    Ambiguous,
    Unresolved,
}

impl DependencyNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalModule => "local-module",
            Self::StandardLibrary => "standard-library",
            Self::InstalledDistribution => "installed-distribution",
            Self::Ambiguous => "ambiguous",
            Self::Unresolved => "unresolved",
        }
    }
}

impl DependencyNode {
    pub fn kind(&self) -> DependencyNodeKind {
        match self {
            Self::LocalModule(_) => DependencyNodeKind::LocalModule,
            Self::StandardLibrary(_) => DependencyNodeKind::StandardLibrary,
            Self::InstalledDistribution { .. } => DependencyNodeKind::InstalledDistribution,
            Self::Ambiguous { .. } => DependencyNodeKind::Ambiguous,
            Self::Unresolved { .. } => DependencyNodeKind::Unresolved,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyRelationKind {
    Exact,
    Ambiguous,
    Unresolved,
}

impl DependencyRelationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Ambiguous => "ambiguous",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GraphEvidence {
    pub source_path: PathBuf,
    pub reference: DependencyReference,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyRelation {
    pub source: DependencyNode,
    pub target: DependencyNode,
    pub kind: DependencyRelationKind,
    pub evidence: Vec<GraphEvidence>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyNodeMetrics {
    pub node: DependencyNode,
    pub fan_in: usize,
    pub fan_out: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyScc {
    pub members: Vec<DependencyNode>,
    pub cyclic: bool,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct DependencyGraphCoverage {
    pub total_references: usize,
    pub exact_references: usize,
    pub ambiguous_references: usize,
    pub unresolved_references: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyGraphAnalysis {
    pub nodes: Vec<DependencyNode>,
    pub relations: Vec<DependencyRelation>,
    pub metrics: Vec<DependencyNodeMetrics>,
    pub strongly_connected_components: Vec<DependencyScc>,
    pub cycles: Vec<DependencyScc>,
    pub coverage: DependencyGraphCoverage,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DependencyGraphError {
    AbsolutePath {
        context: &'static str,
        path: PathBuf,
    },
    ConflictingLocalModule {
        module: ModuleId,
        first_path: PathBuf,
        second_path: PathBuf,
    },
    UnknownSourceModule {
        module: ModuleId,
    },
    SourcePathMismatch {
        module: ModuleId,
        expected: PathBuf,
        actual: PathBuf,
    },
    UnknownExactLocalTarget {
        module: ModuleId,
        path: PathBuf,
    },
    ExactLocalTargetPathMismatch {
        module: ModuleId,
        expected: PathBuf,
        actual: PathBuf,
    },
    EmptyAmbiguousCandidates {
        source_module: ModuleId,
        requested: String,
    },
}

impl fmt::Display for DependencyGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AbsolutePath { context, path } => {
                write!(
                    formatter,
                    "{context} path must be repository-relative: {}",
                    path.display()
                )
            }
            Self::ConflictingLocalModule {
                module,
                first_path,
                second_path,
            } => write!(
                formatter,
                "local module {} has conflicting paths {} and {}",
                module_name(module),
                first_path.display(),
                second_path.display()
            ),
            Self::UnknownSourceModule { module } => write!(
                formatter,
                "dependency source module {} is not in the local module catalog",
                module_name(module)
            ),
            Self::SourcePathMismatch {
                module,
                expected,
                actual,
            } => write!(
                formatter,
                "source module {} has path {}, but resolution reported {}",
                module_name(module),
                expected.display(),
                actual.display()
            ),
            Self::UnknownExactLocalTarget { module, path } => write!(
                formatter,
                "exact local target {} at {} is not in the local module catalog",
                module_name(module),
                path.display()
            ),
            Self::ExactLocalTargetPathMismatch {
                module,
                expected,
                actual,
            } => write!(
                formatter,
                "exact local target {} has catalog path {}, but resolution reported {}",
                module_name(module),
                expected.display(),
                actual.display()
            ),
            Self::EmptyAmbiguousCandidates {
                source_module,
                requested,
            } => write!(
                formatter,
                "ambiguous import {requested} from {} has no candidates",
                module_name(source_module)
            ),
        }
    }
}

impl std::error::Error for DependencyGraphError {}

pub fn analyze_dependency_graph(
    local_modules: &[LocalModule],
    resolutions: &[ProjectDependencyResolution],
) -> Result<DependencyGraphAnalysis, DependencyGraphError> {
    let catalog = build_catalog(local_modules)?;
    let mut nodes = catalog
        .values()
        .flatten()
        .cloned()
        .map(DependencyNode::LocalModule)
        .collect::<BTreeSet<_>>();
    let mut relation_evidence = BTreeMap::<
        (DependencyNode, DependencyNode, DependencyRelationKind),
        Vec<GraphEvidence>,
    >::new();
    let mut exact_edges = BTreeSet::<(DependencyNode, DependencyNode)>::new();
    let mut coverage = DependencyGraphCoverage {
        total_references: resolutions.len(),
        ..DependencyGraphCoverage::default()
    };

    for resolution in resolutions {
        validate_relative_path("dependency source", &resolution.source_path)?;
        let Some(source_candidates) = catalog.get(&resolution.source_module) else {
            return Err(DependencyGraphError::UnknownSourceModule {
                module: resolution.source_module.clone(),
            });
        };
        let Some(source_module) = source_candidates
            .iter()
            .find(|module| module.path == resolution.source_path)
        else {
            return Err(DependencyGraphError::SourcePathMismatch {
                module: resolution.source_module.clone(),
                expected: source_candidates[0].path.clone(),
                actual: resolution.source_path.clone(),
            });
        };

        let source = DependencyNode::LocalModule(source_module.clone());
        let evidence = GraphEvidence {
            source_path: resolution.source_path.clone(),
            reference: resolution.reference.clone(),
        };
        let (target, relation_kind) = match &resolution.outcome {
            DependencyResolutionOutcome::Exact(target) => {
                validate_target_paths(target)?;
                validate_exact_local_target(target, &catalog)?;
                coverage.exact_references += 1;
                (target_node(target), DependencyRelationKind::Exact)
            }
            DependencyResolutionOutcome::Ambiguous {
                requested,
                candidates,
            } => {
                if candidates.is_empty() {
                    return Err(DependencyGraphError::EmptyAmbiguousCandidates {
                        source_module: resolution.source_module.clone(),
                        requested: requested.clone(),
                    });
                }
                for candidate in candidates {
                    validate_target_paths(candidate)?;
                }
                coverage.ambiguous_references += 1;
                (
                    DependencyNode::Ambiguous {
                        source_module: resolution.source_module.clone(),
                        requested: requested.clone(),
                        candidates: sorted_targets(candidates),
                    },
                    DependencyRelationKind::Ambiguous,
                )
            }
            DependencyResolutionOutcome::Unresolved { requested, reason } => {
                coverage.unresolved_references += 1;
                (
                    DependencyNode::Unresolved {
                        source_module: resolution.source_module.clone(),
                        requested: requested.clone(),
                        reason: *reason,
                    },
                    DependencyRelationKind::Unresolved,
                )
            }
        };

        nodes.insert(target.clone());
        if relation_kind == DependencyRelationKind::Exact {
            exact_edges.insert((source.clone(), target.clone()));
        }
        relation_evidence
            .entry((source, target, relation_kind))
            .or_default()
            .push(evidence);
    }

    let relations = relation_evidence
        .into_iter()
        .map(|((source, target, kind), mut evidence)| {
            evidence.sort_by(|left, right| evidence_key(left).cmp(&evidence_key(right)));
            DependencyRelation {
                source,
                target,
                kind,
                evidence,
            }
        })
        .collect::<Vec<_>>();
    let nodes = nodes.into_iter().collect::<Vec<_>>();
    let metrics = calculate_metrics(&nodes, &exact_edges);
    let strongly_connected_components = calculate_sccs(&catalog, &exact_edges);
    let cycles = strongly_connected_components
        .iter()
        .filter(|component| component.cyclic)
        .cloned()
        .collect();

    Ok(DependencyGraphAnalysis {
        nodes,
        relations,
        metrics,
        strongly_connected_components,
        cycles,
        coverage,
    })
}

fn build_catalog(
    local_modules: &[LocalModule],
) -> Result<BTreeMap<ModuleId, Vec<LocalModule>>, DependencyGraphError> {
    let mut catalog = BTreeMap::<ModuleId, Vec<LocalModule>>::new();
    for module in local_modules {
        validate_relative_path("local module", &module.path)?;
        catalog
            .entry(module.id.clone())
            .or_default()
            .push(module.clone());
    }
    for modules in catalog.values_mut() {
        modules.sort();
        modules.dedup();
    }
    Ok(catalog)
}

fn validate_relative_path(
    context: &'static str,
    path: &std::path::Path,
) -> Result<(), DependencyGraphError> {
    if path.is_absolute() {
        return Err(DependencyGraphError::AbsolutePath {
            context,
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn validate_target_paths(target: &DependencyTarget) -> Result<(), DependencyGraphError> {
    if let DependencyTarget::LocalModule(module) = target {
        validate_relative_path("local target", &module.path)?;
    }
    Ok(())
}

fn validate_exact_local_target(
    target: &DependencyTarget,
    catalog: &BTreeMap<ModuleId, Vec<LocalModule>>,
) -> Result<(), DependencyGraphError> {
    let DependencyTarget::LocalModule(module) = target else {
        return Ok(());
    };
    let Some(candidates) = catalog.get(&module.id) else {
        return Err(DependencyGraphError::UnknownExactLocalTarget {
            module: module.id.clone(),
            path: module.path.clone(),
        });
    };
    if !candidates
        .iter()
        .any(|candidate| candidate.path == module.path)
    {
        return Err(DependencyGraphError::ExactLocalTargetPathMismatch {
            module: module.id.clone(),
            expected: candidates[0].path.clone(),
            actual: module.path.clone(),
        });
    }
    Ok(())
}

fn sorted_targets(targets: &[DependencyTarget]) -> Vec<DependencyTarget> {
    let mut targets = targets.to_vec();
    targets.sort();
    targets.dedup();
    targets
}

fn target_node(target: &DependencyTarget) -> DependencyNode {
    match target {
        DependencyTarget::LocalModule(module) => DependencyNode::LocalModule(module.clone()),
        DependencyTarget::StandardLibrary(module) => {
            DependencyNode::StandardLibrary(module.clone())
        }
        DependencyTarget::InstalledDistribution {
            import_module,
            distribution_name,
            distribution_display_name,
            version,
        } => DependencyNode::InstalledDistribution {
            language: import_module.language().clone(),
            distribution_name: distribution_name.clone(),
            distribution_display_name: distribution_display_name.clone(),
            version: version.clone(),
        },
    }
}

fn calculate_metrics(
    nodes: &[DependencyNode],
    exact_edges: &BTreeSet<(DependencyNode, DependencyNode)>,
) -> Vec<DependencyNodeMetrics> {
    let mut fan_in = BTreeMap::<DependencyNode, BTreeSet<DependencyNode>>::new();
    let mut fan_out = BTreeMap::<DependencyNode, BTreeSet<DependencyNode>>::new();
    for node in nodes {
        fan_in.entry(node.clone()).or_default();
        fan_out.entry(node.clone()).or_default();
    }
    for (source, target) in exact_edges {
        fan_out
            .entry(source.clone())
            .or_default()
            .insert(target.clone());
        fan_in
            .entry(target.clone())
            .or_default()
            .insert(source.clone());
    }
    nodes
        .iter()
        .map(|node| DependencyNodeMetrics {
            node: node.clone(),
            fan_in: fan_in[node].len(),
            fan_out: fan_out[node].len(),
        })
        .collect()
}

fn calculate_sccs(
    catalog: &BTreeMap<ModuleId, Vec<LocalModule>>,
    exact_edges: &BTreeSet<(DependencyNode, DependencyNode)>,
) -> Vec<DependencyScc> {
    let mut graph = DiGraph::<DependencyNode, ()>::new();
    let mut indexes = BTreeMap::<DependencyNode, NodeIndex>::new();
    for module in catalog.values().flatten() {
        let node = DependencyNode::LocalModule(module.clone());
        let index = graph.add_node(node.clone());
        indexes.insert(node, index);
    }
    for (source, target) in exact_edges {
        if source.kind() != DependencyNodeKind::LocalModule
            || target.kind() != DependencyNodeKind::LocalModule
        {
            continue;
        }
        let source_index = indexes[source];
        let target_index = indexes[target];
        graph.update_edge(source_index, target_index, ());
    }

    let self_edges = exact_edges
        .iter()
        .filter(|(source, target)| source == target)
        .map(|(node, _)| node.clone())
        .collect::<BTreeSet<_>>();
    let mut components = tarjan_scc(&graph)
        .into_iter()
        .map(|component| {
            let mut members = component
                .into_iter()
                .map(|index| graph[index].clone())
                .collect::<Vec<_>>();
            members.sort();
            let cyclic = members.len() > 1 || self_edges.contains(&members[0]);
            DependencyScc { members, cyclic }
        })
        .collect::<Vec<_>>();
    components.sort_by(|left, right| left.members.cmp(&right.members));
    components
}

fn evidence_key(evidence: &GraphEvidence) -> (&PathBuf, usize, usize, usize, usize) {
    (
        &evidence.source_path,
        evidence.reference.span().start_byte,
        evidence.reference.span().end_byte,
        evidence.reference.span().start.line,
        evidence.reference.span().start.column,
    )
}

fn module_name(module: &ModuleId) -> String {
    format!("{}:{}", module.language().as_str(), module.qualified_name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{ResolutionLevel, SourcePosition, SourceSpan};

    fn module(name: &str) -> ModuleId {
        ModuleId::new(LanguageId::new("python"), name)
    }

    fn local(name: &str) -> LocalModule {
        LocalModule::new(module(name), format!("{name}.py"))
    }

    fn reference(name: &str, start_byte: usize) -> DependencyReference {
        DependencyReference::Import(crate::ImportReference {
            module: Some(name.to_owned()),
            imported_name: None,
            alias: None,
            relative_level: 0,
            wildcard: false,
            resolution: ResolutionLevel::Syntactic,
            enclosing_symbol: None,
            context: crate::ImportContext::default(),
            span: SourceSpan {
                start_byte,
                end_byte: start_byte + name.len(),
                start: SourcePosition {
                    line: start_byte + 1,
                    column: 0,
                },
                end: SourcePosition {
                    line: start_byte + 1,
                    column: name.len(),
                },
            },
        })
    }

    fn exact(
        source: &LocalModule,
        target: &LocalModule,
        start_byte: usize,
    ) -> ProjectDependencyResolution {
        ProjectDependencyResolution::new(
            source.path.clone(),
            source.id.clone(),
            reference(target.id.qualified_name(), start_byte),
            DependencyResolutionOutcome::exact(DependencyTarget::LocalModule(target.clone())),
        )
    }

    fn metric<'a>(analysis: &'a DependencyGraphAnalysis, name: &str) -> &'a DependencyNodeMetrics {
        analysis
            .metrics
            .iter()
            .find(|metric| match &metric.node {
                DependencyNode::LocalModule(module) => module.id.qualified_name() == name,
                _ => false,
            })
            .expect("local module metric")
    }

    #[test]
    fn includes_isolated_modules_and_non_cyclic_singletons() {
        let analysis = analyze_dependency_graph(&[local("a"), local("b")], &[])
            .expect("empty graph should succeed");

        assert_eq!(analysis.nodes.len(), 2);
        assert_eq!(analysis.metrics.len(), 2);
        assert_eq!(metric(&analysis, "a").fan_in, 0);
        assert_eq!(metric(&analysis, "a").fan_out, 0);
        assert_eq!(analysis.strongly_connected_components.len(), 2);
        assert!(analysis.cycles.is_empty());
    }

    #[test]
    fn calculates_chain_fan_in_and_fan_out() {
        let a = local("a");
        let b = local("b");
        let c = local("c");
        let analysis = analyze_dependency_graph(
            &[a.clone(), b.clone(), c.clone()],
            &[exact(&a, &b, 0), exact(&b, &c, 1)],
        )
        .expect("chain should succeed");

        assert_eq!(metric(&analysis, "a").fan_in, 0);
        assert_eq!(metric(&analysis, "a").fan_out, 1);
        assert_eq!(metric(&analysis, "b").fan_in, 1);
        assert_eq!(metric(&analysis, "b").fan_out, 1);
        assert_eq!(metric(&analysis, "c").fan_in, 1);
        assert_eq!(metric(&analysis, "c").fan_out, 0);
        assert_eq!(analysis.relations.len(), 2);
        assert!(analysis.cycles.is_empty());
    }

    #[test]
    fn deduplicates_edges_but_preserves_evidence() {
        let a = local("a");
        let b = local("b");
        let analysis = analyze_dependency_graph(
            &[a.clone(), b.clone()],
            &[exact(&a, &b, 0), exact(&a, &b, 10)],
        )
        .expect("duplicate edges should succeed");

        assert_eq!(analysis.relations.len(), 1);
        assert_eq!(analysis.relations[0].evidence.len(), 2);
        assert_eq!(metric(&analysis, "a").fan_out, 1);
        assert_eq!(metric(&analysis, "b").fan_in, 1);
    }

    #[test]
    fn finds_mutual_and_self_cycles() {
        let a = local("a");
        let b = local("b");
        let c = local("c");
        let analysis = analyze_dependency_graph(
            &[a.clone(), b.clone(), c.clone()],
            &[exact(&a, &b, 0), exact(&b, &a, 1), exact(&c, &c, 2)],
        )
        .expect("cycles should succeed");

        assert_eq!(analysis.cycles.len(), 2);
        assert!(analysis.cycles.iter().any(|cycle| cycle.members.len() == 2));
        assert!(analysis.cycles.iter().any(|cycle| cycle.members.len() == 1));
    }

    #[test]
    fn counts_boundary_targets_but_excludes_them_from_local_sccs() {
        let a = local("a");
        let standard = DependencyTarget::StandardLibrary(module("json"));
        let installed = DependencyTarget::InstalledDistribution {
            import_module: module("requests"),
            distribution_name: "requests".to_owned(),
            distribution_display_name: "requests".to_owned(),
            version: Some("2.32.0".to_owned()),
        };
        let resolutions = [
            ProjectDependencyResolution::new(
                a.path.clone(),
                a.id.clone(),
                reference("json", 0),
                DependencyResolutionOutcome::exact(standard),
            ),
            ProjectDependencyResolution::new(
                a.path.clone(),
                a.id.clone(),
                reference("requests", 1),
                DependencyResolutionOutcome::exact(installed),
            ),
        ];
        let analysis = analyze_dependency_graph(std::slice::from_ref(&a), &resolutions)
            .expect("boundary targets should succeed");

        assert_eq!(metric(&analysis, "a").fan_out, 2);
        assert_eq!(analysis.strongly_connected_components.len(), 1);
        assert!(analysis.cycles.is_empty());
        assert_eq!(analysis.coverage.exact_references, 2);
    }

    #[test]
    fn uncertain_relations_are_reported_but_do_not_affect_metrics() {
        let a = local("a");
        let ambiguous = ProjectDependencyResolution::new(
            a.path.clone(),
            a.id.clone(),
            reference("shared", 0),
            DependencyResolutionOutcome::ambiguous(
                "shared",
                vec![
                    DependencyTarget::LocalModule(local("shared.one")),
                    DependencyTarget::LocalModule(local("shared.two")),
                ],
            ),
        );
        let unresolved = ProjectDependencyResolution::new(
            a.path.clone(),
            a.id.clone(),
            reference("missing", 1),
            DependencyResolutionOutcome::unresolved(
                "missing",
                UnresolvedDependencyReason::ModuleNotFound,
            ),
        );
        let analysis = analyze_dependency_graph(&[a], &[ambiguous, unresolved])
            .expect("uncertain relations should succeed");

        assert_eq!(analysis.relations.len(), 2);
        assert_eq!(analysis.coverage.total_references, 2);
        assert_eq!(analysis.coverage.ambiguous_references, 1);
        assert_eq!(analysis.coverage.unresolved_references, 1);
        assert_eq!(metric(&analysis, "a").fan_out, 0);
    }

    #[test]
    fn equivalent_input_order_has_identical_output() {
        let a = local("a");
        let b = local("b");
        let first = exact(&a, &b, 0);
        let second = exact(&a, &b, 10);
        let left =
            analyze_dependency_graph(&[b.clone(), a.clone()], &[second.clone(), first.clone()])
                .expect("left graph");
        let right = analyze_dependency_graph(&[a, b], &[first, second]).expect("right graph");

        assert_eq!(left, right);
    }

    #[test]
    fn accepts_duplicate_import_names_as_distinct_local_candidates() {
        let analysis = analyze_dependency_graph(
            &[
                LocalModule::new(module("a"), "a.py"),
                LocalModule::new(module("a"), "other.py"),
            ],
            &[],
        )
        .expect("duplicate import names remain distinct by path");
        assert_eq!(analysis.nodes.len(), 2);
    }

    #[test]
    fn rejects_unknown_exact_local_targets() {
        let a = local("a");
        let missing = local("missing");
        let error = analyze_dependency_graph(std::slice::from_ref(&a), &[exact(&a, &missing, 0)])
            .expect_err("unknown exact target should fail");
        assert!(
            error
                .to_string()
                .contains("not in the local module catalog")
        );
    }

    #[test]
    fn rejects_absolute_paths_and_empty_ambiguity() {
        let absolute = LocalModule::new(module("a"), "/tmp/a.py");
        let error =
            analyze_dependency_graph(&[absolute], &[]).expect_err("absolute path should fail");
        assert!(error.to_string().contains("repository-relative"));

        let a = local("a");
        let empty = ProjectDependencyResolution::new(
            a.path.clone(),
            a.id.clone(),
            reference("missing", 0),
            DependencyResolutionOutcome::Ambiguous {
                requested: "missing".to_owned(),
                candidates: Vec::new(),
            },
        );
        let error =
            analyze_dependency_graph(&[a], &[empty]).expect_err("empty ambiguity should fail");
        assert!(error.to_string().contains("has no candidates"));
    }
}
