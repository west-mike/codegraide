use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde::Serialize;

use crate::dependencies::{
    DEPENDENCY_CYCLE_DEFINITION_VERSION, DEPENDENCY_FAN_IN_DEFINITION_VERSION,
    DEPENDENCY_FAN_OUT_DEFINITION_VERSION, DEPENDENCY_GRAPH_DEFINITION_VERSION,
    DEPENDENCY_SCC_DEFINITION_VERSION, DependencyGraphInputExclusions, DependencyTarget,
};
use crate::dependency_cycles::{
    DEPENDENCY_CYCLE_EXPLANATION_DEFINITION_VERSION, explain_dependency_cycles,
};
use crate::dependency_query::{
    DependencyGraphQuery, DependencyGraphQueryResult, DependencyQueryDirection,
};
use crate::graph::{
    DependencyGraphAnalysis, DependencyGraphCoverage, DependencyNode, DependencyNodeKind,
    DependencyRelation, DependencyRelationKind, DependencyScc,
};

pub const DEPENDENCY_REPORT_SCHEMA_VERSION: &str = "0.5.0";

#[derive(Debug, Serialize)]
pub struct DependencyBundleJsonReport {
    pub report_schema_version: &'static str,
    pub tool: DependencyJsonTool,
    pub languages: Vec<DependencyLanguageJsonReport>,
    pub unavailable_languages: Vec<UnavailableDependencyLanguage>,
}

