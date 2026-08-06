//! Deterministic reachability queries over exact local dependency relations.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::dependency_output::{
    DependencyGraphFilter, DependencyGraphView, DependencyGraphViewNode, filter_dependency_graph,
};
use crate::graph::{
    DependencyGraphAnalysis, DependencyNode, DependencyNodeKind, DependencyRelation,
    DependencyRelationKind,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DependencyQueryDirection {
    Dependencies,
    Dependents,
}

impl DependencyQueryDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dependencies => "dependencies",
            Self::Dependents => "dependents",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DependencyGraphQuery {
    ShortestPath {
        from: String,
        to: String,
    },
    Closure {
        module: String,
        direction: DependencyQueryDirection,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyGraphQueryResult {
    pub query: DependencyGraphQuery,
    pub found: bool,
    /// Path order for a path query; sorted module order for a closure query.
    pub nodes: Vec<DependencyNode>,
    pub relations: Vec<DependencyRelation>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyGraphQueryError {
    pub module: String,
    pub suggestions: Vec<String>,
}

impl fmt::Display for DependencyGraphQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "query module {:?} is not a local module",
            self.module
        )?;
        if !self.suggestions.is_empty() {
            write!(formatter, "; did you mean {}?", self.suggestions.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for DependencyGraphQueryError {}

pub fn query_dependency_graph(
    analysis: &DependencyGraphAnalysis,
    query: &DependencyGraphQuery,
) -> Result<DependencyGraphQueryResult, DependencyGraphQueryError> {
    let local = local_nodes(analysis);
    match query {
        DependencyGraphQuery::ShortestPath { from, to } => {
            let starts = resolve_modules(from, &local)?;
            let targets = resolve_modules(to, &local)?;
            let adjacency = adjacency(analysis, DependencyQueryDirection::Dependencies);
            if let Some(nodes) = shortest_path(&starts, &targets, &adjacency) {
                let relations = path_relations(analysis, &nodes);
                Ok(DependencyGraphQueryResult {
                    query: query.clone(),
                    found: true,
                    nodes,
                    relations,
                })
            } else {
                let nodes = starts
                    .into_iter()
                    .chain(targets)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                Ok(DependencyGraphQueryResult {
                    query: query.clone(),
                    found: false,
                    nodes,
                    relations: Vec::new(),
                })
            }
        }
        DependencyGraphQuery::Closure { module, direction } => {
            let roots = resolve_modules(module, &local)?;
            let adjacency = adjacency(analysis, *direction);
            let selected = closure(&roots, &adjacency);
            let relations = analysis
                .relations
                .iter()
                .filter(|relation| {
                    relation.kind == DependencyRelationKind::Exact
                        && relation.source.kind() == DependencyNodeKind::LocalModule
                        && relation.target.kind() == DependencyNodeKind::LocalModule
                        && selected.contains(&relation.source)
                        && selected.contains(&relation.target)
                })
                .cloned()
                .collect();
            Ok(DependencyGraphQueryResult {
                query: query.clone(),
                found: true,
                nodes: selected.into_iter().collect(),
                relations,
            })
        }
    }
}

pub fn dependency_query_view(
    analysis: &DependencyGraphAnalysis,
    result: &DependencyGraphQueryResult,
) -> DependencyGraphView {
    let full = filter_dependency_graph(analysis, &DependencyGraphFilter::default())
        .expect("an unfiltered graph view cannot contain an unknown focus module");
    let selected = result.nodes.iter().cloned().collect::<BTreeSet<_>>();
    let nodes = full
        .nodes
        .into_iter()
        .filter(|node| selected.contains(&node.node))
        .collect::<Vec<DependencyGraphViewNode>>();
    let strongly_connected_components = match &result.query {
        DependencyGraphQuery::Closure { .. } => analysis
            .strongly_connected_components
            .iter()
            .filter(|component| component.members.iter().all(|node| selected.contains(node)))
            .cloned()
            .collect(),
        DependencyGraphQuery::ShortestPath { .. } => Vec::new(),
    };
    DependencyGraphView {
        filter: DependencyGraphFilter::default(),
        nodes,
        relations: result.relations.clone(),
        strongly_connected_components,
    }
}

fn local_nodes(analysis: &DependencyGraphAnalysis) -> Vec<(String, DependencyNode)> {
    analysis
        .nodes
        .iter()
        .filter_map(|node| match node {
            DependencyNode::LocalModule(module) => {
                Some((module.id.qualified_name().to_owned(), node.clone()))
            }
            _ => None,
        })
        .collect()
}

fn resolve_modules(
    requested: &str,
    local: &[(String, DependencyNode)],
) -> Result<Vec<DependencyNode>, DependencyGraphQueryError> {
    let matches = local
        .iter()
        .filter(|(name, _)| name == requested)
        .map(|(_, node)| node.clone())
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(DependencyGraphQueryError {
            module: requested.to_owned(),
            suggestions: suggestions(requested, local),
        });
    }
    Ok(matches)
}

fn adjacency(
    analysis: &DependencyGraphAnalysis,
    direction: DependencyQueryDirection,
) -> BTreeMap<DependencyNode, Vec<DependencyNode>> {
    let mut adjacency = BTreeMap::<DependencyNode, BTreeSet<DependencyNode>>::new();
    for relation in &analysis.relations {
        if relation.kind != DependencyRelationKind::Exact
            || relation.source.kind() != DependencyNodeKind::LocalModule
            || relation.target.kind() != DependencyNodeKind::LocalModule
        {
            continue;
        }
        let (source, target) = match direction {
            DependencyQueryDirection::Dependencies => (&relation.source, &relation.target),
            DependencyQueryDirection::Dependents => (&relation.target, &relation.source),
        };
        adjacency
            .entry(source.clone())
            .or_default()
            .insert(target.clone());
    }
    adjacency
        .into_iter()
        .map(|(node, neighbors)| (node, neighbors.into_iter().collect()))
        .collect()
}

fn shortest_path(
    starts: &[DependencyNode],
    targets: &[DependencyNode],
    adjacency: &BTreeMap<DependencyNode, Vec<DependencyNode>>,
) -> Option<Vec<DependencyNode>> {
    let targets = targets.iter().cloned().collect::<BTreeSet<_>>();
    let mut queue = VecDeque::new();
    let mut previous = BTreeMap::<DependencyNode, Option<DependencyNode>>::new();
    for start in starts {
        if previous.insert(start.clone(), None).is_none() {
            queue.push_back(start.clone());
        }
    }
    while let Some(node) = queue.pop_front() {
        if targets.contains(&node) {
            let mut path = vec![node.clone()];
            let mut cursor = &node;
            while let Some(Some(parent)) = previous.get(cursor) {
                path.push(parent.clone());
                cursor = parent;
            }
            path.reverse();
            return Some(path);
        }
        for neighbor in adjacency.get(&node).into_iter().flatten() {
            if !previous.contains_key(neighbor) {
                previous.insert(neighbor.clone(), Some(node.clone()));
                queue.push_back(neighbor.clone());
            }
        }
    }
    None
}

fn closure(
    roots: &[DependencyNode],
    adjacency: &BTreeMap<DependencyNode, Vec<DependencyNode>>,
) -> BTreeSet<DependencyNode> {
    let mut selected = roots.iter().cloned().collect::<BTreeSet<_>>();
    let mut queue = roots.iter().cloned().collect::<VecDeque<_>>();
    while let Some(node) = queue.pop_front() {
        for neighbor in adjacency.get(&node).into_iter().flatten() {
            if selected.insert(neighbor.clone()) {
                queue.push_back(neighbor.clone());
            }
        }
    }
    selected
}

fn path_relations(
    analysis: &DependencyGraphAnalysis,
    nodes: &[DependencyNode],
) -> Vec<DependencyRelation> {
    nodes
        .windows(2)
        .filter_map(|pair| {
            analysis
                .relations
                .iter()
                .find(|relation| {
                    relation.kind == DependencyRelationKind::Exact
                        && relation.source == pair[0]
                        && relation.target == pair[1]
                })
                .cloned()
        })
        .collect()
}

fn suggestions(requested: &str, modules: &[(String, DependencyNode)]) -> Vec<String> {
    let mut ranked = modules
        .iter()
        .map(|(name, _)| (edit_distance(requested, name), name))
        .collect::<Vec<_>>();
    ranked.sort();
    ranked.dedup_by(|left, right| left.1 == right.1);
    ranked
        .into_iter()
        .take(3)
        .map(|(_, name)| name.clone())
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut previous = (0..=right.chars().count()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_char) in right.chars().enumerate() {
            current.push(std::cmp::min(
                std::cmp::min(current[right_index] + 1, previous[right_index + 1] + 1),
                previous[right_index] + usize::from(left_char != right_char),
            ));
        }
        previous = current;
    }
    previous[right.chars().count()]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        DependencyKind, DependencyReference, DependencyResolutionOutcome, DependencyTarget,
        LanguageId, LocalModule, ModuleId, ProjectDependencyResolution, ResolutionLevel,
        SourcePosition, SourceSpan, analyze_dependency_graph,
    };

    use super::*;

    fn module(name: &str) -> LocalModule {
        LocalModule::new(
            ModuleId::new(LanguageId::new("python"), name),
            PathBuf::from(format!("src/{}.py", name.replace('.', "/"))),
        )
    }

    fn edge(source: &LocalModule, target: &LocalModule) -> ProjectDependencyResolution {
        ProjectDependencyResolution {
            source_path: source.path.clone(),
            source_module: source.id.clone(),
            reference: DependencyReference {
                kind: DependencyKind::Import,
                module: Some(target.id.qualified_name().to_owned()),
                imported_name: None,
                alias: None,
                relative_level: 0,
                wildcard: false,
                resolution: ResolutionLevel::Syntactic,
                enclosing_symbol: None,
                context: crate::ImportContext::default(),
                span: SourceSpan {
                    start: SourcePosition { line: 1, column: 1 },
                    end: SourcePosition { line: 1, column: 2 },
                    start_byte: 0,
                    end_byte: 1,
                },
            },
            outcome: DependencyResolutionOutcome::exact(DependencyTarget::LocalModule(
                target.clone(),
            )),
        }
    }

    fn graph() -> DependencyGraphAnalysis {
        let a = module("pkg.a");
        let b = module("pkg.b");
        let c = module("pkg.c");
        let d = module("pkg.d");
        analyze_dependency_graph(
            &[a.clone(), b.clone(), c.clone(), d.clone()],
            &[edge(&a, &b), edge(&a, &c), edge(&b, &d), edge(&c, &d)],
        )
        .expect("graph")
    }

    #[test]
    fn shortest_path_uses_sorted_neighbors_for_equal_lengths() {
        let result = query_dependency_graph(
            &graph(),
            &DependencyGraphQuery::ShortestPath {
                from: "pkg.a".to_owned(),
                to: "pkg.d".to_owned(),
            },
        )
        .expect("query");
        assert!(result.found);
        assert_eq!(
            result.nodes.iter().map(node_name).collect::<Vec<_>>(),
            ["pkg.a", "pkg.b", "pkg.d"]
        );
    }

    #[test]
    fn closure_supports_both_directed_views() {
        let dependencies = query_dependency_graph(
            &graph(),
            &DependencyGraphQuery::Closure {
                module: "pkg.b".to_owned(),
                direction: DependencyQueryDirection::Dependencies,
            },
        )
        .expect("query");
        assert_eq!(
            dependencies.nodes.iter().map(node_name).collect::<Vec<_>>(),
            ["pkg.b", "pkg.d"]
        );

        let dependents = query_dependency_graph(
            &graph(),
            &DependencyGraphQuery::Closure {
                module: "pkg.d".to_owned(),
                direction: DependencyQueryDirection::Dependents,
            },
        )
        .expect("query");
        assert_eq!(
            dependents.nodes.iter().map(node_name).collect::<Vec<_>>(),
            ["pkg.a", "pkg.b", "pkg.c", "pkg.d"]
        );
    }

    #[test]
    fn same_node_and_unreachable_paths_are_successful_results() {
        let graph = graph();
        let same = query_dependency_graph(
            &graph,
            &DependencyGraphQuery::ShortestPath {
                from: "pkg.a".to_owned(),
                to: "pkg.a".to_owned(),
            },
        )
        .expect("query");
        assert!(same.found);
        assert_eq!(same.nodes.len(), 1);

        let unreachable = query_dependency_graph(
            &graph,
            &DependencyGraphQuery::ShortestPath {
                from: "pkg.d".to_owned(),
                to: "pkg.a".to_owned(),
            },
        )
        .expect("query");
        assert!(!unreachable.found);
        assert_eq!(unreachable.nodes.len(), 2);
        assert!(unreachable.relations.is_empty());
    }

    fn node_name(node: &DependencyNode) -> &str {
        match node {
            DependencyNode::LocalModule(module) => module.id.qualified_name(),
            _ => panic!("expected local module"),
        }
    }
}
