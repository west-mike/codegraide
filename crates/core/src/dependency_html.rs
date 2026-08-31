//! Self-contained interactive dependency graph rendering.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::calls::{CallGraphView, CallNode, CallRelation, ProjectSymbol};
use crate::dependencies::DependencyTarget;
use crate::dependency_cycles::explain_dependency_cycle;
use crate::dependency_hierarchy::{DependencyHierarchyMember, build_dependency_hierarchy};
use crate::dependency_output::{DependencyGraphView, DependencyGraphViewNode};
use crate::dependency_query::{DependencyGraphQuery, DependencyGraphQueryResult};
use crate::graph::{DependencyNode, DependencyRelation};

const VIEWER_TEMPLATE: &str = include_str!("dependency_viewer.html");
const GRAPH_DATA_MARKER: &str = "__CODEGRAIDE_GRAPH_DATA__";

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
    unresolved_reason: Option<&'static str>,
    candidates: Vec<HtmlCandidate>,
    cyclic_component: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<HtmlSource>,
}

#[derive(Debug, Clone, Serialize)]
struct HtmlSource {
    start_line: usize,
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
    render_call_html_graph(view, &BTreeMap::new())
}

/// Render a call graph with repository source embedded for sidebar inspection.
///
/// Source remains an HTML presentation concern: the stable graph and JSON report
/// continue to carry spans and paths without copying repository contents.
pub fn render_call_html_with_source(
    view: &CallGraphView,
    project_root: &Path,
) -> Result<String, CallHtmlSourceError> {
    let sources = load_call_sources(view, project_root)?;
    render_call_html_graph(view, &sources).map_err(CallHtmlSourceError::Serialize)
}