#[derive(Debug, Serialize)]
pub struct DependencyLanguageJsonReport {
    pub language: String,
    pub resolver: DependencyResolverReport,
    pub graph: DependencyJsonReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyResolverReport {
    pub id: String,
    pub version: String,
    pub definition_version: String,
    pub unit_kind: &'static str,
    pub hierarchy_behavior: String,
    pub resolution_capabilities: Vec<String>,
    pub status: &'static str,
    pub context: Vec<DependencyResolverContextReport>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyResolverContextReport {
    pub kind: String,
    pub selected: bool,
    pub total: usize,
    pub supported: usize,
    pub unsupported: usize,
}

#[derive(Debug, Serialize)]
pub struct UnavailableDependencyLanguage {
    pub language: String,
    pub status: &'static str,
    pub installation_hint: Option<String>,
}

impl DependencyBundleJsonReport {
    pub fn new(
        mut languages: Vec<DependencyLanguageJsonReport>,
        mut unavailable_languages: Vec<UnavailableDependencyLanguage>,
    ) -> Self {
        languages.sort_by(|left, right| left.language.cmp(&right.language));
        unavailable_languages.sort_by(|left, right| left.language.cmp(&right.language));
        Self {
            report_schema_version: DEPENDENCY_REPORT_SCHEMA_VERSION,
            tool: DependencyJsonTool {
                name: "codegraide",
                version: env!("CARGO_PKG_VERSION"),
            },
            languages,
            unavailable_languages,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyDirection {
    Dependencies,
    Dependents,
    Both,
}

impl DependencyDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dependencies => "dependencies",
            Self::Dependents => "dependents",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyGraphFilter {
    pub focus_modules: Vec<String>,
    pub direction: DependencyDirection,
    pub depth: usize,
    pub exact_only: bool,
    pub local_only: bool,
    pub cycles_only: bool,
}

impl Default for DependencyGraphFilter {
    fn default() -> Self {
        Self {
            focus_modules: Vec::new(),
            direction: DependencyDirection::Both,
            depth: 1,
            exact_only: false,
            local_only: false,
            cycles_only: false,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyGraphViewNode {
    pub id: String,
    pub node: DependencyNode,
    pub fan_in: usize,
    pub fan_out: usize,
    pub cyclic_component: Option<usize>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyGraphView {
    pub filter: DependencyGraphFilter,
    pub nodes: Vec<DependencyGraphViewNode>,
    pub relations: Vec<DependencyRelation>,
    pub strongly_connected_components: Vec<DependencyScc>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DependencyGraphFilterError {
    pub module: String,
    pub suggestions: Vec<String>,
}

impl fmt::Display for DependencyGraphFilterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "focus unit {:?} is not a local dependency unit",
            self.module
        )?;
        if !self.suggestions.is_empty() {
            write!(formatter, "; did you mean {}?", self.suggestions.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for DependencyGraphFilterError {}

pub fn filter_dependency_graph(
    analysis: &DependencyGraphAnalysis,
    filter: &DependencyGraphFilter,
) -> Result<DependencyGraphView, DependencyGraphFilterError> {
    let full_ids = analysis
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.clone(), format!("n{index:04}")))
        .collect::<BTreeMap<_, _>>();
    let local_names = analysis
        .nodes
        .iter()
        .filter_map(|node| match node {
            DependencyNode::LocalModule(module) => {
                Some((module.id.qualified_name().to_owned(), node.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let allowed_relations = analysis
        .relations
        .iter()
        .filter(|relation| !filter.exact_only || relation.kind == DependencyRelationKind::Exact)
        .cloned()
        .collect::<Vec<_>>();

    let mut selected = if filter.cycles_only {
        analysis
            .cycles
            .iter()
            .flat_map(|component| component.members.iter().cloned())
            .collect::<BTreeSet<_>>()
    } else if filter.focus_modules.is_empty() {
        analysis.nodes.iter().cloned().collect()
    } else {
        let mut seeds = BTreeSet::new();
        for requested in &filter.focus_modules {
            let matches = local_names
                .iter()
                .filter(|(name, _)| name == requested)
                .map(|(_, node)| node.clone())
                .collect::<Vec<_>>();
            if matches.is_empty() {
                return Err(DependencyGraphFilterError {
                    module: requested.clone(),
                    suggestions: suggestions(requested, &local_names),
                });
            }
            seeds.extend(matches);
        }
        traverse(&seeds, &allowed_relations, filter.direction, filter.depth)
    };

    complete_selected_cycles(&mut selected, &analysis.cycles);
    if filter.exact_only {
        selected.retain(|node| {
            !matches!(
                node.kind(),
                DependencyNodeKind::Ambiguous
                    | DependencyNodeKind::Unresolved
                    | DependencyNodeKind::ContextDependent
            )
        });
    }
    if filter.local_only || filter.cycles_only {
        selected.retain(|node| node.kind() == DependencyNodeKind::LocalModule);
    }
    let relations = allowed_relations
        .into_iter()
        .filter(|relation| {
            selected.contains(&relation.source)
                && selected.contains(&relation.target)
                && (!filter.cycles_only || relation.kind == DependencyRelationKind::Exact)
        })
        .collect::<Vec<_>>();
    let metrics = analysis
        .metrics
        .iter()
        .map(|metric| (metric.node.clone(), metric))
        .collect::<BTreeMap<_, _>>();
    let cycle_numbers = analysis
        .cycles
        .iter()
        .enumerate()
        .flat_map(|(index, component)| {
            component
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
        .map(|node| {
            let metric = metrics[node];
            DependencyGraphViewNode {
                id: full_ids[node].clone(),
                node: node.clone(),
                fan_in: metric.fan_in,
                fan_out: metric.fan_out,
                cyclic_component: cycle_numbers.get(node).copied(),
            }
        })
        .collect::<Vec<_>>();
    let strongly_connected_components = analysis
        .strongly_connected_components
        .iter()
        .filter(|component| component.members.iter().all(|node| selected.contains(node)))
        .cloned()
        .collect();
    Ok(DependencyGraphView {
        filter: filter.clone(),
        nodes,
        relations,
        strongly_connected_components,
    })
}

fn traverse(
    seeds: &BTreeSet<DependencyNode>,
    relations: &[DependencyRelation],
    direction: DependencyDirection,
    depth: usize,
) -> BTreeSet<DependencyNode> {
    let mut selected = seeds.clone();
    let mut queue = seeds
        .iter()
        .cloned()
        .map(|node| (node, 0_usize))
        .collect::<VecDeque<_>>();
    while let Some((node, distance)) = queue.pop_front() {
        if distance >= depth {
            continue;
        }
        for relation in relations {
            let neighbor = if matches!(
                direction,
                DependencyDirection::Dependencies | DependencyDirection::Both
            ) && relation.source == node
            {
                Some(&relation.target)
            } else if matches!(
                direction,
                DependencyDirection::Dependents | DependencyDirection::Both
            ) && relation.target == node
            {
                Some(&relation.source)
            } else {
                None
            };
            if let Some(neighbor) = neighbor {
                if selected.insert(neighbor.clone()) {
                    queue.push_back((neighbor.clone(), distance + 1));
                }
            }
        }
    }
    selected
}

fn complete_selected_cycles(selected: &mut BTreeSet<DependencyNode>, cycles: &[DependencyScc]) {
    for component in cycles {
        if component.members.iter().any(|node| selected.contains(node)) {
            selected.extend(component.members.iter().cloned());
        }
    }
}

fn suggestions(requested: &str, modules: &[(String, DependencyNode)]) -> Vec<String> {
    let mut ranked = modules
        .iter()
        .map(|(name, _)| (edit_distance(requested, name), name))
        .collect::<Vec<_>>();
    ranked.sort();
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
            current.push(
                (previous[right_index + 1] + 1)
                    .min(current[right_index] + 1)
                    .min(previous[right_index] + usize::from(left_char != right_char)),
            );
        }
        previous = current;
    }
    previous[right.chars().count()]
}

pub fn render_dependency_mermaid(view: &DependencyGraphView) -> String {
    let ids = view
        .nodes
        .iter()
        .map(|node| (node.node.clone(), node.id.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut output = String::from("flowchart LR\n");
    let cyclic = view
        .nodes
        .iter()
        .filter(|node| node.cyclic_component.is_some())
        .map(|node| node.node.clone())
        .collect::<BTreeSet<_>>();
    for node in view
        .nodes
        .iter()
        .filter(|node| !cyclic.contains(&node.node))
    {
        output.push_str(&format!(
            "  {}[\"{}\"]\n",
            node.id,
            escape_mermaid(&node_label(node))
        ));
    }
    for component in view
        .strongly_connected_components
        .iter()
        .filter(|component| component.cyclic)
    {
        let number = component
            .members
            .iter()
            .find_map(|member| {
                view.nodes
                    .iter()
                    .find(|node| &node.node == member)
                    .and_then(|node| node.cyclic_component)
            })
            .unwrap_or(0);
        output.push_str(&format!("  subgraph cycle_{number}[\"Cycle {number}\"]\n"));
        for member in &component.members {
            if let Some(node) = view.nodes.iter().find(|node| &node.node == member) {
                output.push_str(&format!(
                    "    {}[\"{}\"]\n",
                    node.id,
                    escape_mermaid(&node_label(node))
                ));
            }
        }
        output.push_str("  end\n");
    }
    for relation in &view.relations {
        let source = ids[&relation.source];
        let target = ids[&relation.target];
        match relation.kind {
            DependencyRelationKind::Exact => {
                output.push_str(&format!("  {source} --> {target}\n"));
            }
            DependencyRelationKind::Inferred => {
                output.push_str(&format!("  {source} -. inferred .-> {target}\n"));
            }
            DependencyRelationKind::Ambiguous => {
                output.push_str(&format!("  {source} -. ambiguous .-> {target}\n"));
            }
            DependencyRelationKind::Unresolved => {
                output.push_str(&format!("  {source} -. unresolved .-> {target}\n"));
            }
            DependencyRelationKind::ContextDependent => {
                output.push_str(&format!("  {source} -. context-dependent .-> {target}\n"));
            }
        }
    }
    output.push_str("  classDef local fill:#dbeafe,stroke:#2563eb,color:#172554\n");
    output.push_str("  classDef standard fill:#f3f4f6,stroke:#6b7280,color:#111827\n");
    output.push_str("  classDef installed fill:#dcfce7,stroke:#16a34a,color:#14532d\n");
    output.push_str("  classDef ambiguous fill:#ffedd5,stroke:#ea580c,color:#7c2d12\n");
    output.push_str("  classDef unresolved fill:#fee2e2,stroke:#dc2626,color:#7f1d1d\n");
    output.push_str("  classDef cyclic stroke:#dc2626,stroke-width:3px\n");
    for node in &view.nodes {
        let class = match node.node.kind() {
            DependencyNodeKind::LocalModule => "local",
            DependencyNodeKind::StandardLibrary => "standard",
            DependencyNodeKind::InstalledDistribution => "installed",
            DependencyNodeKind::SystemHeader => "standard",
            DependencyNodeKind::ExternalHeader => "installed",
            DependencyNodeKind::Ambiguous => "ambiguous",
            DependencyNodeKind::Unresolved => "unresolved",
            DependencyNodeKind::ContextDependent => "ambiguous",
        };
        output.push_str(&format!("  class {} {class}\n", node.id));
        if node.cyclic_component.is_some() {
            output.push_str(&format!("  class {} cyclic\n", node.id));
        }
    }
    output
}

pub fn render_dependency_dot(view: &DependencyGraphView) -> String {
    let ids = view
        .nodes
        .iter()
        .map(|node| (node.node.clone(), node.id.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut output = String::from(
        "digraph dependencies {\n  rankdir=LR;\n  graph [fontname=\"Helvetica\"];\n  node [fontname=\"Helvetica\", style=filled];\n  edge [fontname=\"Helvetica\"];\n",
    );
    let cyclic = view
        .nodes
        .iter()
        .filter(|node| node.cyclic_component.is_some())
        .map(|node| node.node.clone())
        .collect::<BTreeSet<_>>();
    for node in view
        .nodes
        .iter()
        .filter(|node| !cyclic.contains(&node.node))
    {
        output.push_str(&dot_node(node, "  "));
    }
    for component in view
        .strongly_connected_components
        .iter()
        .filter(|component| component.cyclic)
    {
        let number = component
            .members
            .iter()
            .find_map(|member| {
                view.nodes
                    .iter()
                    .find(|node| &node.node == member)
                    .and_then(|node| node.cyclic_component)
            })
            .unwrap_or(0);
        output.push_str(&format!("  subgraph cluster_cycle_{number} {{\n    label=\"Cycle {number}\";\n    color=\"#dc2626\";\n"));
        for member in &component.members {
            if let Some(node) = view.nodes.iter().find(|node| &node.node == member) {
                output.push_str(&dot_node(node, "    "));
            }
        }
        output.push_str("  }\n");
    }
    for relation in &view.relations {
        let source = ids[&relation.source];
        let target = ids[&relation.target];
        let attributes = match relation.kind {
            DependencyRelationKind::Exact => "",
            DependencyRelationKind::Inferred => {
                " [style=dashed,color=\"#2563eb\",label=\"inferred\"]"
            }
            DependencyRelationKind::Ambiguous => {
                " [style=dashed,color=\"#ea580c\",label=\"ambiguous\"]"
            }
            DependencyRelationKind::Unresolved => {
                " [style=dashed,color=\"#dc2626\",label=\"unresolved\"]"
            }
            DependencyRelationKind::ContextDependent => {
                " [style=dashed,color=\"#7c3aed\",label=\"context-dependent\"]"
            }
        };
        output.push_str(&format!("  {source} -> {target}{attributes};\n"));
    }
    output.push_str("}\n");
    output
}

fn dot_node(node: &DependencyGraphViewNode, indent: &str) -> String {
    let (shape, fill) = match node.node.kind() {
        DependencyNodeKind::LocalModule => ("box", "#dbeafe"),
        DependencyNodeKind::StandardLibrary => ("ellipse", "#f3f4f6"),
        DependencyNodeKind::InstalledDistribution => ("component", "#dcfce7"),
        DependencyNodeKind::SystemHeader => ("ellipse", "#f3f4f6"),
        DependencyNodeKind::ExternalHeader => ("component", "#dcfce7"),
        DependencyNodeKind::Ambiguous => ("diamond", "#ffedd5"),
        DependencyNodeKind::Unresolved => ("octagon", "#fee2e2"),
        DependencyNodeKind::ContextDependent => ("diamond", "#ede9fe"),
    };
    let color = if node.cyclic_component.is_some() {
        "#dc2626"
    } else {
        "#374151"
    };
    format!(
        "{indent}{} [label=\"{}\",shape={shape},fillcolor=\"{fill}\",color=\"{color}\"];\n",
        node.id,
        escape_dot(&node_label(node))
    )
}

fn node_label(node: &DependencyGraphViewNode) -> String {
    match &node.node {
        DependencyNode::LocalModule(module) => {
            format!("{}\n{}", module.id.qualified_name(), module.path.display())
        }
        DependencyNode::StandardLibrary(module) => {
            format!("{}\nstandard library", module.qualified_name())
        }
        DependencyNode::InstalledDistribution {
            distribution_display_name,
            version,
            ..
        } => format!(
            "{}{}\ninstalled package",
            distribution_display_name,
            version
                .as_ref()
                .map(|value| format!("=={value}"))
                .unwrap_or_default()
        ),
        DependencyNode::SystemHeader { name, .. } => {
            format!("{name}\nsystem header")
        }
        DependencyNode::ExternalHeader { name, .. } => {
            format!("{name}\nexternal header")
        }
        DependencyNode::Ambiguous { requested, .. } => {
            format!("{requested}\nambiguous")
        }
        DependencyNode::Unresolved {
            requested, reason, ..
        } => format!("{requested}\n{}", reason.as_str()),
        DependencyNode::ContextDependent { requested, .. } => {
            format!("{requested}\ncontext dependent")
        }
    }
}

fn escape_mermaid(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "<br/>")
}

fn escape_dot(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyEnvironmentReport {
    pub selection: String,
    pub implementation: String,
    pub python_version: String,
    pub virtual_environment: bool,
    pub distribution_count: usize,
}

#[derive(Debug, Serialize)]
pub struct DependencyJsonReport {
    pub report_schema_version: &'static str,
    pub tool: DependencyJsonTool,
    pub definitions: DependencyDefinitionVersions,
    pub environment: Option<DependencyEnvironmentReport>,
    pub coverage: JsonDependencyCoverage,
    pub input_exclusions: JsonDependencyInputExclusions,
    pub view: JsonDependencyView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<JsonDependencyQuery>,
    pub nodes: Vec<JsonDependencyNode>,
    pub relations: Vec<JsonDependencyRelation>,
    pub strongly_connected_components: Vec<JsonDependencyScc>,
    pub cycle_explanations: Vec<JsonDependencyCycleExplanation>,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyQuery {
    pub kind: &'static str,
    pub from: Option<String>,
    pub to: Option<String>,
    pub unit: Option<String>,
    pub direction: Option<&'static str>,
    pub found: bool,
    pub ordered_units: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DependencyJsonTool {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DependencyDefinitionVersions {
    pub graph: &'static str,
    pub fan_in: &'static str,
    pub fan_out: &'static str,
    pub strongly_connected_components: &'static str,
    pub cycles: &'static str,
    pub cycle_explanations: &'static str,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyCoverage {
    pub total_references: usize,
    pub exact_references: usize,
    pub inferred_references: usize,
    pub ambiguous_references: usize,
    pub unresolved_references: usize,
    pub context_dependent_references: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyInputExclusions {
    pub type_only: bool,
    pub optional: bool,
    pub callable_local: bool,
    pub conditional: bool,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyView {
    pub focus: Vec<String>,
    pub direction: &'static str,
    pub depth: usize,
    pub exact_only: bool,
    pub local_only: bool,
    pub cycles_only: bool,
    pub node_count: usize,
    pub relation_count: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyNode {
    pub id: String,
    pub kind: &'static str,
    pub identity: String,
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outgoing_dependencies_analyzed: Option<bool>,
    pub unresolved_reason: Option<&'static str>,
    pub candidates: Vec<JsonDependencyCandidate>,
    pub fan_in: usize,
    pub fan_out: usize,
    pub cyclic_component: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyCandidate {
    pub kind: &'static str,
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyRelation {
    pub source: String,
    pub target: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_basis: Option<&'static str>,
    pub evidence: Vec<JsonDependencyEvidence>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum JsonDependencyEvidence {
    Import {
        source_path: String,
        module: Option<String>,
        imported_name: Option<String>,
        relative_level: usize,
        line: usize,
        column: usize,
        scope: &'static str,
        usage: &'static str,
        requirement: &'static str,
        conditional: bool,
    },
    Include {
        source_path: String,
        target: String,
        delimiter: &'static str,
        line: usize,
        column: usize,
        conditional: bool,
    },
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyScc {
    pub cyclic: bool,
    pub members: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyCycleExplanation {
    pub component: usize,
    pub members: Vec<String>,
    pub witness_nodes: Vec<String>,
    pub witness_relations: Vec<JsonDependencyRelation>,
    pub recommended_cuts: Vec<JsonDependencyRelation>,
}

impl DependencyJsonReport {
    pub fn from_analysis(
        analysis: &DependencyGraphAnalysis,
        view: &DependencyGraphView,
        environment: Option<DependencyEnvironmentReport>,
    ) -> Self {
        Self::from_analysis_with_query(analysis, view, environment, None)
    }

    pub fn from_analysis_with_query(
        analysis: &DependencyGraphAnalysis,
        view: &DependencyGraphView,
        environment: Option<DependencyEnvironmentReport>,
        query: Option<&DependencyGraphQueryResult>,
    ) -> Self {
        Self::from_analysis_with_query_and_exclusions(
            analysis,
            view,
            environment,
            query,
            DependencyGraphInputExclusions::default(),
        )
    }

    pub fn from_analysis_with_query_and_exclusions(
        analysis: &DependencyGraphAnalysis,
        view: &DependencyGraphView,
        environment: Option<DependencyEnvironmentReport>,
        query: Option<&DependencyGraphQueryResult>,
        exclusions: DependencyGraphInputExclusions,
    ) -> Self {
        let ids = view
            .nodes
            .iter()
            .map(|node| (node.node.clone(), node.id.clone()))
            .collect::<BTreeMap<_, _>>();
        Self {
            report_schema_version: DEPENDENCY_REPORT_SCHEMA_VERSION,
            tool: DependencyJsonTool {
                name: "codegraide",
                version: env!("CARGO_PKG_VERSION"),
            },
            definitions: DependencyDefinitionVersions {
                graph: DEPENDENCY_GRAPH_DEFINITION_VERSION,
                fan_in: DEPENDENCY_FAN_IN_DEFINITION_VERSION,
                fan_out: DEPENDENCY_FAN_OUT_DEFINITION_VERSION,
                strongly_connected_components: DEPENDENCY_SCC_DEFINITION_VERSION,
                cycles: DEPENDENCY_CYCLE_DEFINITION_VERSION,
                cycle_explanations: DEPENDENCY_CYCLE_EXPLANATION_DEFINITION_VERSION,
            },
            environment,
            coverage: coverage_report(analysis.coverage),
            input_exclusions: JsonDependencyInputExclusions {
                type_only: exclusions.type_only,
                optional: exclusions.optional,
                callable_local: exclusions.callable_local,
                conditional: exclusions.conditional,
            },
            view: JsonDependencyView {
                focus: view.filter.focus_modules.clone(),
                direction: view.filter.direction.as_str(),
                depth: view.filter.depth,
                exact_only: view.filter.exact_only,
                local_only: view.filter.local_only,
                cycles_only: view.filter.cycles_only,
                node_count: view.nodes.len(),
                relation_count: view.relations.len(),
            },
            query: query.map(json_query),
            nodes: view.nodes.iter().map(json_node).collect(),
            relations: view
                .relations
                .iter()
                .map(|relation| json_relation(relation, &ids))
                .collect(),
            strongly_connected_components: view
                .strongly_connected_components
                .iter()
                .map(|component| JsonDependencyScc {
                    cyclic: component.cyclic,
                    members: component
                        .members
                        .iter()
                        .filter_map(|node| ids.get(node).cloned())
                        .collect(),
                })
                .collect(),
            cycle_explanations: explain_dependency_cycles(analysis)
                .into_iter()
                .filter(|explanation| {
                    explanation
                        .members
                        .iter()
                        .all(|member| ids.contains_key(member))
                })
                .map(|explanation| JsonDependencyCycleExplanation {
                    component: explanation.component_number,
                    members: explanation
                        .members
                        .iter()
                        .filter_map(|node| ids.get(node).cloned())
                        .collect(),
                    witness_nodes: explanation
                        .witness_nodes
                        .iter()
                        .filter_map(|node| ids.get(node).cloned())
                        .collect(),
                    witness_relations: explanation
                        .witness_relations
                        .iter()
                        .map(|relation| json_relation(relation, &ids))
                        .collect(),
                    recommended_cuts: explanation
                        .recommended_cuts
                        .iter()
                        .map(|relation| json_relation(relation, &ids))
                        .collect(),
                })
                .collect(),
        }
    }
}

fn json_relation(
    relation: &DependencyRelation,
    ids: &BTreeMap<DependencyNode, String>,
) -> JsonDependencyRelation {
    JsonDependencyRelation {
        source: ids[&relation.source].clone(),
        target: ids[&relation.target].clone(),
        kind: relation.kind.as_str(),
        inference_basis: (relation.kind == DependencyRelationKind::Inferred)
            .then_some("unique-repository-suffix"),
        evidence: relation
            .evidence
            .iter()
            .map(|evidence| match &evidence.reference {
                crate::DependencyReference::Import(reference) => JsonDependencyEvidence::Import {
                    source_path: crate::report::json_path(&evidence.source_path),
                    module: reference.module.clone(),
                    imported_name: reference.imported_name.clone(),
                    relative_level: reference.relative_level,
                    line: reference.span.start.line,
                    column: reference.span.start.column,
                    scope: reference.context.scope.as_str(),
                    usage: reference.context.usage.as_str(),
                    requirement: reference.context.requirement.as_str(),
                    conditional: reference.context.conditional,
                },
                crate::DependencyReference::Include(reference) => JsonDependencyEvidence::Include {
                    source_path: crate::report::json_path(&evidence.source_path),
                    target: reference.target.clone(),
                    delimiter: reference.delimiter.as_str(),
                    line: reference.span.start.line,
                    column: reference.span.start.column,
                    conditional: reference.conditional,
                },
            })
            .collect(),
    }
}

fn json_query(result: &DependencyGraphQueryResult) -> JsonDependencyQuery {
    let ordered_units = result
        .nodes
        .iter()
        .filter_map(|node| match node {
            DependencyNode::LocalModule(module) => Some(module.id.qualified_name().to_owned()),
            _ => None,
        })
        .collect();
    match &result.query {
        DependencyGraphQuery::ShortestPath { from, to } => JsonDependencyQuery {
            kind: "shortest-path",
            from: Some(from.clone()),
            to: Some(to.clone()),
            unit: None,
            direction: None,
            found: result.found,
            ordered_units,
        },
        DependencyGraphQuery::Closure { module, direction } => JsonDependencyQuery {
            kind: "closure",
            from: None,
            to: None,
            unit: Some(module.clone()),
            direction: Some(match direction {
                DependencyQueryDirection::Dependencies => "dependencies",
                DependencyQueryDirection::Dependents => "dependents",
            }),
            found: result.found,
            ordered_units,
        },
    }
}

fn coverage_report(coverage: DependencyGraphCoverage) -> JsonDependencyCoverage {
    JsonDependencyCoverage {
        total_references: coverage.total_references,
        exact_references: coverage.exact_references,
        inferred_references: coverage.inferred_references,
        ambiguous_references: coverage.ambiguous_references,
        unresolved_references: coverage.unresolved_references,
        context_dependent_references: coverage.context_dependent_references,
    }
}

fn json_node(node: &DependencyGraphViewNode) -> JsonDependencyNode {
    let (identity, name, path, version, unresolved_reason, candidates) = match &node.node {
        DependencyNode::LocalModule(module) => {
            let path = crate::report::json_path(&module.path);
            (
                format!(
                    "{}:{}@{path}",
                    module.id.language().as_str(),
                    module.id.qualified_name()
                ),
                module.id.qualified_name().to_owned(),
                Some(path),
                None,
                None,
                Vec::new(),
            )
        }
        DependencyNode::StandardLibrary(module) => (
            format!(
                "{}:stdlib:{}",
                module.language().as_str(),
                module.qualified_name()
            ),
            module.qualified_name().to_owned(),
            None,
            None,
            None,
            Vec::new(),
        ),
        DependencyNode::InstalledDistribution {
            language,
            distribution_name,
            distribution_display_name,
            version,
        } => (
            format!("{}:distribution:{distribution_name}", language.as_str()),
            distribution_display_name.clone(),
            None,
            version.clone(),
            None,
            Vec::new(),
        ),
        DependencyNode::SystemHeader { language, name } => (
            format!("{}:system-header:{name}", language.as_str()),
            name.clone(),
            None,
            None,
            None,
            Vec::new(),
        ),
        DependencyNode::ExternalHeader { language, name } => (
            format!("{}:external-header:{name}", language.as_str()),
            name.clone(),
            None,
            None,
            None,
            Vec::new(),
        ),
        DependencyNode::Ambiguous {
            source_module,
            requested,
            candidates,
        } => (
            format!(
                "{}:ambiguous:{}:{requested}",
                source_module.language().as_str(),
                source_module.qualified_name()
            ),
            requested.clone(),
            None,
            None,
            None,
            candidates.iter().map(json_candidate).collect(),
        ),
        DependencyNode::Unresolved {
            source_module,
            requested,
            reason,
        } => (
            format!(
                "{}:unresolved:{}:{requested}:{}",
                source_module.language().as_str(),
                source_module.qualified_name(),
                reason.as_str()
            ),
            requested.clone(),
            None,
            None,
            Some(reason.as_str()),
            Vec::new(),
        ),
        DependencyNode::ContextDependent {
            source_module,
            requested,
            candidates,
            unresolved_reasons,
        } => (
            format!(
                "{}:context-dependent:{}:{requested}",
                source_module.language().as_str(),
                source_module.qualified_name()
            ),
            requested.clone(),
            None,
            None,
            unresolved_reasons.first().map(|reason| reason.as_str()),
            candidates.iter().map(json_candidate).collect(),
        ),
    };
    JsonDependencyNode {
        id: node.id.clone(),
        kind: match &node.node {
            DependencyNode::LocalModule(module) if module.id.language().as_str() == "cpp" => {
                "local-file"
            }
            _ => node.node.kind().as_str(),
        },
        identity,
        name,
        path,
        version,
        outgoing_dependencies_analyzed: match &node.node {
            DependencyNode::LocalModule(module) => Some(module.outgoing_dependencies_analyzed),
            _ => None,
        },
        unresolved_reason,
        candidates,
        fan_in: node.fan_in,
        fan_out: node.fan_out,
        cyclic_component: node.cyclic_component,
    }
}

fn json_candidate(target: &DependencyTarget) -> JsonDependencyCandidate {
    match target {
        DependencyTarget::LocalModule(module) => JsonDependencyCandidate {
            kind: target.kind().as_str(),
            name: module.id.qualified_name().to_owned(),
            path: Some(crate::report::json_path(&module.path)),
            version: None,
        },
        DependencyTarget::StandardLibrary(module) => JsonDependencyCandidate {
            kind: target.kind().as_str(),
            name: module.qualified_name().to_owned(),
            path: None,
            version: None,
        },
        DependencyTarget::InstalledDistribution {
            distribution_display_name,
            version,
            ..
        } => JsonDependencyCandidate {
            kind: target.kind().as_str(),
            name: distribution_display_name.clone(),
            path: None,
            version: version.clone(),
        },
        DependencyTarget::SystemHeader { name, .. }
        | DependencyTarget::ExternalHeader { name, .. } => JsonDependencyCandidate {
            kind: target.kind().as_str(),
            name: name.clone(),
            path: None,
            version: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DependencyReference, DependencyResolutionOutcome, LocalModule, ModuleId,
        ProjectDependencyResolution, ResolutionLevel, SourcePosition, SourceSpan,
        analyze_dependency_graph,
    };

    fn local(name: &str) -> LocalModule {
        LocalModule::new(
            ModuleId::new(crate::LanguageId::new("python"), name),
            format!("{name}.py"),
        )
    }

    fn edge(source: &LocalModule, target: &LocalModule) -> ProjectDependencyResolution {
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
                    start_byte: 0,
                    end_byte: 1,
                    start: SourcePosition { line: 1, column: 0 },
                    end: SourcePosition { line: 1, column: 1 },
                },
            }),
            DependencyResolutionOutcome::exact(DependencyTarget::LocalModule(target.clone())),
        )
    }

    #[test]
    fn focused_views_are_cycle_complete_and_keep_global_metrics() {
        let a = local("a");
        let b = local("b");
        let c = local("c");
        let analysis =
            analyze_dependency_graph(&[a.clone(), b.clone(), c], &[edge(&a, &b), edge(&b, &a)])
                .expect("graph");
        let view = filter_dependency_graph(
            &analysis,
            &DependencyGraphFilter {
                focus_modules: vec!["a".to_owned()],
                depth: 0,
                ..DependencyGraphFilter::default()
            },
        )
        .expect("view");
        assert_eq!(view.nodes.len(), 2);
        assert!(
            view.nodes
                .iter()
                .all(|node| node.cyclic_component == Some(1))
        );
        assert!(
            view.nodes
                .iter()
                .all(|node| node.fan_in == 1 && node.fan_out == 1)
        );
    }

    #[test]
    fn renderers_emit_valid_directed_graph_envelopes() {
        let a = local("end");
        let analysis = analyze_dependency_graph(&[a], &[]).expect("graph");
        let view =
            filter_dependency_graph(&analysis, &DependencyGraphFilter::default()).expect("view");
        let mermaid = render_dependency_mermaid(&view);
        let dot = render_dependency_dot(&view);
        assert!(mermaid.starts_with("flowchart LR\n"));
        assert!(mermaid.contains("[\"end<br/>end.py\"]"));
        assert!(!mermaid.contains("in 0 · out 0"));
        assert!(dot.starts_with("digraph dependencies"));
        assert!(dot.ends_with("}\n"));
    }

    #[test]
    fn exact_only_removes_uncertain_placeholder_nodes() {
        let a = local("a");
        let unresolved = ProjectDependencyResolution::new(
            a.path.clone(),
            a.id.clone(),
            DependencyReference::Import(crate::ImportReference {
                module: Some("missing".to_owned()),
                imported_name: None,
                alias: None,
                relative_level: 0,
                wildcard: false,
                resolution: ResolutionLevel::Syntactic,
                enclosing_symbol: None,
                context: crate::ImportContext::default(),
                span: SourceSpan {
                    start_byte: 0,
                    end_byte: 1,
                    start: SourcePosition { line: 1, column: 0 },
                    end: SourcePosition { line: 1, column: 1 },
                },
            }),
            DependencyResolutionOutcome::unresolved(
                "missing",
                crate::UnresolvedDependencyReason::ModuleNotFound,
            ),
        );
        let analysis = analyze_dependency_graph(&[a], &[unresolved]).expect("graph");
        let view = filter_dependency_graph(
            &analysis,
            &DependencyGraphFilter {
                exact_only: true,
                ..DependencyGraphFilter::default()
            },
        )
        .expect("view");

        assert_eq!(view.nodes.len(), 1);
        assert!(view.relations.is_empty());
    }
}
