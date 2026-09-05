//! Self-contained interactive dependency graph rendering.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::calls::{CallGraphView, CallNode, CallRelation, ProjectLanguageModule, ProjectSymbol};
use crate::dependencies::DependencyTarget;
use crate::dependency_cycles::explain_dependency_cycle;
use crate::dependency_hierarchy::{DependencyHierarchyMember, build_dependency_hierarchy};
use crate::dependency_output::{DependencyGraphView, DependencyGraphViewNode};
use crate::dependency_query::{DependencyGraphQuery, DependencyGraphQueryResult};
use crate::graph::{DependencyNode, DependencyRelation};

const VIEWER_TEMPLATE: &str = include_str!("dependency_viewer.html");
const CALL_VIEWER_TEMPLATE: &str = include_str!("call_viewer.html");
const GRAPH_DATA_MARKER: &str = "__CODEGRAIDE_GRAPH_DATA__";

#[derive(Debug, Serialize)]
struct CallExplorerGraph {
    flow_definition: &'static str,
    max_expansion_depth: u8,
    initial_selection: Option<String>,
    nodes: Vec<CallExplorerNode>,
    relations: Vec<CallExplorerRelation>,
}

#[derive(Debug, Serialize)]
struct CallExplorerNode {
    call_flow: Option<crate::CallFlow>,
    id: String,
    name: String,
    short_name: String,
    kind: String,
    path: Option<String>,
    signature: Option<String>,
    parameters: Vec<String>,
    link_status: Option<&'static str>,
    declarations: Vec<String>,
    definition: Option<String>,
    module: Option<String>,
    module_imports: Vec<String>,
    module_exports: Vec<String>,
    architecture_groups: Vec<String>,
    primary_architecture_group: Option<String>,
    fan_in: usize,
    fan_out: usize,
    cyclic_component: Option<usize>,
    candidates: Vec<CallExplorerCandidate>,
    occurrences: Vec<CallExplorerOccurrence>,
    source: Option<HtmlSource>,
    noise: bool,
}

#[derive(Debug, Serialize)]
struct CallExplorerOccurrence {
    kind: &'static str,
    label: String,
    source: HtmlSource,
}

#[derive(Debug, Serialize)]
struct CallExplorerCandidate {
    id: Option<String>,
    name: String,
    signature: Option<String>,
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct CallExplorerRelation {
    source: String,
    target: String,
    status: &'static str,
    reason: Option<String>,
    alternatives: Vec<CallExplorerCandidate>,
    evidence: Vec<CallExplorerEvidence>,
}

#[derive(Debug, Serialize)]
struct CallExplorerEvidence {
    path: String,
    line: usize,
    column: usize,
    expression: String,
    callee: String,
    form: &'static str,
    arguments: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HtmlGraph {
    graph_kind: &'static str,
    nodes: Vec<HtmlNode>,
    relations: Vec<HtmlRelation>,
    cycles: Vec<HtmlCycle>,
    hierarchy: Vec<HtmlHierarchyGroup>,
    query: Option<HtmlQuery>,
}

#[derive(Debug, Serialize)]
struct HtmlNode {
    id: String,
    name: String,
    short_name: String,
    kind: &'static str,
    subtitle: String,
    path: Option<String>,
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outgoing_dependencies_analyzed: Option<bool>,
    unresolved_reason: Option<&'static str>,
    candidates: Vec<HtmlCandidate>,
    cyclic_component: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<HtmlSource>,
}

#[derive(Debug, Clone, Serialize)]
struct HtmlSource {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    lines: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HtmlCandidate {
    kind: &'static str,
    name: String,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
struct HtmlRelation {
    source: String,
    target: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_basis: Option<&'static str>,
    evidence: Vec<HtmlEvidence>,
}

#[derive(Debug, Serialize)]
struct HtmlEvidence {
    source_path: String,
    import_name: String,
    line: usize,
    column: usize,
    scope: &'static str,
    usage: &'static str,
    requirement: &'static str,
    conditional: bool,
}

#[derive(Debug, Serialize)]
struct HtmlCycle {
    number: usize,
    members: Vec<String>,
    witness_nodes: Vec<String>,
    witness_relations: Vec<HtmlRelation>,
    recommended_cuts: Vec<HtmlRelation>,
}

#[derive(Debug, Serialize)]
struct HtmlHierarchyGroup {
    id: String,
    name: String,
    qualified_name: String,
    parent: Option<String>,
    direct_members: Vec<String>,
    members: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HtmlQuery {
    kind: &'static str,
    found: bool,
    label: String,
    ordered_nodes: Vec<String>,
}

#[derive(Debug)]
pub enum CallHtmlSourceError {
    Read { path: PathBuf, source: io::Error },
    Serialize(serde_json::Error),
}

impl fmt::Display for CallHtmlSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "could not read source file {}: {source}",
                    path.display()
                )
            }
            Self::Serialize(source) => {
                write!(formatter, "could not serialize call graph: {source}")
            }
        }
    }
}

