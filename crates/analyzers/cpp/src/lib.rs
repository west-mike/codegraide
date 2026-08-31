use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use codegraide_core::{
    AnalysisDiagnostic, AnalysisFacts, AnalysisInput, AnalysisLevel, AnalyzerCapability,
    AnalyzerDescriptor, DecisionEvent, DecisionEventKind, DependencyReference, DiagnosticSeverity,
    FileAnalysis, FileAnalysisStatus, GrammarDescriptor, IncludeDelimiter, IncludeReference,
    LanguageAnalyzer, LanguageId, Measurement, MeasurementConcept, MeasurementDescriptor,
    MeasurementStatus, NestingEvent, NestingEventKind, QueryDescriptor, ResolutionLevel,
    SourcePosition, SourceSpan, Symbol, SymbolCompleteness, SymbolId, SymbolKind,
};
use tree_sitter::{Language, Node, Parser, Query};

pub const CPP_CYCLOMATIC_COMPLEXITY: &str = "cpp-cyclomatic-complexity";
pub const CPP_CYCLOMATIC_COMPLEXITY_DEFINITION_VERSION: &str = "cpp-cyclomatic-complexity-v1";
pub const CPP_MAX_CONTROL_FLOW_NESTING: &str = "cpp-max-control-flow-nesting";
pub const CPP_MAX_CONTROL_FLOW_NESTING_DEFINITION_VERSION: &str = "cpp-max-control-flow-nesting-v1";

const ANALYZER_VERSION: &str = "0.1.0";
const GRAMMAR_VERSION: &str = "0.23.4";

pub struct CppAnalyzer {
    descriptor: AnalyzerDescriptor,
    parser: Parser,
    _symbol_query: Query,
    _include_query: Query,
    _nesting_query: Query,
    _decision_query: Query,
}

#[derive(Debug)]
pub struct CppAnalyzerError(String);

impl fmt::Display for CppAnalyzerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not initialize the C++ analyzer: {}",
            self.0
        )
    }
}

impl std::error::Error for CppAnalyzerError {}

impl CppAnalyzer {
    pub fn new() -> Result<Self, CppAnalyzerError> {
        let language: Language = tree_sitter_cpp::LANGUAGE.into();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|error| CppAnalyzerError(error.to_string()))?;
        let symbol_query = query(&language, "symbols", include_str!("../queries/symbols.scm"))?;
        let include_query = query(
            &language,
            "includes",
            include_str!("../queries/includes.scm"),
        )?;
        let nesting_query = query(&language, "nesting", include_str!("../queries/nesting.scm"))?;
        let decision_query = query(
            &language,
            "decisions",
            include_str!("../queries/decisions.scm"),
        )?;

