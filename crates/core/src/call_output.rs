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
    pub language: String,
    pub definitions: CallDefinitionVersions,
    pub coverage: JsonCallCoverage,
    pub cpp_modules: Vec<JsonCppModuleSummary>,
    pub architecture_groups: Vec<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_index: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declaration_definition_linking: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_references: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_resolution: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct JsonCppModuleSummary {
    pub path: String,
    pub name: String,
    pub partition: Option<String>,
    pub kind: &'static str,
    pub exported: bool,
    pub complete: bool,
    pub exported_symbols: Vec<String>,
    pub imports: Vec<JsonCppModuleImport>,
    pub exports: Vec<JsonCppModuleExport>,
}

#[derive(Debug, Serialize)]
pub struct JsonCppModuleImport {
    pub target: String,
    pub kind: &'static str,
    pub exported: bool,
    pub conditional: bool,
    pub complete: bool,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonCppModuleExport {
    pub target: String,
    pub complete: bool,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonCallCoverage {
    pub total_calls: usize,
    pub exact_calls: usize,
    pub inferred_calls: usize,
    pub external_calls: usize,
    pub ambiguous_calls: usize,
    pub unresolved_calls: usize,
    pub unavailable_calls: usize,
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
    pub candidates: Vec<JsonCallCandidate>,
    pub signature: Option<String>,
    pub link_status: Option<&'static str>,
    pub declarations: Vec<JsonSymbolLocation>,
    pub definition: Option<JsonSymbolLocation>,
    pub language_module: Option<String>,
    pub architecture_groups: Vec<String>,
    pub primary_architecture_group: Option<String>,
    pub fan_in: usize,
    pub fan_out: usize,
    pub cyclic_component: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct JsonCallRelation {
    pub source: String,
    pub target: String,
    pub kind: &'static str,
    pub alternatives: Vec<JsonCallCandidate>,
    pub reason: Option<String>,
    pub evidence: Vec<JsonCallEvidence>,
}

#[derive(Debug, Serialize)]
pub struct JsonCallCandidate {
    pub selector: String,
    pub signature: Option<String>,
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonSymbolLocation {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonCallEvidence {
    pub source_path: String,
    pub expression: String,
    pub callee: String,
    pub components: Vec<String>,
    pub line: usize,
    pub column: usize,
    pub positional_arguments: usize,
    pub keyword_arguments: Vec<String>,
    pub form: &'static str,
    pub receiver: Option<String>,
    pub receiver_type_hint: Option<String>,
    pub argument_expressions: Vec<String>,
    pub argument_type_hints: Vec<Option<String>>,
    pub syntax_complete: bool,
    pub preprocessing_uncertain: bool,
}

#[derive(Debug, Serialize)]
pub struct JsonCallScc {
    pub cyclic: bool,
    pub members: Vec<String>,
}

impl CallJsonReport {
    pub fn from_analysis(analysis: &CallGraphAnalysis, view: &CallGraphView) -> Self {
        let language = view
            .nodes
            .iter()
            .find_map(|node| match &node.node {
                CallNode::LocalSymbol(symbol) => Some(symbol.id.language.as_str()),
                _ => None,
            })
            .unwrap_or("unknown");
        Self::from_analysis_for_language(language, analysis, view)
    }

    pub fn from_analysis_for_language(
        language: &str,
        analysis: &CallGraphAnalysis,
        view: &CallGraphView,
    ) -> Self {
        let ids = view
            .nodes
            .iter()
            .map(|node| (node.node.clone(), node.id.clone()))
            .collect::<BTreeMap<_, _>>();
        Self {
            report_schema_version: CALL_REPORT_SCHEMA_VERSION,
            language: language.to_owned(),
            definitions: CallDefinitionVersions {
                graph: CALL_GRAPH_DEFINITION_VERSION,
                fan_in: CALL_FAN_IN_DEFINITION_VERSION,
                fan_out: CALL_FAN_OUT_DEFINITION_VERSION,
                strongly_connected_components: CALL_SCC_DEFINITION_VERSION,
                symbol_index: (language == "cpp").then_some("cpp-symbol-index-v2"),
                declaration_definition_linking: (language == "cpp")
                    .then_some("cpp-declaration-definition-linking-v2"),
                call_references: (language == "cpp").then_some("cpp-call-references-v1"),
                call_resolution: (language == "cpp").then_some("cpp-call-resolution-v1"),
                modules: (language == "cpp").then_some("cpp-modules-v1"),
            },
            coverage: JsonCallCoverage {
                total_calls: analysis.coverage.total_calls,
                exact_calls: analysis.coverage.exact_calls,
                inferred_calls: analysis.coverage.inferred_calls,
                external_calls: analysis.coverage.external_calls,
                ambiguous_calls: analysis.coverage.ambiguous_calls,
                unresolved_calls: analysis.coverage.unresolved_calls,
                unavailable_calls: analysis.coverage.unavailable_calls,
            },
            cpp_modules: cpp_modules(view),
            architecture_groups: architecture_groups(view),
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
                    let (symbol_kind, path, ordinal, candidates, symbol) = match &node.node {
                        CallNode::LocalSymbol(symbol) => (
                            Some(symbol.id.kind.as_str()),
                            Some(crate::report::json_path(&symbol.path)),
                            Some(symbol.id.duplicate_ordinal),
                            Vec::new(),
                            Some(symbol),
                        ),
                        CallNode::Ambiguous { candidates, .. } => (
                            None,
                            None,
                            None,
                            candidates.iter().map(json_candidate).collect(),
                            None,
                        ),
                        _ => (None, None, None, Vec::new(), None),
                    };
                    JsonCallNode {
                        id: node.id.clone(),
                        kind: node.node.kind(),
                        selector: call_node_name(&node.node),
                        symbol_kind,
                        path,
                        duplicate_ordinal: ordinal,
                        candidates,
                        signature: symbol
                            .and_then(|symbol| symbol.signature.as_ref())
                            .map(|signature| signature.display.clone()),
                        link_status: symbol.map(|symbol| symbol.link_status.as_str()),
                        declarations: symbol
                            .into_iter()
                            .flat_map(|symbol| &symbol.declarations)
                            .map(json_location)
                            .collect(),
                        definition: symbol
                            .and_then(|symbol| symbol.definition.as_ref())
                            .map(json_location),
                        language_module: symbol.and_then(|symbol| symbol.language_module.clone()),
                        architecture_groups: symbol
                            .map(|symbol| symbol.architecture_groups.clone())
                            .unwrap_or_default(),
                        primary_architecture_group: symbol
                            .and_then(|symbol| symbol.primary_architecture_group.clone()),
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

fn json_candidate(symbol: &ProjectSymbol) -> JsonCallCandidate {
    JsonCallCandidate {
        selector: symbol.id.ordinal_selector(),
        signature: symbol
            .signature
            .as_ref()
            .map(|signature| signature.display.clone()),
        path: crate::report::json_path(&symbol.path),
        line: symbol.span.start.line,
    }
}

fn json_relation(relation: &CallRelation, ids: &BTreeMap<CallNode, String>) -> JsonCallRelation {
    JsonCallRelation {
        source: ids[&relation.source].clone(),
        target: ids[&relation.target].clone(),
        kind: relation.kind.as_str(),
        alternatives: relation.alternatives.iter().map(json_candidate).collect(),
        reason: relation.reason.clone(),
        evidence: relation
            .evidence
            .iter()
            .map(|evidence| JsonCallEvidence {
                source_path: crate::report::json_path(&evidence.source_path),
                expression: evidence.reference.expression.clone(),
                callee: evidence.reference.callee.clone(),
                components: evidence.reference.components.clone(),
                line: evidence.reference.span.start.line,
                column: evidence.reference.span.start.column,
                positional_arguments: evidence.reference.arguments.positional,
                keyword_arguments: evidence.reference.arguments.keywords.clone(),
                form: evidence.reference.form.as_str(),
                receiver: evidence.reference.receiver.clone(),
                receiver_type_hint: evidence.reference.receiver_type_hint.clone(),
                argument_expressions: evidence
                    .reference
                    .argument_details
                    .iter()
                    .map(|argument| argument.expression.clone())
                    .collect(),
                argument_type_hints: evidence
                    .reference
                    .argument_details
                    .iter()
                    .map(|argument| argument.type_hint.clone())
                    .collect(),
                syntax_complete: evidence.reference.syntax_complete,
                preprocessing_uncertain: evidence.reference.preprocessing_uncertain,
            })
            .collect(),
    }
}

fn cpp_modules(view: &CallGraphView) -> Vec<JsonCppModuleSummary> {
    let mut exported_by_module = BTreeMap::<String, Vec<String>>::new();
    for symbol in view.nodes.iter().filter_map(|node| match &node.node {
        CallNode::LocalSymbol(symbol) => Some(symbol),
        _ => None,
    }) {
        if let Some(module) = &symbol.language_module {
            exported_by_module
                .entry(module.clone())
                .or_default()
                .push(symbol.id.ordinal_selector());
        }
    }
    let mut modules = view
        .language_modules
        .iter()
        .map(|project_module| {
            let mut exported_symbols = exported_by_module
                .get(&project_module.module.name)
                .cloned()
                .unwrap_or_default();
            exported_symbols.sort();
            exported_symbols.dedup();
            JsonCppModuleSummary {
                path: crate::report::json_path(&project_module.path),
                name: project_module.module.name.clone(),
                partition: project_module.module.partition.clone(),
                kind: project_module.module.kind.as_str(),
                exported: project_module.module.exported,
                complete: project_module.module.complete,
                exported_symbols,
                imports: project_module
                    .imports
                    .iter()
                    .map(|import| JsonCppModuleImport {
                        target: import.target.clone(),
                        kind: import.kind.as_str(),
                        exported: import.exported,
                        conditional: import.conditional,
                        complete: import.complete,
                        line: import.span.start.line,
                        column: import.span.start.column,
                    })
                    .collect(),
                exports: project_module
                    .exports
                    .iter()
                    .map(|export| JsonCppModuleExport {
                        target: export.target.clone(),
                        complete: export.complete,
                        line: export.span.start.line,
                        column: export.span.start.column,
                    })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.partition.cmp(&right.partition))
            .then_with(|| left.path.cmp(&right.path))
    });
    modules
}

fn architecture_groups(view: &CallGraphView) -> Vec<String> {
    let mut groups = view
        .nodes
        .iter()
        .filter_map(|node| match &node.node {
            CallNode::LocalSymbol(symbol) => Some(&symbol.architecture_groups),
            _ => None,
        })
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    groups.sort();
    groups.dedup();
    groups
}

fn json_location(location: &crate::ProjectSymbolLocation) -> JsonSymbolLocation {
    JsonSymbolLocation {
        path: crate::report::json_path(&location.path),
        line: location.span.start.line,
        column: location.span.start.column,
    }
}