impl std::error::Error for CallHtmlSourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Serialize(source) => Some(source),
        }
    }
}

/// Render a complete, standalone dependency explorer.
///
/// The document contains its graph data, styles, and interaction code, so it
/// can be opened directly from disk without Mermaid, Graphviz, Node, or a web
/// server.
pub fn render_dependency_html(view: &DependencyGraphView) -> Result<String, serde_json::Error> {
    render_dependency_html_with_query(view, None)
}

pub fn render_dependency_html_with_query(
    view: &DependencyGraphView,
    query: Option<&DependencyGraphQueryResult>,
) -> Result<String, serde_json::Error> {
    let ids = view
        .nodes
        .iter()
        .map(|node| (node.node.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let graph = HtmlGraph {
        graph_kind: "dependencies",
        nodes: view.nodes.iter().map(html_node).collect(),
        relations: view
            .relations
            .iter()
            .map(|relation| html_relation(relation, &ids))
            .collect(),
        cycles: view
            .strongly_connected_components
            .iter()
            .filter(|component| component.cyclic)
            .enumerate()
            .map(|(index, component)| {
                let number = component
                    .members
                    .iter()
                    .find_map(|member| {
                        view.nodes
                            .iter()
                            .find(|node| &node.node == member)
                            .and_then(|node| node.cyclic_component)
                    })
                    .unwrap_or(index + 1);
                let explanation = explain_dependency_cycle(number, component, &view.relations);
                HtmlCycle {
                    number,
                    members: component
                        .members
                        .iter()
                        .filter_map(|member| ids.get(member).cloned())
                        .collect(),
                    witness_nodes: explanation
                        .witness_nodes
                        .iter()
                        .filter_map(|member| ids.get(member).cloned())
                        .collect(),
                    witness_relations: explanation
                        .witness_relations
                        .iter()
                        .map(|relation| html_relation(relation, &ids))
                        .collect(),
                    recommended_cuts: explanation
                        .recommended_cuts
                        .iter()
                        .map(|relation| html_relation(relation, &ids))
                        .collect(),
                }
            })
            .collect(),
        hierarchy: html_hierarchy(view),
        query: query.map(|result| html_query(result, &ids)),
    };
    render_html_graph(&graph)
}

/// Render a call graph through the same offline explorer infrastructure.
pub fn render_call_html(view: &CallGraphView) -> Result<String, serde_json::Error> {
    render_call_html_graph(view, &BTreeMap::new(), 3)
}

/// Render a call graph with repository source embedded for sidebar inspection.
///
/// Source remains an HTML presentation concern: the stable graph and JSON report
/// continue to carry spans and paths without copying repository contents.
pub fn render_call_html_with_source(
    view: &CallGraphView,
    project_root: &Path,
    max_expansion_depth: u8,
) -> Result<String, CallHtmlSourceError> {
    let sources = load_call_sources(view, project_root)?;
    render_call_html_graph(view, &sources, max_expansion_depth)
        .map_err(CallHtmlSourceError::Serialize)
}

fn render_call_html_graph(
    view: &CallGraphView,
    sources: &BTreeMap<String, HtmlSource>,
    max_expansion_depth: u8,
) -> Result<String, serde_json::Error> {
    let ids = view
        .nodes
        .iter()
        .map(|node| (node.node.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let graph = CallExplorerGraph {
        flow_definition: "cpp-structural-flow-v1",
        max_expansion_depth,
        initial_selection: view
            .filter
            .focus_symbols
            .first()
            .and_then(|requested| {
                view.nodes.iter().find_map(|node| match &node.node {
                    CallNode::LocalSymbol(symbol)
                        if symbol.id.base_selector() == *requested
                            || symbol.id.ordinal_selector() == *requested =>
                    {
                        Some(node.id.clone())
                    }
                    _ => None,
                })
            })
            .or_else(|| default_call_selection(view)),
        nodes: view
            .nodes
            .iter()
            .map(|node| {
                call_explorer_node(
                    &node.id,
                    &node.node,
                    node.fan_in,
                    node.fan_out,
                    node.cyclic_component,
                    sources,
                    &view.language_modules,
                )
            })
            .collect(),
        relations: view
            .relations
            .iter()
            .map(|relation| call_explorer_relation(relation, &ids))
            .collect(),
    };
    let data = serde_json::to_string(&graph)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Ok(CALL_VIEWER_TEMPLATE.replace(GRAPH_DATA_MARKER, &data))
}

fn call_explorer_node(
    id: &str,
    node: &CallNode,
    fan_in: usize,
    fan_out: usize,
    cyclic_component: Option<usize>,
    sources: &BTreeMap<String, HtmlSource>,
    language_modules: &[ProjectLanguageModule],
) -> CallExplorerNode {
    let (
        name,
        kind,
        path,
        signature,
        parameters,
        link_status,
        declarations,
        definition,
        module,
        groups,
        primary,
        candidates,
        occurrences,
        source,
    ) = match node {
        CallNode::LocalSymbol(symbol) => {
            let occurrences = symbol_occurrences(symbol, sources);
            let source = symbol
                .definition
                .as_ref()
                .and_then(|location| sources.get(&location_label(location)))
                .cloned()
                .or_else(|| occurrences.first().map(|item| item.source.clone()));
            (
                symbol.id.base_selector(),
                symbol.id.kind.as_str().to_owned(),
                Some(crate::report::json_path(&symbol.path)),
                symbol
                    .signature
                    .as_ref()
                    .map(|signature| signature.normalized_key.clone()),
                symbol
                    .signature
                    .as_ref()
                    .map(|signature| {
                        signature
                            .parameters
                            .iter()
                            .map(|parameter| {
                                parameter.name.clone().unwrap_or_else(|| {
                                    parameter
                                        .type_spelling
                                        .clone()
                                        .unwrap_or_else(|| "?".to_owned())
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                Some(symbol.link_status.as_str()),
                symbol.declarations.iter().map(location_label).collect(),
                symbol.definition.as_ref().map(location_label),
                symbol.language_module.clone(),
                symbol.architecture_groups.clone(),
                symbol.primary_architecture_group.clone(),
                Vec::new(),
                occurrences,
                source,
            )
        }
        CallNode::External { name, .. } => (
            name.clone(),
            "external".to_owned(),
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
            None,
        ),
        CallNode::Ambiguous {
            spelling,
            candidates,
            ..
        } => (
            spelling.clone(),
            "ambiguous".to_owned(),
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            candidates.iter().map(call_explorer_candidate).collect(),
            Vec::new(),
            None,
        ),
        CallNode::Unresolved { spelling, .. } => (
            spelling.clone(),
            "unresolved".to_owned(),
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
            None,
        ),
        CallNode::Unavailable { spelling, .. } => (
            spelling.clone(),
            "unavailable".to_owned(),
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
            None,
        ),
    };
    let short_name = name
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(&name)
        .to_owned();
    let noise = is_low_level_call_node(&name, &kind);
    let module_imports = module
        .as_ref()
        .into_iter()
        .flat_map(|name| {
            language_modules
                .iter()
                .filter(move |item| item.module.name == *name)
        })
        .flat_map(|item| item.imports.iter())
        .map(|import| {
            let prefix = if import.exported {
                "export import"
            } else {
                "import"
            };
            format!("{prefix} {} ({})", import.target, import.kind.as_str())
        })
        .collect::<Vec<_>>();
    let module_exports = module
        .as_ref()
        .into_iter()
        .flat_map(|name| {
            language_modules
                .iter()
                .filter(move |item| item.module.name == *name)
        })
        .flat_map(|item| item.exports.iter())
        .map(|export| export.target.clone())
        .collect::<Vec<_>>();
    CallExplorerNode {
        call_flow: match node {
            CallNode::LocalSymbol(symbol) if source.is_some() => symbol.call_flow.clone(),
            _ => None,
        },
        id: id.to_owned(),
        name,
        short_name,
        kind,
        path,
        signature,
        parameters,
        link_status,
        declarations,
        definition,
        module,
        module_imports,
        module_exports,
        architecture_groups: groups,
        primary_architecture_group: primary,
        fan_in,
        fan_out,
        cyclic_component,
        candidates,
        occurrences,
        source,
        noise,
    }
}

fn default_call_selection(view: &CallGraphView) -> Option<String> {
    view.nodes
        .iter()
        .filter(|node| match &node.node {
            CallNode::LocalSymbol(symbol) => {
                matches!(
                    symbol.id.kind,
                    crate::SymbolKind::Function | crate::SymbolKind::Method
                ) && !is_low_level_call_node(&symbol.id.base_selector(), symbol.id.kind.as_str())
            }
            _ => false,
        })
        .max_by(|left, right| {
            let rank = |node: &crate::calls::CallGraphViewNode| {
                let implementation_path = match &node.node {
                    CallNode::LocalSymbol(symbol) => {
                        !["test", "tests", "sample", "samples", "example", "examples"]
                            .iter()
                            .any(|prefix| symbol.path.starts_with(prefix))
                    }
                    _ => false,
                };
                let outgoing = view
                    .relations
                    .iter()
                    .filter(|relation| {
                        relation.source == node.node
                            && matches!(
                                relation.kind,
                                crate::calls::CallRelationKind::Exact
                                    | crate::calls::CallRelationKind::Inferred
                            )
                    })
                    .count();
                (implementation_path, outgoing, node.fan_in + node.fan_out)
            };
            rank(left).cmp(&rank(right)).then_with(|| {
                crate::call_node_name(&right.node).cmp(&crate::call_node_name(&left.node))
            })
        })
        .map(|node| node.id.clone())
        .or_else(|| {
            view.nodes
                .iter()
                .find(|node| matches!(node.node, CallNode::LocalSymbol(_)))
                .map(|node| node.id.clone())
        })
}

fn is_low_level_call_node(name: &str, kind: &str) -> bool {
    name.contains("<anonymous-")
        || name.contains("<lambda>@")
        || name.starts_with("@file::")
        || name.split("::").any(|part| part.starts_with("DOCTEST_"))
        || name.matches('<').count() != name.matches('>').count()
        || kind == "lambda"
}

fn symbol_occurrences(
    symbol: &ProjectSymbol,
    sources: &BTreeMap<String, HtmlSource>,
) -> Vec<CallExplorerOccurrence> {
    let mut result = symbol
        .declarations
        .iter()
        // A definition is also a declaration; show one source choice at that location.
        .filter(|location| {
            symbol
                .definition
                .as_ref()
                .is_none_or(|definition| location_label(location) != location_label(definition))
        })
        .filter_map(|location| {
            let label = location_label(location);
            sources
                .get(&label)
                .cloned()
                .map(|source| CallExplorerOccurrence {
                    kind: "declaration",
                    label,
                    source,
                })
        })
        .collect::<Vec<_>>();
    if let Some(location) = &symbol.definition {
        let label = location_label(location);
        if let Some(source) = sources.get(&label).cloned() {
            result.push(CallExplorerOccurrence {
                kind: "definition",
                label,
                source,
            });
        }
    }
    result
}

fn location_label(location: &crate::ProjectSymbolLocation) -> String {
    format!(
        "{}:{}:{}",
        crate::report::json_path(&location.path),
        location.span.start.line,
        location.span.start.column
    )
}

fn call_explorer_candidate(symbol: &ProjectSymbol) -> CallExplorerCandidate {
    CallExplorerCandidate {
        id: None,
        name: symbol.id.ordinal_selector(),
        signature: symbol
            .signature
            .as_ref()
            .map(|signature| signature.normalized_key.clone()),
        path: Some(crate::report::json_path(&symbol.path)),
    }
}

fn call_explorer_relation(
    relation: &CallRelation,
    ids: &BTreeMap<CallNode, String>,
) -> CallExplorerRelation {
    let id_by_symbol = ids
        .iter()
        .filter_map(|(node, id)| match node {
            CallNode::LocalSymbol(symbol) => Some((&symbol.id, id)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let alternative = |symbol: &ProjectSymbol| CallExplorerCandidate {
        id: id_by_symbol.get(&symbol.id).map(|id| (*id).clone()),
        name: symbol.id.ordinal_selector(),
        signature: symbol
            .signature
            .as_ref()
            .map(|signature| signature.normalized_key.clone()),
        path: Some(crate::report::json_path(&symbol.path)),
    };
    CallExplorerRelation {
        source: ids[&relation.source].clone(),
        target: ids[&relation.target].clone(),
        status: relation.kind.as_str(),
        reason: relation.reason.clone(),
        alternatives: relation.alternatives.iter().map(alternative).collect(),
        evidence: relation
            .evidence
            .iter()
            .map(|evidence| CallExplorerEvidence {
                path: crate::report::json_path(&evidence.source_path),
                line: evidence.reference.span.start.line,
                column: evidence.reference.span.start.column,
                expression: evidence.reference.expression.clone(),
                callee: evidence
                    .reference
                    .components
                    .last()
                    .cloned()
                    .unwrap_or_default(),
                form: evidence.reference.form.as_str(),
                arguments: evidence
                    .reference
                    .argument_details
                    .iter()
                    .map(|argument| argument.expression.clone())
                    .collect(),
            })
            .collect(),
    }
}

fn load_call_sources(
    view: &CallGraphView,
    project_root: &Path,
) -> Result<BTreeMap<String, HtmlSource>, CallHtmlSourceError> {
    let mut files = BTreeMap::<PathBuf, String>::new();
    let mut sources = BTreeMap::new();
    for symbol in view.nodes.iter().filter_map(|node| match &node.node {
        CallNode::LocalSymbol(symbol) => Some(symbol),
        _ => None,
    }) {
        let mut locations = symbol.declarations.clone();
        locations.extend(symbol.definition.clone());
        if locations.is_empty() {
            locations.push(crate::ProjectSymbolLocation {
                path: symbol.path.clone(),
                span: symbol.span,
            });
        }
        locations.sort();
        locations.dedup();
        for location in locations {
            if !files.contains_key(&location.path) {
                let absolute_path = project_root.join(&location.path);
                let contents = fs::read_to_string(&absolute_path).map_err(|source| {
                    CallHtmlSourceError::Read {
                        path: absolute_path,
                        source,
                    }
                })?;
                files.insert(location.path.clone(), contents);
            }
            if let Some(source) = files
                .get(&location.path)
                .and_then(|contents| source_for_span(contents, location.span))
            {
                sources.insert(location_label(&location), source);
            }
        }
    }
    Ok(sources)
}

fn source_for_span(source: &str, span: crate::SourceSpan) -> Option<HtmlSource> {
    // Byte bounds keep a declaration preview from including adjacent code on its line.
    let excerpt = source.get(span.start_byte..span.end_byte)?;
    let lines = excerpt.lines().map(str::to_owned).collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    Some(HtmlSource {
        start_line: span.start.line,
        start_column: span.start.column,
        end_line: span.start.line + lines.len() - 1,
        lines,
    })
}

fn render_html_graph(graph: &HtmlGraph) -> Result<String, serde_json::Error> {
    let data = serde_json::to_string(graph)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Ok(VIEWER_TEMPLATE.replace(GRAPH_DATA_MARKER, &data))
}

fn html_query(
    result: &DependencyGraphQueryResult,
    ids: &BTreeMap<DependencyNode, String>,
) -> HtmlQuery {
    let (kind, label) = match &result.query {
        DependencyGraphQuery::ShortestPath { from, to } => {
            ("shortest-path", format!("Shortest path: {from} → {to}"))
        }
        DependencyGraphQuery::Closure { module, direction } => (
            "closure",
            format!("{} closure: {module}", direction.as_str()),
        ),
    };
    HtmlQuery {
        kind,
        found: result.found,
        label,
        ordered_nodes: result
            .nodes
            .iter()
            .filter_map(|node| ids.get(node).cloned())
            .collect(),
    }
}

fn html_hierarchy(view: &DependencyGraphView) -> Vec<HtmlHierarchyGroup> {
    let members = view
        .nodes
        .iter()
        .filter_map(|node| match &node.node {
            DependencyNode::LocalModule(module) => {
                let mut segments = if module.id.language().as_str() == "python" {
                    module
                        .id
                        .qualified_name()
                        .split('.')
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                } else {
                    module
                        .path
                        .parent()
                        .into_iter()
                        .flat_map(|path| path.components())
                        .map(|part| part.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                };
                if module.id.language().as_str() == "python"
                    && module.path.file_name().and_then(|name| name.to_str()) != Some("__init__.py")
                {
                    segments.pop();
                }
                Some(DependencyHierarchyMember {
                    node_id: node.id.clone(),
                    group_segments: segments,
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    build_dependency_hierarchy(&members)
        .into_iter()
        .map(|group| HtmlHierarchyGroup {
            id: group.id,
            name: group.name,
            qualified_name: group.qualified_name,
            parent: group.parent,
            direct_members: group.direct_modules,
            members: group.descendants,
        })
        .collect()
}

fn html_node(node: &DependencyGraphViewNode) -> HtmlNode {
    let (name, kind, subtitle, path, version, unresolved_reason, candidates) = match &node.node {
        DependencyNode::LocalModule(module) => {
            let path = crate::report::json_path(&module.path);
            (
                module.id.qualified_name().to_owned(),
                if module.id.language().as_str() == "cpp" {
                    "local-file"
                } else {
                    "local-module"
                },
                path.clone(),
                Some(path),
                None,
                None,
                Vec::new(),
            )
        }
        DependencyNode::StandardLibrary(module) => (
            module.qualified_name().to_owned(),
            "standard-library",
            "Python standard library".to_owned(),
            None,
            None,
            None,
            Vec::new(),
        ),
        DependencyNode::InstalledDistribution {
            distribution_display_name,
            version,
            ..
        } => (
            distribution_display_name.clone(),
            "installed-distribution",
            version
                .as_ref()
                .map(|value| format!("Installed package · {value}"))
                .unwrap_or_else(|| "Installed package".to_owned()),
            None,
            version.clone(),
            None,
            Vec::new(),
        ),
        DependencyNode::SystemHeader { name, .. } => (
            name.clone(),
            "system-header",
            "System header".to_owned(),
            None,
            None,
            None,
            Vec::new(),
        ),
        DependencyNode::ExternalHeader { name, .. } => (
            name.clone(),
            "external-header",
            "External header".to_owned(),
            None,
            None,
            None,
            Vec::new(),
        ),
        DependencyNode::Ambiguous {
            requested,
            candidates,
            ..
        } => (
            requested.clone(),
            "ambiguous",
            "Multiple possible targets".to_owned(),
            None,
            None,
            None,
            candidates.iter().map(html_candidate).collect(),
        ),
        DependencyNode::Unresolved {
            requested, reason, ..
        } => (
            requested.clone(),
            "unresolved",
            reason.as_str().replace('-', " "),
            None,
            None,
            Some(reason.as_str()),
            Vec::new(),
        ),
        DependencyNode::ContextDependent {
            requested,
            candidates,
            unresolved_reasons,
            ..
        } => (
            requested.clone(),
            "context-dependent",
            "Different build contexts select different targets".to_owned(),
            None,
            None,
            unresolved_reasons.first().map(|reason| reason.as_str()),
            candidates.iter().map(html_candidate).collect(),
        ),
    };
    let short_name = if kind == "local-file" {
        path.as_deref()
            .and_then(|path| Path::new(path).file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| short_name(&name))
    } else {
        short_name(&name)
    };
    HtmlNode {
        id: node.id.clone(),
        short_name,
        name,
        kind,
        subtitle,
        path,
        version,
        outgoing_dependencies_analyzed: match &node.node {
            DependencyNode::LocalModule(module) => Some(module.outgoing_dependencies_analyzed),
            _ => None,
        },
        unresolved_reason,
        candidates,
        cyclic_component: node.cyclic_component,
        source: None,
    }
}

fn html_candidate(candidate: &DependencyTarget) -> HtmlCandidate {
    match candidate {
        DependencyTarget::LocalModule(module) => HtmlCandidate {
            kind: if module.id.language().as_str() == "cpp" {
                "local-file"
            } else {
                candidate.kind().as_str()
            },
            name: module.id.qualified_name().to_owned(),
            detail: Some(crate::report::json_path(&module.path)),
        },
        DependencyTarget::StandardLibrary(module) => HtmlCandidate {
            kind: candidate.kind().as_str(),
            name: module.qualified_name().to_owned(),
            detail: Some("Python standard library".to_owned()),
        },
        DependencyTarget::InstalledDistribution {
            distribution_display_name,
            version,
            ..
        } => HtmlCandidate {
            kind: candidate.kind().as_str(),
            name: distribution_display_name.clone(),
            detail: version.clone(),
        },
        DependencyTarget::SystemHeader { name, .. } => HtmlCandidate {
            kind: candidate.kind().as_str(),
            name: name.clone(),
            detail: Some("System header".to_owned()),
        },
        DependencyTarget::ExternalHeader { name, .. } => HtmlCandidate {
            kind: candidate.kind().as_str(),
            name: name.clone(),
            detail: Some("External header".to_owned()),
        },
    }
}

fn html_relation(
    relation: &DependencyRelation,
    ids: &BTreeMap<DependencyNode, String>,
) -> HtmlRelation {
    HtmlRelation {
        source: ids[&relation.source].clone(),
        target: ids[&relation.target].clone(),
        kind: relation.kind.as_str(),
        inference_basis: (relation.kind == crate::DependencyRelationKind::Inferred)
            .then_some("unique repository suffix"),
        evidence: relation
            .evidence
            .iter()
            .map(|evidence| match &evidence.reference {
                crate::DependencyReference::Import(reference) => HtmlEvidence {
                    source_path: crate::report::json_path(&evidence.source_path),
                    import_name: reference
                        .module
                        .as_deref()
                        .or(reference.imported_name.as_deref())
                        .unwrap_or("import")
                        .to_owned(),
                    line: reference.span.start.line,
                    column: reference.span.start.column,
                    scope: reference.context.scope.as_str(),
                    usage: reference.context.usage.as_str(),
                    requirement: reference.context.requirement.as_str(),
                    conditional: reference.context.conditional,
                },
                crate::DependencyReference::Include(reference) => HtmlEvidence {
                    source_path: crate::report::json_path(&evidence.source_path),
                    import_name: reference.target.clone(),
                    line: reference.span.start.line,
                    column: reference.span.start.column,
                    scope: "file",
                    usage: "include",
                    requirement: "required",
                    conditional: reference.conditional,
                },
            })
            .collect(),
    }
}

fn short_name(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DependencyGraphFilter, LanguageId, LocalModule, ModuleId, analyze_dependency_graph,
        filter_dependency_graph,
    };

    fn local(name: &str) -> LocalModule {
        LocalModule::new(
            ModuleId::new(LanguageId::new("python"), name),
            format!("src/{}.py", name.replace('.', "/")),
        )
    }

    #[test]
    fn html_is_standalone_and_uses_short_visual_labels() {
        let module = local("shop.deeply_nested.service");
        let analysis = analyze_dependency_graph(&[module], &[]).expect("graph");
        let view = filter_dependency_graph(&analysis, &DependencyGraphFilter::default())
            .expect("filtered graph");

        let html = render_dependency_html(&view).expect("HTML");

        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Codegraide Dependency Explorer"));
        assert!(html.contains("\"short_name\":\"service\""));
        assert!(html.contains("\"name\":\"shop.deeply_nested.service\""));
        assert!(html.contains("\"qualified_name\":\"shop.deeply_nested\""));
        assert!(!html.contains(GRAPH_DATA_MARKER));
        assert!(!html.contains("https://"));
        assert!(!html.contains("fan_in"));
        assert!(!html.contains("fan_out"));
    }

    #[test]
    fn embedded_json_escapes_script_terminators() {
        let module = LocalModule::new(
            ModuleId::new(LanguageId::new("python"), "bad</script>name"),
            "bad.py",
        );
        let analysis = analyze_dependency_graph(&[module], &[]).expect("graph");
        let view = filter_dependency_graph(&analysis, &DependencyGraphFilter::default())
            .expect("filtered graph");

        let html = render_dependency_html(&view).expect("HTML");

        assert_eq!(html.matches("</script>").count(), 2);
        assert!(!html.contains("bad</script>name"));
        assert!(html.contains("bad\\u003c/script\\u003ename"));
    }

    #[test]
    fn source_excerpt_uses_one_based_inclusive_line_spans() {
        let source = "first\nsecond\nthird\nfourth\n";
        let excerpt = source_for_span(
            source,
            crate::SourceSpan {
                start_byte: 6,
                end_byte: 18,
                start: crate::SourcePosition { line: 2, column: 1 },
                end: crate::SourcePosition { line: 3, column: 6 },
            },
        )
        .expect("source excerpt");

        assert_eq!(excerpt.start_line, 2);
        assert_eq!(excerpt.end_line, 3);
        assert_eq!(excerpt.lines, ["second", "third"]);
    }
}
