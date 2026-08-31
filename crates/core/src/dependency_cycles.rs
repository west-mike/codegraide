//! Compact, deterministic explanations for cyclic dependency SCCs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::graph::{
    DependencyGraphAnalysis, DependencyNode, DependencyRelation, DependencyRelationKind,
    DependencyScc,
};

pub const DEPENDENCY_CYCLE_EXPLANATION_DEFINITION_VERSION: &str = "dependency-cycle-explanation-v1";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyCycleExplanation {
    pub component_number: usize,
    pub members: Vec<DependencyNode>,
    /// Closed walk: the first node is repeated at the end.
    pub witness_nodes: Vec<DependencyNode>,
    pub witness_relations: Vec<DependencyRelation>,
    /// Approximate feedback-edge set; removing all entries makes the SCC acyclic.
    pub recommended_cuts: Vec<DependencyRelation>,
}

pub fn explain_dependency_cycles(
    analysis: &DependencyGraphAnalysis,
) -> Vec<DependencyCycleExplanation> {
    analysis
        .cycles
        .iter()
        .enumerate()
        .map(|(index, component)| {
            explain_dependency_cycle(index + 1, component, &analysis.relations)
        })
        .collect()
}

pub fn explain_dependency_cycle(
    component_number: usize,
    component: &DependencyScc,
    all_relations: &[DependencyRelation],
) -> DependencyCycleExplanation {
    let members = component.members.iter().cloned().collect::<BTreeSet<_>>();
    let relations = all_relations
        .iter()
        .filter(|relation| {
            relation.kind == DependencyRelationKind::Exact
                && members.contains(&relation.source)
                && members.contains(&relation.target)
        })
        .cloned()
        .collect::<Vec<_>>();
    let witness_nodes = shortest_witness(&component.members, &relations);
    let witness_relations = relations_for_walk(&witness_nodes, &relations);
    let recommended_cuts = weighted_feedback_edges(&component.members, &relations);
    debug_assert!(is_acyclic_after_cuts(
        &component.members,
        &relations,
        &recommended_cuts
    ));
    DependencyCycleExplanation {
        component_number,
        members: component.members.clone(),
        witness_nodes,
        witness_relations,
        recommended_cuts,
    }
}

fn shortest_witness(
    members: &[DependencyNode],
    relations: &[DependencyRelation],
) -> Vec<DependencyNode> {
    let adjacency = adjacency(relations);
    let mut best: Option<Vec<DependencyNode>> = None;
    for start in members {
        for neighbor in adjacency.get(start).into_iter().flatten() {
            let candidate = if neighbor == start {
                Some(vec![start.clone(), start.clone()])
            } else {
                shortest_path(neighbor, start, &adjacency).map(|mut path| {
                    path.insert(0, start.clone());
                    path
                })
            };
            if let Some(candidate) = candidate
                && best.as_ref().is_none_or(|current| {
                    candidate.len() < current.len()
                        || (candidate.len() == current.len() && candidate < *current)
                })
            {
                best = Some(candidate);
            }
        }
    }
    best.unwrap_or_default()
}