        Ok(Self {
            descriptor: AnalyzerDescriptor {
                id: "cpp-tree-sitter".to_owned(),
                language: LanguageId::new("cpp"),
                version: ANALYZER_VERSION.to_owned(),
                level: AnalysisLevel::Syntax,
                capabilities: [
                    AnalyzerCapability::Parse,
                    AnalyzerCapability::Symbols,
                    AnalyzerCapability::DependencyReferences,
                    AnalyzerCapability::DecisionEvents,
                    AnalyzerCapability::NestingEvents,
                    AnalyzerCapability::Measurements,
                ]
                .into_iter()
                .collect(),
                grammar: Some(GrammarDescriptor {
                    name: "tree-sitter-cpp".to_owned(),
                    version: GRAMMAR_VERSION.to_owned(),
                }),
                queries: vec![
                    QueryDescriptor {
                        name: "symbols".to_owned(),
                        version: "cpp-symbols-v1".to_owned(),
                    },
                    QueryDescriptor {
                        name: "includes".to_owned(),
                        version: "cpp-includes-v1".to_owned(),
                    },
                    QueryDescriptor {
                        name: "nesting".to_owned(),
                        version: "cpp-nesting-v1".to_owned(),
                    },
                    QueryDescriptor {
                        name: "decisions".to_owned(),
                        version: "cpp-decisions-v1".to_owned(),
                    },
                ],
                measurements: vec![
                    measurement_descriptor(
                        MeasurementConcept::DeclarationPhysicalLines,
                        "function-declaration-physical-lines",
                        "lines",
                    ),
                    measurement_descriptor(
                        MeasurementConcept::BodyPhysicalLines,
                        "function-body-physical-lines",
                        "lines",
                    ),
                    measurement_descriptor(
                        MeasurementConcept::MaxControlFlowNesting,
                        CPP_MAX_CONTROL_FLOW_NESTING,
                        "levels",
                    ),
                    measurement_descriptor(
                        MeasurementConcept::CyclomaticComplexity,
                        CPP_CYCLOMATIC_COMPLEXITY,
                        "score",
                    ),
                ],
                limitations: vec![
                    "C++ facts are syntax-derived; preprocessing, macro expansion, include search paths, types, and overload resolution are not performed."
                        .to_owned(),
                    "Conditional preprocessing inside a callable makes its complexity and nesting unavailable without an active build configuration."
                        .to_owned(),
                    "Macro-expanded control flow is not visible, so otherwise measured complexity is a syntactic lower bound."
                        .to_owned(),
                    "Templates are parsed without instantiation, and overloads retain syntax-level identities only."
                        .to_owned(),
                    "Out-of-class qualified definitions remain functions unless lexical class or struct ownership is present."
                        .to_owned(),
                    "Function prototypes, forward declarations, unions, enums, and C++ modules are not emitted as symbols in v1."
                        .to_owned(),
                    "Ambiguous .h headers require future explicit or project-aware language selection."
                        .to_owned(),
                    "Non-UTF-8 source is parsed by byte span but extracted names use lossy decoding."
                        .to_owned(),
                ],
            },
            parser,
            _symbol_query: symbol_query,
            _include_query: include_query,
            _nesting_query: nesting_query,
            _decision_query: decision_query,
        })
    }
}

impl LanguageAnalyzer for CppAnalyzer {
    fn descriptor(&self) -> &AnalyzerDescriptor {
        &self.descriptor
    }

    fn analyze(&mut self, input: AnalysisInput<'_>) -> FileAnalysis {
        let Some(tree) = self.parser.parse(input.source, None) else {
            return FileAnalysis {
                path: input.path.to_path_buf(),
                status: FileAnalysisStatus::Failed,
                diagnostics: vec![AnalysisDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: "parse-failed".to_owned(),
                    message: "the C++ parser did not produce a syntax tree".to_owned(),
                    span: None,
                }],
                facts: AnalysisFacts::default(),
            };
        };

        let mut diagnostics = Vec::new();
        if std::str::from_utf8(input.source).is_err() {
            diagnostics.push(AnalysisDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "source-not-utf8".to_owned(),
                message: "C++ source is not UTF-8; extracted names may be lossy".to_owned(),
                span: None,
            });
        }
        collect_diagnostics(tree.root_node(), false, &mut diagnostics);
        diagnostics.sort_by(diagnostic_order);

        let mut extraction = Extraction::new(input.path, input.source);
        extraction.visit_node(tree.root_node(), None, None, 0);
        extraction.finish_measurements();

        FileAnalysis {
            path: input.path.to_path_buf(),
            status: if diagnostics.is_empty() {
                FileAnalysisStatus::Successful
            } else {
                FileAnalysisStatus::Partial
            },
            diagnostics,
            facts: AnalysisFacts {
                symbols: extraction.symbols,
                dependencies: extraction.includes,
                calls: Vec::new(),
                explicit_exports: None,
            },
        }
    }
}

fn query(language: &Language, name: &str, source: &str) -> Result<Query, CppAnalyzerError> {
    Query::new(language, source)
        .map_err(|error| CppAnalyzerError(format!("invalid {name} query: {error}")))
}

fn measurement_descriptor(
    concept: MeasurementConcept,
    id: &str,
    unit: &str,
) -> MeasurementDescriptor {
    MeasurementDescriptor {
        concept,
        id: id.to_owned(),
        definition_version: format!("{id}-v1"),
        unit: unit.to_owned(),
    }
}

struct Extraction<'a> {
    path: &'a Path,
    source: &'a [u8],
    symbols: Vec<Symbol>,
    includes: Vec<DependencyReference>,
    id_counts: BTreeMap<String, usize>,
    preprocessor_uncertain: BTreeSet<SymbolId>,
}

