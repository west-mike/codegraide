use std::collections::BTreeMap;

use serde::Serialize;

use crate::calls::{
    CALL_FAN_IN_DEFINITION_VERSION, CALL_FAN_OUT_DEFINITION_VERSION, CALL_GRAPH_DEFINITION_VERSION,
    CALL_REPORT_SCHEMA_VERSION, CALL_SCC_DEFINITION_VERSION, CallGraphAnalysis, CallGraphView,
    CallNode, CallRelation, ProjectSymbol, call_node_name,
};

#[derive(Debug, Serialize)]
pub struct CallJsonReport {
    pub report_schema_version: &'static str,
    pub definitions: CallDefinitionVersions,
    pub coverage: JsonCallCoverage,
    pub view: JsonCallView,
    pub nodes: Vec<JsonCallNode>,
    pub relations: Vec<JsonCallRelation>,
    pub strongly_connected_components: Vec<JsonCallScc>,
}

#[derive(Debug, Serialize)]
pub struct CallDefinitionVersions {
    pub graph: &'static str,
    pub fan_in: &'static str,
    pub fan_out: &'static str,
    pub strongly_connected_components: &'static str,
}

#[derive(Debug, Serialize)]
pub struct JsonCallCoverage {
    pub total_calls: usize,
    pub exact_calls: usize,
    pub external_calls: usize,
    pub ambiguous_calls: usize,
    pub unresolved_calls: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonCallView {
    pub focus_symbols: Vec<String>,
    pub direction: &'static str,
    pub depth: usize,
    pub exact_only: bool,
    pub local_only: bool,
    pub cycles_only: bool,
    pub node_count: usize,
    pub relation_count: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonCallNode {
    pub id: String,
    pub kind: &'static str,
    pub selector: String,
    pub symbol_kind: Option<&'static str>,
    pub path: Option<String>,
    pub duplicate_ordinal: Option<usize>,
    pub candidates: Vec<String>,
    pub fan_in: usize,
    pub fan_out: usize,
    pub cyclic_component: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct JsonCallRelation {
    pub source: String,
    pub target: String,
    pub kind: &'static str,
    pub evidence: Vec<JsonCallEvidence>,
}

#[derive(Debug, Serialize)]
pub struct JsonCallEvidence {
    pub source_path: String,
    pub callee: String,
    pub components: Vec<String>,
    pub line: usize,
    pub column: usize,
    pub positional_arguments: usize,
    pub keyword_arguments: Vec<String>,
    pub syntax_complete: bool,
}

#[derive(Debug, Serialize)]
pub struct JsonCallScc {
    pub cyclic: bool,
    pub members: Vec<String>,
}

impl CallJsonReport {
    pub fn from_analysis(analysis: &CallGraphAnalysis, view: &CallGraphView) -> Self {
        let ids = view
            .nodes
            .iter()
            .map(|node| (node.node.clone(), node.id.clone()))
            .collect::<BTreeMap<_, _>>();
        Self {
            report_schema_version: CALL_REPORT_SCHEMA_VERSION,
            definitions: CallDefinitionVersions {
                graph: CALL_GRAPH_DEFINITION_VERSION,
                fan_in: CALL_FAN_IN_DEFINITION_VERSION,
                fan_out: CALL_FAN_OUT_DEFINITION_VERSION,
                strongly_connected_components: CALL_SCC_DEFINITION_VERSION,
            },
            coverage: JsonCallCoverage {
                total_calls: analysis.coverage.total_calls,
                exact_calls: analysis.coverage.exact_calls,
                external_calls: analysis.coverage.external_calls,
                ambiguous_calls: analysis.coverage.ambiguous_calls,
                unresolved_calls: analysis.coverage.unresolved_calls,
            },
            view: JsonCallView {
                focus_symbols: view.filter.focus_symbols.clone(),
                direction: match view.filter.direction {
                    crate::CallDirection::Callers => "callers",
                    crate::CallDirection::Callees => "callees",
                    crate::CallDirection::Both => "both",
                },
                depth: view.filter.depth,
                exact_only: view.filter.exact_only,
                local_only: view.filter.local_only,
                cycles_only: view.filter.cycles_only,
                node_count: view.nodes.len(),
                relation_count: view.relations.len(),
            },
            nodes: view
                .nodes
                .iter()
                .map(|node| {
                    let (symbol_kind, path, ordinal, candidates) = match &node.node {
                        CallNode::LocalSymbol(symbol) => (
                            Some(symbol.id.kind.as_str()),
                            Some(crate::report::json_path(&symbol.path)),
                            Some(symbol.id.duplicate_ordinal),
                            Vec::new(),
                        ),
                        CallNode::Ambiguous { candidates, .. } => {
                            (None, None, None, candidates.iter().map(selector).collect())
                        }
                        _ => (None, None, None, Vec::new()),
                    };
                    JsonCallNode {
                        id: node.id.clone(),
                        kind: node.node.kind(),
                        selector: call_node_name(&node.node),
                        symbol_kind,
                        path,
                        duplicate_ordinal: ordinal,
                        candidates,
                        fan_in: node.fan_in,
                        fan_out: node.fan_out,
                        cyclic_component: node.cyclic_component,
                    }
                })
                .collect(),
            relations: view
                .relations
                .iter()
                .map(|relation| json_relation(relation, &ids))
                .collect(),
            strongly_connected_components: view
                .strongly_connected_components
                .iter()
                .map(|component| JsonCallScc {
                    cyclic: component.cyclic,
                    members: component
                        .members
                        .iter()
                        .filter_map(|member| ids.get(member).cloned())
                        .collect(),
                })
                .collect(),
        }
    }
}

fn selector(symbol: &ProjectSymbol) -> String {
    symbol.id.ordinal_selector()
}

fn json_relation(relation: &CallRelation, ids: &BTreeMap<CallNode, String>) -> JsonCallRelation {
    JsonCallRelation {
        source: ids[&relation.source].clone(),
        target: ids[&relation.target].clone(),
        kind: relation.kind.as_str(),
        evidence: relation
            .evidence
            .iter()
            .map(|evidence| JsonCallEvidence {
                source_path: crate::report::json_path(&evidence.source_path),
                callee: evidence.reference.callee.clone(),
                components: evidence.reference.components.clone(),
                line: evidence.reference.span.start.line,
                column: evidence.reference.span.start.column,
                positional_arguments: evidence.reference.arguments.positional,
                keyword_arguments: evidence.reference.arguments.keywords.clone(),
                syntax_complete: evidence.reference.syntax_complete,
            })
            .collect(),
    }
}