fn render_call_html_graph(
    view: &CallGraphView,
    sources: &BTreeMap<ProjectSymbol, HtmlSource>,
) -> Result<String, serde_json::Error> {
    let ids = view
        .nodes
        .iter()
        .map(|node| (node.node.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let hierarchy_members = view
        .nodes
        .iter()
        .filter_map(|node| match &node.node {
            CallNode::LocalSymbol(symbol) => {
                let mut segments = symbol
                    .id
                    .module
                    .qualified_name()
                    .split('.')
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if matches!(
                    symbol.id.kind,
                    crate::SymbolKind::Class | crate::SymbolKind::Method
                ) {
                    let mut owners = symbol.id.qualified_name.split('.').collect::<Vec<_>>();
                    if symbol.id.kind == crate::SymbolKind::Method {
                        owners.pop();
                    }
                    segments.extend(owners.into_iter().map(str::to_owned));
                }
                Some(DependencyHierarchyMember {
                    node_id: node.id.clone(),
                    group_segments: segments,
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let graph = HtmlGraph {
        graph_kind: "calls",
        nodes: view
            .nodes
            .iter()
            .map(|node| {
                let source = match &node.node {
                    CallNode::LocalSymbol(symbol) => sources.get(symbol).cloned(),
                    _ => None,
                };
                html_call_node(&node.id, &node.node, node.cyclic_component, source)
            })
            .collect(),
        relations: view
            .relations
            .iter()
            .map(|relation| html_call_relation(relation, &ids))
            .collect(),
        cycles: view
            .strongly_connected_components
            .iter()
            .filter(|component| component.cyclic)
            .enumerate()
            .map(|(index, component)| HtmlCycle {
                number: index + 1,
                members: component
                    .members
                    .iter()
                    .filter_map(|member| ids.get(member).cloned())
                    .collect(),
                witness_nodes: Vec::new(),
                witness_relations: Vec::new(),
                recommended_cuts: Vec::new(),
            })
            .collect(),
        hierarchy: build_dependency_hierarchy(&hierarchy_members)
            .into_iter()
            .map(|group| HtmlHierarchyGroup {
                id: group.id,
                name: group.name,
                qualified_name: group.qualified_name,
                parent: group.parent,
                direct_members: group.direct_modules,
                members: group.descendants,
            })
            .collect(),
        query: None,
    };
    render_html_graph(&graph)
}

fn load_call_sources(
    view: &CallGraphView,
    project_root: &Path,
) -> Result<BTreeMap<ProjectSymbol, HtmlSource>, CallHtmlSourceError> {
    let mut files = BTreeMap::<PathBuf, String>::new();
    let mut sources = BTreeMap::new();
    for symbol in view.nodes.iter().filter_map(|node| match &node.node {
        CallNode::LocalSymbol(symbol)
            if matches!(
                symbol.id.kind,
                crate::SymbolKind::Function | crate::SymbolKind::Method | crate::SymbolKind::Lambda
            ) =>
        {
            Some(symbol)
        }
        _ => None,
    }) {
        if !files.contains_key(&symbol.path) {
            let absolute_path = project_root.join(&symbol.path);
            let contents =
                fs::read_to_string(&absolute_path).map_err(|source| CallHtmlSourceError::Read {
                    path: absolute_path,
                    source,
                })?;
            files.insert(symbol.path.clone(), contents);
        }
        if let Some(source) = files
            .get(&symbol.path)
            .and_then(|contents| source_for_span(contents, symbol.span))
        {
            sources.insert(symbol.clone(), source);
        }
    }
    Ok(sources)
}

fn source_for_span(source: &str, span: crate::SourceSpan) -> Option<HtmlSource> {
    let lines = source.lines().collect::<Vec<_>>();
    let start = span.start.line.checked_sub(1)?;
    let end = span.end.line.min(lines.len());
    if start >= end {
        return None;
    }
    Some(HtmlSource {
        start_line: span.start.line,
        end_line: end,
        lines: lines[start..end]
            .iter()
            .map(|line| (*line).to_owned())
            .collect(),
    })
}

fn render_html_graph(graph: &HtmlGraph) -> Result<String, serde_json::Error> {
    let data = serde_json::to_string(graph)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Ok(VIEWER_TEMPLATE.replace(GRAPH_DATA_MARKER, &data))
}

fn html_call_node(
    id: &str,
    node: &CallNode,
    cyclic_component: Option<usize>,
    source: Option<HtmlSource>,
) -> HtmlNode {
    let (name, kind, subtitle, path, candidates) = match node {
        CallNode::LocalSymbol(symbol) => {
            let path = crate::report::json_path(&symbol.path);
            (
                symbol.id.base_selector(),
                "local-module",
                format!("{} · {path}", symbol.id.kind.as_str()),
                Some(path),
                Vec::new(),
            )
        }
        CallNode::External { name, .. } => (
            name.clone(),
            "installed-distribution",
            "External call boundary".to_owned(),
            None,
            Vec::new(),
        ),
        CallNode::Ambiguous {
            spelling,
            candidates,
            ..
        } => (
            spelling.clone(),
            "ambiguous",
            "Multiple possible call targets".to_owned(),
            None,
            candidates
                .iter()
                .map(|candidate| HtmlCandidate {
                    kind: "local-symbol",
                    name: candidate.id.ordinal_selector(),
                    detail: Some(crate::report::json_path(&candidate.path)),
                })
                .collect(),
        ),
        CallNode::Unresolved { spelling, .. } => (
            spelling.clone(),
            "unresolved",
            "Conservative static resolution stopped here".to_owned(),
            None,
            Vec::new(),
        ),
    };
    HtmlNode {
        id: id.to_owned(),
        short_name: name
            .rsplit(['.', ':'])
            .find(|part| !part.is_empty())
            .unwrap_or(&name)
            .to_owned(),
        name,
        kind,
        subtitle,
        path,
        version: None,
        unresolved_reason: matches!(node, CallNode::Unresolved { .. }).then_some("unresolved-call"),
        candidates,
        cyclic_component,
        source,
    }
}

fn html_call_relation(relation: &CallRelation, ids: &BTreeMap<CallNode, String>) -> HtmlRelation {
    HtmlRelation {
        source: ids[&relation.source].clone(),
        target: ids[&relation.target].clone(),
        kind: relation.kind.as_str(),
        evidence: relation
            .evidence
            .iter()
            .map(|evidence| HtmlEvidence {
                source_path: crate::report::json_path(&evidence.source_path),
                import_name: evidence.reference.callee.clone(),
                line: evidence.reference.span.start.line,
                column: evidence.reference.span.start.column,
                scope: "call-site",
                usage: "runtime",
                requirement: "required",
                conditional: false,
            })
            .collect(),
    }
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
                let mut segments = module
                    .id
                    .qualified_name()
                    .split('.')
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if module.path.file_name().and_then(|name| name.to_str()) != Some("__init__.py") {
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
                "local-module",
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
    };
    HtmlNode {
        id: node.id.clone(),
        short_name: short_name(&name),
        name,
        kind,
        subtitle,
        path,
        version,
        unresolved_reason,
        candidates,
        cyclic_component: node.cyclic_component,
        source: None,
    }
}

fn html_candidate(candidate: &DependencyTarget) -> HtmlCandidate {
    match candidate {
        DependencyTarget::LocalModule(module) => HtmlCandidate {
            kind: candidate.kind().as_str(),
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
        evidence: relation
            .evidence
            .iter()
            .map(|evidence| {
                let reference = evidence
                    .reference
                    .as_import()
                    .expect("Python dependency graphs contain import evidence");
                HtmlEvidence {
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
                }
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
