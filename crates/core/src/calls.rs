//! Language-neutral project call identities, resolution outcomes, and graphs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::PathBuf;

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use crate::{
    CallReference, CallableSignature, LanguageId, LanguageModule, ModuleExport, ModuleId,
    ModuleImport, SourceSpan, SymbolKind,
};

pub const CALL_GRAPH_DEFINITION_VERSION: &str = "call-graph-v2";
pub const CALL_FAN_IN_DEFINITION_VERSION: &str = "call-fan-in-v2";
pub const CALL_FAN_OUT_DEFINITION_VERSION: &str = "call-fan-out-v2";
pub const CALL_SCC_DEFINITION_VERSION: &str = "call-scc-v2";
pub const CALL_REPORT_SCHEMA_VERSION: &str = "0.2.0";

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectSymbolId {
    pub language: LanguageId,
    pub module: ModuleId,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub duplicate_ordinal: usize,
}

impl ProjectSymbolId {
    pub fn base_selector(&self) -> String {
        if self.language.as_str() == "cpp" {
            self.qualified_name.clone()
        } else {
            format!("{}::{}", self.module.qualified_name(), self.qualified_name)
        }
    }

    pub fn ordinal_selector(&self) -> String {
        format!("{}#{}", self.base_selector(), self.duplicate_ordinal)
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum SymbolLinkStatus {
    Linked,
    DeclarationOnly,
    DefinitionOnly,
    Ambiguous,
    Unavailable,
}

impl SymbolLinkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::DeclarationOnly => "declaration-only",
            Self::DefinitionOnly => "definition-only",
            Self::Ambiguous => "ambiguous",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectSymbolLocation {
    pub path: PathBuf,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectSymbol {
    // Call relations share immutable flow metadata instead of copying the whole tree.
    pub call_flow: Option<std::sync::Arc<crate::CallFlow>>,
    pub id: ProjectSymbolId,
    pub path: PathBuf,
    pub span: SourceSpan,
    pub signature: Option<CallableSignature>,
    pub declarations: Vec<ProjectSymbolLocation>,
    pub definition: Option<ProjectSymbolLocation>,
    pub link_status: SymbolLinkStatus,
    pub language_module: Option<String>,
    pub architecture_groups: Vec<String>,
    pub primary_architecture_group: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CallResolutionOutcome {
    Exact(ProjectSymbol),
    Inferred {
        target: ProjectSymbol,
        alternatives: Vec<ProjectSymbol>,
        reason: String,
    },
    Ambiguous(Vec<ProjectSymbol>),
    External(String),
    Unresolved(String),
    Unavailable(String),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProjectCallResolution {
    pub source: ProjectSymbol,
    pub source_path: PathBuf,
    pub reference: CallReference,
    pub outcome: CallResolutionOutcome,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ProjectLanguageModule {
    pub path: PathBuf,
    pub module: LanguageModule,
    pub imports: Vec<ModuleImport>,
    pub exports: Vec<ModuleExport>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum CallNode {
    LocalSymbol(Box<ProjectSymbol>),
    External {
        source: ProjectSymbolId,
        name: String,
    },
    Ambiguous {
        source: ProjectSymbolId,
        spelling: String,
        candidates: Vec<ProjectSymbol>,
    },
    Unresolved {
        source: ProjectSymbolId,
        spelling: String,
    },
    Unavailable {
        source: ProjectSymbolId,
        spelling: String,
    },
}

impl CallNode {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::LocalSymbol(_) => "local-symbol",
            Self::External { .. } => "external",
            Self::Ambiguous { .. } => "ambiguous",
            Self::Unresolved { .. } => "unresolved",
            Self::Unavailable { .. } => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum CallRelationKind {
    Exact,
    Inferred,
    External,
    Ambiguous,
    Unresolved,
    Unavailable,
}

impl CallRelationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Inferred => "inferred",
            Self::External => "external",
            Self::Ambiguous => "ambiguous",
            Self::Unresolved => "unresolved",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallEvidence {
    pub source_path: PathBuf,
    pub reference: CallReference,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallRelation {
    pub source: CallNode,
    pub target: CallNode,
    pub kind: CallRelationKind,
    pub evidence: Vec<CallEvidence>,
    pub alternatives: Vec<ProjectSymbol>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallNodeMetrics {
    pub node: CallNode,
    pub fan_in: usize,
    pub fan_out: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallScc {
    pub members: Vec<CallNode>,
    pub cyclic: bool,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct CallGraphCoverage {
    pub total_calls: usize,
    pub exact_calls: usize,
    pub inferred_calls: usize,
    pub external_calls: usize,
    pub ambiguous_calls: usize,
    pub unresolved_calls: usize,
    pub unavailable_calls: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallGraphAnalysis {
    pub nodes: Vec<CallNode>,
    pub relations: Vec<CallRelation>,
    pub metrics: Vec<CallNodeMetrics>,
    pub strongly_connected_components: Vec<CallScc>,
    pub cycles: Vec<CallScc>,
    pub coverage: CallGraphCoverage,
    pub language_modules: Vec<ProjectLanguageModule>,
}

pub fn analyze_call_graph(
    symbols: &[ProjectSymbol],
    resolutions: &[ProjectCallResolution],
) -> CallGraphAnalysis {
    analyze_call_graph_with_modules(symbols, resolutions, Vec::new())
}

pub fn analyze_call_graph_with_modules(
    symbols: &[ProjectSymbol],
    resolutions: &[ProjectCallResolution],
    language_modules: Vec<ProjectLanguageModule>,
) -> CallGraphAnalysis {
    let mut nodes = symbols
        .iter()
        .cloned()
        .map(|symbol| CallNode::LocalSymbol(Box::new(symbol)))
        .collect::<BTreeSet<_>>();
    let mut grouped = BTreeMap::<
        (CallNode, CallNode, CallRelationKind),
        (Vec<CallEvidence>, BTreeSet<ProjectSymbol>, BTreeSet<String>),
    >::new();
    let mut coverage = CallGraphCoverage {
        total_calls: resolutions.len(),
        ..Default::default()
    };
    for resolution in resolutions {
        let source = CallNode::LocalSymbol(Box::new(resolution.source.clone()));
        let (target, kind, alternatives, reason) = match &resolution.outcome {
            CallResolutionOutcome::Exact(target) => {
                coverage.exact_calls += 1;
                (
                    CallNode::LocalSymbol(Box::new(target.clone())),
                    CallRelationKind::Exact,
                    Vec::new(),
                    None,
                )
            }
            CallResolutionOutcome::Inferred {
                target,
                alternatives,
                reason,
            } => {
                coverage.inferred_calls += 1;
                (
                    CallNode::LocalSymbol(Box::new(target.clone())),
                    CallRelationKind::Inferred,
                    alternatives.clone(),
                    Some(reason.clone()),
                )
            }
            CallResolutionOutcome::External(name) => {
                coverage.external_calls += 1;
                (
                    CallNode::External {
                        source: resolution.source.id.clone(),
                        name: name.clone(),
                    },
                    CallRelationKind::External,
                    Vec::new(),
                    None,
                )
            }
            CallResolutionOutcome::Ambiguous(candidates) => {
                coverage.ambiguous_calls += 1;
                (
                    CallNode::Ambiguous {
                        source: resolution.source.id.clone(),
                        spelling: resolution.reference.callee.clone(),
                        candidates: candidates.clone(),
                    },
                    CallRelationKind::Ambiguous,
                    candidates.clone(),
                    Some("multiple equally plausible local targets".to_owned()),
                )
            }
            CallResolutionOutcome::Unresolved(spelling) => {
                coverage.unresolved_calls += 1;
                (
                    CallNode::Unresolved {
                        source: resolution.source.id.clone(),
                        spelling: spelling.clone(),
                    },
                    CallRelationKind::Unresolved,
                    Vec::new(),
                    Some(spelling.clone()),
                )
            }
            CallResolutionOutcome::Unavailable(reason) => {
                coverage.unavailable_calls += 1;
                (
                    CallNode::Unavailable {
                        source: resolution.source.id.clone(),
                        spelling: resolution.reference.callee.clone(),
                    },
                    CallRelationKind::Unavailable,
                    Vec::new(),
                    Some(reason.clone()),
                )
            }
        };
        nodes.insert(target.clone());
        let group = grouped.entry((source, target, kind)).or_default();
        group.0.push(CallEvidence {
            source_path: resolution.source_path.clone(),
            reference: resolution.reference.clone(),
        });
        group.1.extend(alternatives);
        if let Some(reason) = reason {
            group.2.insert(reason);
        }
    }
    let relations = grouped
        .into_iter()
        .map(
            |((source, target, kind), (mut evidence, alternatives, reasons))| {
                evidence.sort_by_key(|item| item.reference.span.start_byte);
                CallRelation {
                    source,
                    target,
                    kind,
                    evidence,
                    alternatives: alternatives.into_iter().collect(),
                    reason: (!reasons.is_empty())
                        .then(|| reasons.into_iter().collect::<Vec<_>>().join("; ")),
                }
            },
        )
        .collect::<Vec<_>>();
    let nodes = nodes.into_iter().collect::<Vec<_>>();
    let edges = relations
        .iter()
        .filter(|relation| relation.kind == CallRelationKind::Exact)
        .map(|relation| (relation.source.clone(), relation.target.clone()))
        .collect::<BTreeSet<_>>();
    let metrics = nodes
        .iter()
        .map(|node| CallNodeMetrics {
            node: node.clone(),
            fan_in: edges.iter().filter(|(_, target)| target == node).count(),
            fan_out: edges.iter().filter(|(source, _)| source == node).count(),
        })
        .collect();
    let strongly_connected_components = call_sccs(symbols, &edges);
    let cycles = strongly_connected_components
        .iter()
        .filter(|component| component.cyclic)
        .cloned()
        .collect();
    CallGraphAnalysis {
        nodes,
        relations,
        metrics,
        strongly_connected_components,
        cycles,
        coverage,
        language_modules,
    }
}

fn call_sccs(symbols: &[ProjectSymbol], edges: &BTreeSet<(CallNode, CallNode)>) -> Vec<CallScc> {
    let mut graph = DiGraph::<CallNode, ()>::new();
    let indexes = symbols
        .iter()
        .cloned()
        .map(|symbol| CallNode::LocalSymbol(Box::new(symbol)))
        .map(|node| {
            let index = graph.add_node(node.clone());
            (node, index)
        })
        .collect::<BTreeMap<_, _>>();
    for (source, target) in edges {
        if let (Some(source), Some(target)) = (indexes.get(source), indexes.get(target)) {
            graph.update_edge(*source, *target, ());
        }
    }
    let self_edges = edges
        .iter()
        .filter(|(source, target)| source == target)
        .map(|(source, _)| source.clone())
        .collect::<BTreeSet<_>>();
    let mut result = tarjan_scc(&graph)
        .into_iter()
        .map(|component| {
            let mut members = component
                .into_iter()
                .map(|index| graph[index].clone())
                .collect::<Vec<_>>();
            members.sort();
            let cyclic = members.len() > 1 || self_edges.contains(&members[0]);
            CallScc { members, cyclic }
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.members.cmp(&right.members));
    result
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CallDirection {
    Callers,
    Callees,
    Both,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallGraphFilter {
    pub focus_symbols: Vec<String>,
    pub direction: CallDirection,
    pub depth: usize,
    pub exact_only: bool,
    pub local_only: bool,
    pub cycles_only: bool,
}

impl Default for CallGraphFilter {
    fn default() -> Self {
        Self {
            focus_symbols: Vec::new(),
            direction: CallDirection::Both,
            depth: 1,
            exact_only: false,
            local_only: false,
            cycles_only: false,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallGraphViewNode {
    pub id: String,
    pub node: CallNode,
    pub fan_in: usize,
    pub fan_out: usize,
    pub cyclic_component: Option<usize>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallGraphView {
    pub filter: CallGraphFilter,
    pub nodes: Vec<CallGraphViewNode>,
    pub relations: Vec<CallRelation>,
    pub strongly_connected_components: Vec<CallScc>,
    pub language_modules: Vec<ProjectLanguageModule>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallGraphFilterError(pub String);
impl fmt::Display for CallGraphFilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CallGraphFilterError {}

pub fn filter_call_graph(
    analysis: &CallGraphAnalysis,
    filter: &CallGraphFilter,
) -> Result<CallGraphView, CallGraphFilterError> {
    let ids = analysis
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.clone(), format!("c{index:04}")))
        .collect::<BTreeMap<_, _>>();
    let local = analysis
        .nodes
        .iter()
        .filter_map(|node| match node {
            CallNode::LocalSymbol(symbol) => Some((symbol.id.base_selector(), node.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let relations = analysis
        .relations
        .iter()
        .filter(|relation| !filter.exact_only || relation.kind == CallRelationKind::Exact)
        .cloned()
        .collect::<Vec<_>>();
    let mut selected = if filter.cycles_only {
        analysis
            .cycles
            .iter()
            .flat_map(|component| component.members.iter().cloned())
            .collect()
    } else if filter.focus_symbols.is_empty() {
        analysis.nodes.iter().cloned().collect()
    } else {
        let mut seeds = BTreeSet::new();
        for requested in &filter.focus_symbols {
            let matches = local.iter().filter(|(selector, node)| selector == requested || matches!(node, CallNode::LocalSymbol(symbol) if symbol.id.ordinal_selector() == *requested)).map(|(_, node)| node.clone()).collect::<Vec<_>>();
            if matches.len() > 1 && !requested.contains('#') {
                return Err(CallGraphFilterError(format!(
                    "symbol {requested:?} has duplicate definitions; select one with #N"
                )));
            }
            if matches.is_empty() {
                return Err(CallGraphFilterError(format!(
                    "symbol {requested:?} was not found"
                )));
            }
            seeds.extend(matches);
        }
        traverse(&seeds, &relations, filter.direction, filter.depth)
    };
    for cycle in &analysis.cycles {
        if cycle.members.iter().any(|node| selected.contains(node)) {
            selected.extend(cycle.members.iter().cloned());
        }
    }
    let candidate_nodes = relations
        .iter()
        .filter(|relation| {
            selected.contains(&relation.source) || selected.contains(&relation.target)
        })
        .flat_map(|relation| relation.alternatives.iter().cloned())
        .map(|symbol| CallNode::LocalSymbol(Box::new(symbol)))
        .collect::<Vec<_>>();
    selected.extend(candidate_nodes);
    if filter.exact_only || filter.local_only || filter.cycles_only {
        selected.retain(|node| matches!(node, CallNode::LocalSymbol(_)));
    }
    let relations = relations
        .into_iter()
        .filter(|relation| {
            selected.contains(&relation.source) && selected.contains(&relation.target)
        })
        .collect();
    let metrics = analysis
        .metrics
        .iter()
        .map(|metric| (metric.node.clone(), metric))
        .collect::<BTreeMap<_, _>>();
    let cycle_numbers = analysis
        .cycles
        .iter()
        .enumerate()
        .flat_map(|(index, cycle)| {
            cycle
                .members
                .iter()
                .cloned()
                .map(move |node| (node, index + 1))
        })
        .collect::<BTreeMap<_, _>>();
    let nodes = analysis
        .nodes
        .iter()
        .filter(|node| selected.contains(*node))
        .map(|node| CallGraphViewNode {
            id: ids[node].clone(),
            node: node.clone(),
            fan_in: metrics[node].fan_in,
            fan_out: metrics[node].fan_out,
            cyclic_component: cycle_numbers.get(node).copied(),
        })
        .collect();
    let strongly_connected_components = analysis
        .strongly_connected_components
        .iter()
        .filter(|component| component.members.iter().all(|node| selected.contains(node)))
        .cloned()
        .collect();
    Ok(CallGraphView {
        filter: filter.clone(),
        nodes,
        relations,
        strongly_connected_components,
        language_modules: analysis.language_modules.clone(),
    })
}

/// Return a deterministic shortest path over exact local call relations.
pub fn shortest_call_path(
    analysis: &CallGraphAnalysis,
    from: &str,
    to: &str,
) -> Result<Option<Vec<CallNode>>, CallGraphFilterError> {
    let start = resolve_call_selector(analysis, from)?;
    let target = resolve_call_selector(analysis, to)?;
    let adjacency = call_adjacency(analysis, CallDirection::Callees);
    let mut queue = VecDeque::from([start.clone()]);
    let mut previous = BTreeMap::from([(start, None::<CallNode>)]);
    while let Some(node) = queue.pop_front() {
        if node == target {
            let mut path = vec![node.clone()];
            let mut cursor = &node;
            while let Some(Some(parent)) = previous.get(cursor) {
                path.push(parent.clone());
                cursor = parent;
            }
            path.reverse();
            return Ok(Some(path));
        }
        for neighbor in adjacency.get(&node).into_iter().flatten() {
            if !previous.contains_key(neighbor) {
                previous.insert(neighbor.clone(), Some(node.clone()));
                queue.push_back(neighbor.clone());
            }
        }
    }
    Ok(None)
}

/// Return a sorted exact-local caller or callee closure, including the root.
pub fn call_closure(
    analysis: &CallGraphAnalysis,
    root: &str,
    direction: CallDirection,
) -> Result<Vec<CallNode>, CallGraphFilterError> {
    if direction == CallDirection::Both {
        return Err(CallGraphFilterError(
            "call closure direction must be callers or callees, not both".to_owned(),
        ));
    }
    let root = resolve_call_selector(analysis, root)?;
    let adjacency = call_adjacency(analysis, direction);
    let mut selected = BTreeSet::from([root.clone()]);
    let mut queue = VecDeque::from([root]);
    while let Some(node) = queue.pop_front() {
        for neighbor in adjacency.get(&node).into_iter().flatten() {
            if selected.insert(neighbor.clone()) {
                queue.push_back(neighbor.clone());
            }
        }
    }
    Ok(selected.into_iter().collect())
}

fn resolve_call_selector(
    analysis: &CallGraphAnalysis,
    requested: &str,
) -> Result<CallNode, CallGraphFilterError> {
    let matches = analysis
        .nodes
        .iter()
        .filter(|node| match node {
            CallNode::LocalSymbol(symbol) => {
                symbol.id.base_selector() == requested || symbol.id.ordinal_selector() == requested
            }
            _ => false,
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [node] => Ok(node.clone()),
        [] => Err(CallGraphFilterError(format!(
            "symbol {requested:?} was not found"
        ))),
        _ => Err(CallGraphFilterError(format!(
            "symbol {requested:?} has duplicate definitions; select one with #N"
        ))),
    }
}

fn call_adjacency(
    analysis: &CallGraphAnalysis,
    direction: CallDirection,
) -> BTreeMap<CallNode, Vec<CallNode>> {
    let mut adjacency = BTreeMap::<CallNode, BTreeSet<CallNode>>::new();
    for relation in &analysis.relations {
        if relation.kind != CallRelationKind::Exact {
            continue;
        }
        let (source, target) = match direction {
            CallDirection::Callees => (&relation.source, &relation.target),
            CallDirection::Callers => (&relation.target, &relation.source),
            CallDirection::Both => continue,
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

fn traverse(
    seeds: &BTreeSet<CallNode>,
    relations: &[CallRelation],
    direction: CallDirection,
    depth: usize,
) -> BTreeSet<CallNode> {
    let mut selected = seeds.clone();
    let mut queue = seeds
        .iter()
        .cloned()
        .map(|node| (node, 0))
        .collect::<VecDeque<_>>();
    while let Some((node, distance)) = queue.pop_front() {
        if distance >= depth {
            continue;
        }
        for relation in relations {
            let neighbor = if matches!(direction, CallDirection::Callees | CallDirection::Both)
                && relation.source == node
            {
                Some(&relation.target)
            } else if matches!(direction, CallDirection::Callers | CallDirection::Both)
                && relation.target == node
            {
                Some(&relation.source)
            } else {
                None
            };
            if let Some(neighbor) = neighbor
                && selected.insert(neighbor.clone())
            {
                queue.push_back((neighbor.clone(), distance + 1));
            }
        }
    }
    selected
}

pub fn call_node_name(node: &CallNode) -> String {
    match node {
        CallNode::LocalSymbol(symbol) => symbol.id.base_selector(),
        CallNode::External { name, .. } => name.clone(),
        CallNode::Ambiguous { spelling, .. }
        | CallNode::Unresolved { spelling, .. }
        | CallNode::Unavailable { spelling, .. } => spelling.clone(),
    }
}

pub fn render_call_mermaid(view: &CallGraphView) -> String {
    let mut output = String::from("flowchart LR\n");
    let ids = view
        .nodes
        .iter()
        .map(|node| (node.node.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    for node in &view.nodes {
        output.push_str(&format!(
            "  {}[\"{}\"]\n",
            node.id,
            escape_mermaid(&call_node_name(&node.node))
        ));
    }
    for relation in &view.relations {
        let arrow = match relation.kind {
            CallRelationKind::Exact => "-->|exact|",
            CallRelationKind::Inferred => "-. inferred .->",
            CallRelationKind::Ambiguous => "-. ambiguous .->",
            CallRelationKind::External => "-. external .->",
            CallRelationKind::Unresolved => "-. unresolved .->",
            CallRelationKind::Unavailable => "-. unavailable .->",
        };
        output.push_str(&format!(
            "  {} {arrow} {}\n",
            ids[&relation.source], ids[&relation.target]
        ));
    }
    output
}

pub fn render_call_dot(view: &CallGraphView) -> String {
    let mut output = String::from("digraph call_graph {\n  rankdir=LR;\n");
    let ids = view
        .nodes
        .iter()
        .map(|node| (node.node.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    for node in &view.nodes {
        output.push_str(&format!(
            "  {} [label=\"{}\"];\n",
            node.id,
            escape_dot(&call_node_name(&node.node))
        ));
    }
    for relation in &view.relations {
        let attributes = match relation.kind {
            CallRelationKind::Exact => "label=\"exact\",color=\"#16805c\"",
            CallRelationKind::Inferred => "label=\"inferred\",style=dashed,color=\"#a76210\"",
            CallRelationKind::Ambiguous => "label=\"ambiguous\",style=dashed,color=\"#b93443\"",
            CallRelationKind::External => "label=\"external\",style=dashed",
            CallRelationKind::Unresolved => "label=\"unresolved\",style=dashed,color=\"#b93443\"",
            CallRelationKind::Unavailable => "label=\"unavailable\",style=dotted,color=\"#b93443\"",
        };
        output.push_str(&format!(
            "  {} -> {} [{attributes}];\n",
            ids[&relation.source], ids[&relation.target]
        ));
    }
    output.push_str("}\n");
    output
}

fn escape_mermaid(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn escape_dot(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallArgumentShape, CallReference, SourcePosition};

    fn symbol(name: &str) -> ProjectSymbol {
        let module = ModuleId::new(LanguageId::new("python"), "pkg.mod");
        ProjectSymbol {
            call_flow: None,
            id: ProjectSymbolId {
                language: LanguageId::new("python"),
                module,
                qualified_name: name.to_owned(),
                kind: SymbolKind::Function,
                duplicate_ordinal: 1,
            },
            path: PathBuf::from("src/pkg/mod.py"),
            span: span(0),
            signature: None,
            declarations: Vec::new(),
            definition: Some(ProjectSymbolLocation {
                path: PathBuf::from("src/pkg/mod.py"),
                span: span(0),
            }),
            link_status: SymbolLinkStatus::DefinitionOnly,
            language_module: None,
            architecture_groups: Vec::new(),
            primary_architecture_group: None,
        }
    }

    fn span(byte: usize) -> SourceSpan {
        SourceSpan {
            start_byte: byte,
            end_byte: byte + 1,
            start: SourcePosition {
                line: byte + 1,
                column: 1,
            },
            end: SourcePosition {
                line: byte + 1,
                column: 2,
            },
        }
    }

    fn resolution(
        source: &ProjectSymbol,
        callee: &str,
        outcome: CallResolutionOutcome,
        byte: usize,
    ) -> ProjectCallResolution {
        ProjectCallResolution {
            source: source.clone(),
            source_path: source.path.clone(),
            reference: CallReference {
                expression: format!("{callee}()"),
                callee: callee.to_owned(),
                components: vec![callee.to_owned()],
                enclosing_symbol: None,
                arguments: CallArgumentShape {
                    positional: 0,
                    keywords: Vec::new(),
                    has_star_args: false,
                    has_star_kwargs: false,
                },
                argument_details: Vec::new(),
                form: crate::CallForm::Unknown,
                receiver: None,
                receiver_type_hint: None,
                span: span(byte),
                syntax_complete: true,
                preprocessing_uncertain: false,
            },
            outcome,
        }
    }

    #[test]
    fn builds_recursive_sccs_and_preserves_uncertain_calls() {
        let a = symbol("a");
        let b = symbol("b");
        let graph = analyze_call_graph(
            &[a.clone(), b.clone()],
            &[
                resolution(&a, "b", CallResolutionOutcome::Exact(b.clone()), 0),
                resolution(&b, "a", CallResolutionOutcome::Exact(a.clone()), 1),
                resolution(
                    &a,
                    "dynamic",
                    CallResolutionOutcome::Unresolved("dynamic".to_owned()),
                    2,
                ),
            ],
        );

        assert_eq!(graph.coverage.exact_calls, 2);
        assert_eq!(graph.coverage.unresolved_calls, 1);
        assert_eq!(graph.cycles.len(), 1);
        let view = filter_call_graph(
            &graph,
            &CallGraphFilter {
                focus_symbols: vec!["pkg.mod::a".to_owned()],
                depth: 0,
                ..CallGraphFilter::default()
            },
        )
        .expect("view");
        assert_eq!(view.nodes.len(), 2, "focused cycles are complete");
        assert_eq!(
            shortest_call_path(&graph, "pkg.mod::a", "pkg.mod::b")
                .expect("path")
                .expect("connected")
                .len(),
            2
        );
        assert_eq!(
            call_closure(&graph, "pkg.mod::a", CallDirection::Callees)
                .expect("closure")
                .len(),
            2
        );
    }
}