impl<'a> Extraction<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        Self {
            path,
            source,
            symbols: Vec::new(),
            includes: Vec::new(),
            id_counts: BTreeMap::new(),
            preprocessor_uncertain: BTreeSet::new(),
        }
    }

    fn visit_node(
        &mut self,
        node: Node<'_>,
        parent_id: Option<SymbolId>,
        callable_id: Option<SymbolId>,
        depth: usize,
    ) {
        match node.kind() {
            "namespace_definition" => {
                self.process_container(node, parent_id, callable_id, depth, SymbolKind::Namespace);
                return;
            }
            "class_specifier" => {
                self.process_container(node, parent_id, callable_id, depth, SymbolKind::Class);
                return;
            }
            "struct_specifier" => {
                self.process_container(node, parent_id, callable_id, depth, SymbolKind::Struct);
                return;
            }
            "function_definition" => {
                self.process_function(node, parent_id);
                return;
            }
            "lambda_expression" => {
                self.process_lambda(node, parent_id);
                return;
            }
            "preproc_include" => {
                self.process_include(node, parent_id);
                return;
            }
            _ => {}
        }

        if is_conditional_preprocessor(node.kind())
            && let Some(callable_id) = callable_id.as_ref()
        {
            self.preprocessor_uncertain.insert(callable_id.clone());
        }

        if let Some(callable_id) = callable_id.as_ref()
            && let Some(kind) = decision_kind(node, self.source)
        {
            self.push_decision(callable_id, kind, source_span(node));
        }

        let mut child_depth = depth;
        if let Some(callable_id) = callable_id.as_ref()
            && let Some(kind) = nesting_kind(node.kind())
        {
            let same_level = is_else_if(node) || node.kind() == "catch_clause";
            let event_depth = if same_level { depth.max(1) } else { depth + 1 };
            self.push_nesting(callable_id, kind, event_depth, source_span(node));
            child_depth = event_depth;
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit_node(child, parent_id.clone(), callable_id.clone(), child_depth);
        }
    }

    fn process_container(
        &mut self,
        node: Node<'_>,
        parent_id: Option<SymbolId>,
        callable_id: Option<SymbolId>,
        depth: usize,
        kind: SymbolKind,
    ) {
        let name_node = node.child_by_field_name("name");
        let position = node.start_position();
        let name = name_node.map_or_else(
            || {
                format!(
                    "<anonymous-{}>@{}:{}",
                    kind.as_str(),
                    position.row + 1,
                    position.column + 1
                )
            },
            |name| node_text(name, self.source),
        );
        let qualified_name = self.qualified_name(parent_id.as_ref(), &name);
        let id = self.symbol_id(kind, &qualified_name);
        let span = source_span(node);
        let body = node.child_by_field_name("body");
        self.symbols.push(Symbol {
            id: id.clone(),
            parent_id: parent_id.clone(),
            kind,
            direct_declaration: !has_conditional_preprocessor_ancestor(node),
            name,
            qualified_name,
            span,
            body_span: body.map(source_span),
            name_span: name_node.map(source_span),
            completeness: completeness(node),
            modifiers: BTreeSet::new(),
            parameters: Vec::new(),
            decorators: Vec::new(),
            documentation: None,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        if let Some(body) = body {
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                self.visit_node(child, Some(id.clone()), callable_id.clone(), depth);
            }
        }
    }

    fn process_function(&mut self, node: Node<'_>, parent_id: Option<SymbolId>) {
        let Some(declarator) = node.child_by_field_name("declarator") else {
            return;
        };
        let Some(name_node) = declarator_name_node(declarator) else {
            return;
        };
        let name = node_text(name_node, self.source);
        let parent_kind = parent_id
            .as_ref()
            .and_then(|id| self.symbol(id))
            .map(|symbol| symbol.kind);
        let kind = if matches!(parent_kind, Some(SymbolKind::Class | SymbolKind::Struct)) {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        let qualified_name = self.qualified_name(parent_id.as_ref(), &name);
        let id = self.symbol_id(kind, &qualified_name);
        let declaration = declaration_wrapper(node);
        let body = node.child_by_field_name("body");
        self.symbols.push(Symbol {
            id: id.clone(),
            parent_id,
            kind,
            direct_declaration: !has_conditional_preprocessor_ancestor(node),
            name,
            qualified_name,
            span: source_span(declaration),
            body_span: body.map(source_span),
            name_span: Some(source_span(name_node)),
            completeness: completeness(node),
            modifiers: BTreeSet::new(),
            parameters: Vec::new(),
            decorators: Vec::new(),
            documentation: None,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        if let Some(body) = body {
            self.visit_node(body, Some(id.clone()), Some(id), 0);
        }
    }

    fn process_lambda(&mut self, node: Node<'_>, parent_id: Option<SymbolId>) {
        let position = node.start_position();
        let name = format!("<lambda>@{}:{}", position.row + 1, position.column + 1);
        let qualified_name = self.qualified_name(parent_id.as_ref(), &name);
        let id = self.symbol_id(SymbolKind::Lambda, &qualified_name);
        let body = node.child_by_field_name("body");
        self.symbols.push(Symbol {
            id: id.clone(),
            parent_id,
            kind: SymbolKind::Lambda,
            direct_declaration: false,
            name: "<lambda>".to_owned(),
            qualified_name,
            span: source_span(node),
            body_span: body.map(source_span),
            name_span: None,
            completeness: completeness(node),
            modifiers: BTreeSet::new(),
            parameters: Vec::new(),
            decorators: Vec::new(),
            documentation: None,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        if let Some(body) = body {
            self.visit_node(body, Some(id.clone()), Some(id), 0);
        }
    }

    fn process_include(&mut self, node: Node<'_>, parent_id: Option<SymbolId>) {
        let text = node_text(node, self.source);
        let value = text
            .trim_start()
            .strip_prefix('#')
            .map(str::trim_start)
            .and_then(|value| value.strip_prefix("include"))
            .map(str::trim)
            .unwrap_or_default();
        let (target, delimiter) = if value.starts_with('<') && value.ends_with('>') {
            (
                value[1..value.len() - 1].to_owned(),
                IncludeDelimiter::Angle,
            )
        } else if value.starts_with('"') && value.ends_with('"') {
            (
                value[1..value.len() - 1].to_owned(),
                IncludeDelimiter::Quote,
            )
        } else {
            (value.to_owned(), IncludeDelimiter::Macro)
        };
        self.includes
            .push(DependencyReference::Include(IncludeReference {
                target,
                delimiter,
                conditional: has_conditional_preprocessor_ancestor(node),
                resolution: ResolutionLevel::Syntactic,
                enclosing_symbol: parent_id,
                span: source_span(node),
            }));
    }

    fn qualified_name(&self, parent_id: Option<&SymbolId>, name: &str) -> String {
        let Some(parent) = parent_id.and_then(|id| self.symbol(id)) else {
            return name.to_owned();
        };
        if name.starts_with(&format!("{}::", parent.qualified_name)) {
            name.to_owned()
        } else {
            format!("{}::{name}", parent.qualified_name)
        }
    }

    fn symbol_id(&mut self, kind: SymbolKind, qualified_name: &str) -> SymbolId {
        let base = format!(
            "{}::{}:{qualified_name}",
            path_string(self.path),
            kind.as_str()
        );
        let ordinal = self.id_counts.entry(base.clone()).or_default();
        *ordinal += 1;
        SymbolId::new(format!("{base}#{ordinal}"))
    }

    fn symbol(&self, id: &SymbolId) -> Option<&Symbol> {
        self.symbols.iter().find(|symbol| &symbol.id == id)
    }

    fn symbol_mut(&mut self, id: &SymbolId) -> Option<&mut Symbol> {
        self.symbols.iter_mut().find(|symbol| &symbol.id == id)
    }

    fn push_decision(&mut self, id: &SymbolId, kind: DecisionEventKind, span: SourceSpan) {
        if let Some(symbol) = self.symbol_mut(id) {
            symbol.decision_events.push(DecisionEvent { kind, span });
        }
    }

    fn push_nesting(
        &mut self,
        id: &SymbolId,
        kind: NestingEventKind,
        depth: usize,
        span: SourceSpan,
    ) {
        if let Some(symbol) = self.symbol_mut(id) {
            symbol
                .nesting_events
                .push(NestingEvent { kind, depth, span });
        }
    }

    fn finish_measurements(&mut self) {
        for symbol in &mut self.symbols {
            if !matches!(
                symbol.kind,
                SymbolKind::Function | SymbolKind::Method | SymbolKind::Lambda
            ) {
                continue;
            }
            let partial = symbol.completeness == SymbolCompleteness::Partial;
            let conditional = self.preprocessor_uncertain.contains(&symbol.id);
            for (id, definition_version, unit, value) in [
                (
                    "function-declaration-physical-lines",
                    "function-declaration-physical-lines-v1",
                    "lines",
                    Some(line_count(symbol.span)),
                ),
                (
                    "function-body-physical-lines",
                    "function-body-physical-lines-v1",
                    "lines",
                    symbol.body_span.map(line_count),
                ),
                (
                    CPP_MAX_CONTROL_FLOW_NESTING,
                    CPP_MAX_CONTROL_FLOW_NESTING_DEFINITION_VERSION,
                    "levels",
                    Some(
                        symbol
                            .nesting_events
                            .iter()
                            .map(|event| event.depth as u64)
                            .max()
                            .unwrap_or(0),
                    ),
                ),
                (
                    CPP_CYCLOMATIC_COMPLEXITY,
                    CPP_CYCLOMATIC_COMPLEXITY_DEFINITION_VERSION,
                    "score",
                    Some(1 + symbol.decision_events.len() as u64),
                ),
            ] {
                let control_flow =
                    matches!(id, CPP_MAX_CONTROL_FLOW_NESTING | CPP_CYCLOMATIC_COMPLEXITY);
                let reason = if partial {
                    Some("symbol contains parser recovery nodes".to_owned())
                } else if conditional && control_flow {
                    Some(
                        "conditional preprocessing prevents selecting one active control-flow graph"
                            .to_owned(),
                    )
                } else if value.is_none() {
                    Some("symbol body is unavailable".to_owned())
                } else {
                    None
                };
                symbol.measurements.push(Measurement {
                    id: id.to_owned(),
                    definition_version: definition_version.to_owned(),
                    unit: unit.to_owned(),
                    status: if reason.is_some() {
                        MeasurementStatus::Unavailable
                    } else {
                        MeasurementStatus::Measured
                    },
                    value: reason.is_none().then_some(value).flatten(),
                    reason,
                });
            }
            symbol.decision_events.sort_by(event_order);
            symbol.nesting_events.sort_by(nesting_order);
        }
        self.symbols.sort_by(|left, right| {
            left.span
                .start_byte
                .cmp(&right.span.start_byte)
                .then_with(|| left.span.end_byte.cmp(&right.span.end_byte))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.includes.sort_by(|left, right| {
            left.span()
                .start_byte
                .cmp(&right.span().start_byte)
                .then_with(|| left.span().end_byte.cmp(&right.span().end_byte))
        });
    }
}

fn decision_kind(node: Node<'_>, source: &[u8]) -> Option<DecisionEventKind> {
    match node.kind() {
        "if_statement" => Some(DecisionEventKind::Conditional),
        "for_statement" | "for_range_loop" | "while_statement" | "do_statement" => {
            Some(DecisionEventKind::Loop)
        }
        "catch_clause" => Some(DecisionEventKind::ExceptionHandler),
        "case_statement" if !node_text(node, source).trim_start().starts_with("default") => {
            Some(DecisionEventKind::SwitchCase)
        }
        "conditional_expression" => Some(DecisionEventKind::ConditionalExpression),
        "binary_expression" => node
            .child_by_field_name("operator")
            .map(|operator| node_text(operator, source))
            .filter(|operator| matches!(operator.as_str(), "&&" | "||"))
            .map(|_| DecisionEventKind::BooleanShortCircuit),
        _ => None,
    }
}

fn nesting_kind(kind: &str) -> Option<NestingEventKind> {
    match kind {
        "if_statement" => Some(NestingEventKind::Conditional),
        "for_statement" | "for_range_loop" | "while_statement" | "do_statement" => {
            Some(NestingEventKind::Loop)
        }
        "switch_statement" => Some(NestingEventKind::Switch),
        "try_statement" | "catch_clause" => Some(NestingEventKind::ExceptionHandling),
        _ => None,
    }
}

fn declarator_name_node(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "identifier"
        | "field_identifier"
        | "operator_name"
        | "destructor_name"
        | "qualified_identifier"
        | "template_function"
        | "template_method" => return Some(node),
        _ => {}
    }
    if let Some(declarator) = node.child_by_field_name("declarator") {
        return declarator_name_node(declarator);
    }
    if let Some(name) = node.child_by_field_name("name") {
        return declarator_name_node(name).or(Some(name));
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(declarator_name_node)
}

fn declaration_wrapper(mut node: Node<'_>) -> Node<'_> {
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "template_declaration" | "declaration") {
            node = parent;
        } else {
            break;
        }
    }
    node
}

fn completeness(node: Node<'_>) -> SymbolCompleteness {
    if node.has_error() || span_has_error(node) {
        SymbolCompleteness::Partial
    } else {
        SymbolCompleteness::Complete
    }
}

fn span_has_error(node: Node<'_>) -> bool {
    if node.is_error() || node.is_missing() {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).any(span_has_error)
}

fn collect_diagnostics(
    node: Node<'_>,
    ancestor_is_error: bool,
    diagnostics: &mut Vec<AnalysisDiagnostic>,
) {
    let is_error = node.is_error();
    if is_error && !ancestor_is_error {
        diagnostics.push(AnalysisDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "parse-error".to_owned(),
            message: "C++ parser recovered from invalid or unsupported syntax".to_owned(),
            span: Some(source_span(node)),
        });
    } else if node.is_missing() {
        diagnostics.push(AnalysisDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "missing-syntax".to_owned(),
            message: format!("C++ parser inserted missing {} syntax", node.kind()),
            span: Some(source_span(node)),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_diagnostics(child, ancestor_is_error || is_error, diagnostics);
    }
}

fn is_conditional_preprocessor(kind: &str) -> bool {
    matches!(kind, "preproc_if" | "preproc_ifdef" | "preproc_elif")
}

fn has_conditional_preprocessor_ancestor(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if is_conditional_preprocessor(ancestor.kind()) {
            return true;
        }
        parent = ancestor.parent();
    }
    false
}

fn is_else_if(node: Node<'_>) -> bool {
    node.kind() == "if_statement"
        && node.parent().is_some_and(|parent| {
            parent.kind() == "else_clause"
                && parent
                    .parent()
                    .is_some_and(|grandparent| grandparent.kind() == "if_statement")
        })
}

fn node_text(node: Node<'_>, source: &[u8]) -> String {
    String::from_utf8_lossy(
        source
            .get(node.start_byte()..node.end_byte())
            .unwrap_or_default(),
    )
    .into_owned()
}

fn source_span(node: Node<'_>) -> SourceSpan {
    SourceSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start: SourcePosition {
            line: node.start_position().row + 1,
            column: node.start_position().column + 1,
        },
        end: SourcePosition {
            line: node.end_position().row + 1,
            column: node.end_position().column + 1,
        },
    }
}

fn line_count(span: SourceSpan) -> u64 {
    (span.end.line.saturating_sub(span.start.line) + 1) as u64
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn diagnostic_order(left: &AnalysisDiagnostic, right: &AnalysisDiagnostic) -> std::cmp::Ordering {
    left.span
        .map(|span| (span.start_byte, span.end_byte))
        .cmp(&right.span.map(|span| (span.start_byte, span.end_byte)))
        .then_with(|| left.code.cmp(&right.code))
}

fn event_order(left: &DecisionEvent, right: &DecisionEvent) -> std::cmp::Ordering {
    left.span
        .start_byte
        .cmp(&right.span.start_byte)
        .then_with(|| left.span.end_byte.cmp(&right.span.end_byte))
        .then_with(|| left.kind.cmp(&right.kind))
}

fn nesting_order(left: &NestingEvent, right: &NestingEvent) -> std::cmp::Ordering {
    left.span
        .start_byte
        .cmp(&right.span.start_byte)
        .then_with(|| left.span.end_byte.cmp(&right.span.end_byte))
        .then_with(|| left.kind.cmp(&right.kind))
}