fn shortest_path(
    start: &DependencyNode,
    target: &DependencyNode,
    adjacency: &BTreeMap<DependencyNode, Vec<DependencyNode>>,
) -> Option<Vec<DependencyNode>> {
    let mut queue = VecDeque::from([start.clone()]);
    let mut previous = BTreeMap::from([(start.clone(), None::<DependencyNode>)]);
    while let Some(node) = queue.pop_front() {
        if &node == target {
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

fn adjacency(relations: &[DependencyRelation]) -> BTreeMap<DependencyNode, Vec<DependencyNode>> {
    let mut result = BTreeMap::<DependencyNode, BTreeSet<DependencyNode>>::new();
    for relation in relations {
        result
            .entry(relation.source.clone())
            .or_default()
            .insert(relation.target.clone());
    }
    result
        .into_iter()
        .map(|(node, neighbors)| (node, neighbors.into_iter().collect()))
        .collect()
}

fn relations_for_walk(
    nodes: &[DependencyNode],
    relations: &[DependencyRelation],
) -> Vec<DependencyRelation> {
    nodes
        .windows(2)
        .filter_map(|pair| {
            relations
                .iter()
                .find(|relation| relation.source == pair[0] && relation.target == pair[1])
                .cloned()
        })
        .collect()
}

fn weighted_feedback_edges(
    members: &[DependencyNode],
    relations: &[DependencyRelation],
) -> Vec<DependencyRelation> {
    let mut remaining = members.iter().cloned().collect::<BTreeSet<_>>();
    let mut left = Vec::new();
    let mut right = Vec::new();
    while !remaining.is_empty() {
        let weighted = weighted_degrees(&remaining, relations);
        if let Some(sink) = remaining
            .iter()
            .find(|node| weighted.get(*node).is_none_or(|(_, out)| *out == 0))
            .cloned()
        {
            remaining.remove(&sink);
            right.push(sink);
            continue;
        }
        if let Some(source) = remaining
            .iter()
            .find(|node| weighted.get(*node).is_none_or(|(input, _)| *input == 0))
            .cloned()
        {
            remaining.remove(&source);
            left.push(source);
            continue;
        }
        let selected = remaining
            .iter()
            .max_by(|left_node, right_node| {
                let (left_in, left_out) = weighted[left_node];
                let (right_in, right_out) = weighted[right_node];
                (left_out as isize - left_in as isize)
                    .cmp(&(right_out as isize - right_in as isize))
                    .then_with(|| right_node.cmp(left_node))
            })
            .cloned()
            .expect("remaining cycle member");
        remaining.remove(&selected);
        left.push(selected);
    }
    right.reverse();
    left.extend(right);
    let positions = left
        .into_iter()
        .enumerate()
        .map(|(index, node)| (node, index))
        .collect::<BTreeMap<_, _>>();
    relations
        .iter()
        .filter(|relation| positions[&relation.source] >= positions[&relation.target])
        .cloned()
        .collect()
}

fn weighted_degrees(
    remaining: &BTreeSet<DependencyNode>,
    relations: &[DependencyRelation],
) -> BTreeMap<DependencyNode, (usize, usize)> {
    let mut degrees = remaining
        .iter()
        .cloned()
        .map(|node| (node, (0, 0)))
        .collect::<BTreeMap<_, _>>();
    for relation in relations {
        if remaining.contains(&relation.source) && remaining.contains(&relation.target) {
            let weight = relation.evidence.len().max(1);
            degrees.get_mut(&relation.source).expect("source").1 += weight;
            degrees.get_mut(&relation.target).expect("target").0 += weight;
        }
    }
    degrees
}

fn is_acyclic_after_cuts(
    members: &[DependencyNode],
    relations: &[DependencyRelation],
    cuts: &[DependencyRelation],
) -> bool {
    let cut_keys = cuts
        .iter()
        .map(|relation| (relation.source.clone(), relation.target.clone()))
        .collect::<BTreeSet<_>>();
    let kept = relations
        .iter()
        .filter(|relation| !cut_keys.contains(&(relation.source.clone(), relation.target.clone())))
        .cloned()
        .collect::<Vec<_>>();
    let adjacency = adjacency(&kept);
    let mut indegree = members
        .iter()
        .cloned()
        .map(|member| (member, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for relation in &kept {
        indegree
            .entry(relation.target.clone())
            .and_modify(|value| *value += 1);
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, value)| **value == 0)
        .map(|(node, _)| node.clone())
        .collect::<BTreeSet<_>>();
    let mut visited = 0;
    while let Some(node) = ready.pop_first() {
        visited += 1;
        for target in adjacency.get(&node).into_iter().flatten() {
            let value = indegree.get_mut(target).expect("target");
            *value -= 1;
            if *value == 0 {
                ready.insert(target.clone());
            }
        }
    }
    visited == members.len()
}

#[cfg(test)]
mod tests {
    use crate::{
        DependencyReference, DependencyResolutionOutcome, DependencyTarget, LanguageId,
        LocalModule, ModuleId, ProjectDependencyResolution, ResolutionLevel, SourcePosition,
        SourceSpan, analyze_dependency_graph,
    };

    use super::*;

    fn module(name: &str) -> LocalModule {
        LocalModule::new(
            ModuleId::new(LanguageId::new("python"), name),
            format!("{name}.py"),
        )
    }

    fn edge(
        source: &LocalModule,
        target: &LocalModule,
        byte: usize,
    ) -> ProjectDependencyResolution {
        ProjectDependencyResolution::new(
            source.path.clone(),
            source.id.clone(),
            DependencyReference::Import(crate::ImportReference {
                module: Some(target.id.qualified_name().to_owned()),
                imported_name: None,
                alias: None,
                relative_level: 0,
                wildcard: false,
                resolution: ResolutionLevel::Syntactic,
                enclosing_symbol: None,
                context: crate::ImportContext::default(),
                span: SourceSpan {
                    start: SourcePosition {
                        line: byte + 1,
                        column: 0,
                    },
                    end: SourcePosition {
                        line: byte + 1,
                        column: 1,
                    },
                    start_byte: byte,
                    end_byte: byte + 1,
                },
            }),
            DependencyResolutionOutcome::exact(DependencyTarget::LocalModule(target.clone())),
        )
    }

    #[test]
    fn chooses_a_short_witness_and_the_lower_weight_cut() {
        let a = module("a");
        let b = module("b");
        let resolutions = vec![
            edge(&a, &b, 0),
            edge(&a, &b, 1),
            edge(&a, &b, 2),
            edge(&b, &a, 3),
        ];
        let analysis = analyze_dependency_graph(&[a, b], &resolutions).expect("graph");
        let explanations = explain_dependency_cycles(&analysis);

        assert_eq!(explanations.len(), 1);
        assert_eq!(explanations[0].witness_nodes.len(), 3);
        assert_eq!(explanations[0].recommended_cuts.len(), 1);
        assert_eq!(explanations[0].recommended_cuts[0].evidence.len(), 1);
        assert!(is_acyclic_after_cuts(
            &explanations[0].members,
            &analysis.relations,
            &explanations[0].recommended_cuts
        ));
    }

    #[test]
    fn explains_self_cycles() {
        let a = module("a");
        let analysis =
            analyze_dependency_graph(std::slice::from_ref(&a), &[edge(&a, &a, 0)]).expect("graph");
        let explanation = explain_dependency_cycles(&analysis).remove(0);
        assert_eq!(explanation.witness_nodes.len(), 2);
        assert_eq!(explanation.witness_relations.len(), 1);
        assert_eq!(explanation.recommended_cuts.len(), 1);
    }

    #[test]
    fn overlapping_loops_are_deterministic_and_the_complete_cut_set_is_acyclic() {
        let a = module("a");
        let b = module("b");
        let c = module("c");
        let mut resolutions = vec![
            edge(&a, &b, 0),
            edge(&b, &a, 1),
            edge(&a, &c, 2),
            edge(&c, &a, 3),
            edge(&b, &c, 4),
        ];
        let analysis = analyze_dependency_graph(&[a.clone(), b.clone(), c.clone()], &resolutions)
            .expect("graph");
        let first = explain_dependency_cycles(&analysis).remove(0);
        resolutions.reverse();
        let reordered = analyze_dependency_graph(&[c, b, a], &resolutions).expect("graph");
        let second = explain_dependency_cycles(&reordered).remove(0);

        assert_eq!(first, second);
        assert_eq!(
            first
                .witness_nodes
                .iter()
                .map(|node| match node {
                    DependencyNode::LocalModule(module) => module.id.qualified_name(),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>(),
            ["a", "b", "a"]
        );
        assert!(is_acyclic_after_cuts(
            &first.members,
            &analysis.relations,
            &first.recommended_cuts
        ));
    }
}
