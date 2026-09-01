use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use codegraide_core::{
    AnalysisDiagnostic, AnalysisFacts, AnalysisInput, AnalysisLevel, AnalyzerCapability,
    AnalyzerDescriptor, CallArgumentShape, CallReference, DecisionEvent, DecisionEventKind,
    Decorator, DependencyReference, DiagnosticSeverity, DocumentationStatus, FileAnalysis,
    FileAnalysisStatus, GrammarDescriptor, ImportContext, ImportReference, ImportRequirement,
    ImportScope, ImportUsage, LanguageAnalyzer, LanguageId, Measurement, MeasurementConcept,
    MeasurementDescriptor, MeasurementStatus, NestingEvent, NestingEventKind, Parameter,
    ParameterKind, QueryDescriptor, ResolutionLevel, SourcePosition, SourceSpan, Symbol,
    SymbolCompleteness, SymbolDocumentation, SymbolId, SymbolKind, SymbolModifier,
};
use tree_sitter::{Language, Node, Parser, Query};

mod call_resolution;
mod explicit_exports;
mod resolution;

pub use call_resolution::{PythonCallResolution, resolve_python_calls};
pub use resolution::{
    PYTHON_IMPORT_RESOLUTION_DEFINITION_VERSION, PythonDependencyResolution,
    PythonDependencyResolver, PythonEnvironmentSelection, PythonEnvironmentSummary,
    PythonResolutionError, PythonResolutionOptions, resolve_python_dependencies,
};

const ANALYZER_VERSION: &str = "0.3.0";
const GRAMMAR_VERSION: &str = "0.25.0";
pub const PYTHON_CYCLOMATIC_COMPLEXITY: &str = "python-cyclomatic-complexity";
pub const PYTHON_CYCLOMATIC_COMPLEXITY_DEFINITION_VERSION: &str = "python-cyclomatic-complexity-v1";

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

pub struct PythonAnalyzer {
    descriptor: AnalyzerDescriptor,
    parser: Parser,
    documentation_enabled: bool,
    _symbol_query: Query,
    _import_query: Query,
    _nesting_query: Query,
    _decision_query: Query,
}

#[derive(Debug)]
pub struct PythonAnalyzerError(String);

impl fmt::Display for PythonAnalyzerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not initialize the Python analyzer: {}",
            self.0
        )
    }
}

impl std::error::Error for PythonAnalyzerError {}

impl PythonAnalyzer {
    pub fn new() -> Result<Self, PythonAnalyzerError> {
        Self::with_documentation(true)
    }

    pub fn without_documentation() -> Result<Self, PythonAnalyzerError> {
        Self::with_documentation(false)
    }

    fn with_documentation(documentation_enabled: bool) -> Result<Self, PythonAnalyzerError> {
        let language: Language = tree_sitter_python::LANGUAGE.into();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|error| PythonAnalyzerError(error.to_string()))?;
        let symbol_query = Query::new(&language, include_str!("../queries/symbols.scm"))
            .map_err(|error| PythonAnalyzerError(format!("invalid symbols query: {error}")))?;
        let import_query = Query::new(&language, include_str!("../queries/imports.scm"))
            .map_err(|error| PythonAnalyzerError(format!("invalid imports query: {error}")))?;
        let nesting_query = Query::new(&language, include_str!("../queries/nesting.scm"))
            .map_err(|error| PythonAnalyzerError(format!("invalid nesting query: {error}")))?;
        let decision_query = Query::new(&language, include_str!("../queries/decisions.scm"))
            .map_err(|error| PythonAnalyzerError(format!("invalid decision query: {error}")))?;

        let mut capabilities = [
            AnalyzerCapability::Parse,
            AnalyzerCapability::Symbols,
            AnalyzerCapability::DependencyReferences,
            AnalyzerCapability::CallReferences,
            AnalyzerCapability::DecisionEvents,
            AnalyzerCapability::NestingEvents,
            AnalyzerCapability::Measurements,
            AnalyzerCapability::ExplicitExports,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        let mut queries = vec![
            QueryDescriptor {
                name: "symbols".to_owned(),
                version: "python-symbols-v1".to_owned(),
            },
            QueryDescriptor {
                name: "imports".to_owned(),
                version: "python-imports-v1".to_owned(),
            },
            QueryDescriptor {
                name: "import-context".to_owned(),
                version: "python-import-context-v1".to_owned(),
            },
            QueryDescriptor {
                name: "calls".to_owned(),
                version: "python-call-references-v1".to_owned(),
            },
            QueryDescriptor {
                name: "nesting".to_owned(),
                version: "python-nesting-v1".to_owned(),
            },
            QueryDescriptor {
                name: "decisions".to_owned(),
                version: "python-decisions-v1".to_owned(),
            },
            QueryDescriptor {
                name: "explicit-exports".to_owned(),
                version: "python-explicit-exports-v1".to_owned(),
            },
        ];
        if documentation_enabled {
            capabilities.insert(AnalyzerCapability::Documentation);
            queries.push(QueryDescriptor {
                name: "docstrings".to_owned(),
                version: "python-docstrings-v1".to_owned(),
            });
        }

        let mut limitations = vec![
            "Reports parser recovery and syntax-derived facts; it does not lint or type-check Python."
                .to_owned(),
            "Module names are repository-relative paths until project or environment resolution is added."
                .to_owned(),
            "Dynamic imports and runtime metaprogramming are not resolved.".to_owned(),
            "Call resolution is conservative: arbitrary instance dispatch, inheritance lookup, decorators, assignment aliases, and higher-order values are not guessed."
                .to_owned(),
            "Non-UTF-8 source text is parsed by byte span but names and snippets use lossy decoding."
                .to_owned(),
            "Cyclomatic complexity covers callable bodies; module and class initialization bodies are not scored in v1."
                .to_owned(),
            "Explicit exports statically evaluate direct module-level __all__ list/tuple assignments, += updates, append, and extend; conditional updates, aliases, arbitrary calls, escaped string values, and other dynamic behavior remain partial or unavailable."
                .to_owned(),
        ];
        if documentation_enabled {
            limitations.push(
                "Documentation coverage recognizes nonblank conventional docstrings lexically; escape sequences are not decoded."
                    .to_owned(),
            );
        }

        Ok(Self {
            descriptor: AnalyzerDescriptor {
                id: "python-tree-sitter".to_owned(),
                language: LanguageId::new("python"),
                version: ANALYZER_VERSION.to_owned(),
                level: AnalysisLevel::Syntax,
                capabilities,
                grammar: Some(GrammarDescriptor {
                    name: "tree-sitter-python".to_owned(),
                    version: GRAMMAR_VERSION.to_owned(),
                }),
                queries,
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
                        MeasurementConcept::DeclaredParameterCount,
                        "declared-parameter-count",
                        "parameters",
                    ),
                    measurement_descriptor(
                        MeasurementConcept::CallerParameterCount,
                        "caller-parameter-count",
                        "parameters",
                    ),
                    measurement_descriptor(
                        MeasurementConcept::MaxControlFlowNesting,
                        "python-max-control-flow-nesting",
                        "levels",
                    ),
                    measurement_descriptor(
                        MeasurementConcept::CyclomaticComplexity,
                        PYTHON_CYCLOMATIC_COMPLEXITY,
                        "score",
                    ),
                ],
                limitations,
            },
            parser,
            documentation_enabled,
            _symbol_query: symbol_query,
            _import_query: import_query,
            _nesting_query: nesting_query,
            _decision_query: decision_query,
        })
    }
}

impl LanguageAnalyzer for PythonAnalyzer {
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
                    message: "the Python parser did not produce a syntax tree".to_owned(),
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
                message: "Python source is not UTF-8; extracted names may be lossy".to_owned(),
                span: None,
            });
        }
        collect_diagnostics(tree.root_node(), false, &mut diagnostics);
        diagnostics.sort_by(|left, right| diagnostic_key(left).cmp(&diagnostic_key(right)));

        let explicit_exports = explicit_exports::extract(tree.root_node(), input.source);
        let mut extraction = Extraction::new(input.path, input.source, self.documentation_enabled);
        extraction.extract_module(tree.root_node());
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
                dependencies: extraction.dependencies,
                calls: extraction.calls,
                explicit_exports: Some(explicit_exports),
            },
        }
    }
}

struct Extraction<'a> {
    path: &'a Path,
    source: &'a [u8],
    symbols: Vec<Symbol>,
    dependencies: Vec<DependencyReference>,
    calls: Vec<CallReference>,
    id_counts: BTreeMap<String, usize>,
    typing_aliases: BTreeSet<String>,
    type_checking_names: BTreeSet<String>,
    documentation_enabled: bool,
}

impl<'a> Extraction<'a> {
    fn new(path: &'a Path, source: &'a [u8], documentation_enabled: bool) -> Self {
        Self {
            path,
            source,
            symbols: Vec::new(),
            dependencies: Vec::new(),
            calls: Vec::new(),
            id_counts: BTreeMap::new(),
            typing_aliases: BTreeSet::new(),
            type_checking_names: BTreeSet::new(),
            documentation_enabled,
        }
    }

    fn extract_module(&mut self, root: Node<'_>) {
        collect_type_checking_aliases(
            root,
            self.source,
            &mut self.typing_aliases,
            &mut self.type_checking_names,
        );
        let module_path = path_string(self.path);
        let module_id = SymbolId::new(format!("{module_path}::module"));
        let complete = !root.has_error();
        let documentation = self
            .documentation_enabled
            .then(|| documentation_for_body(root, complete, self.source));
        self.symbols.push(Symbol {
            id: module_id.clone(),
            parent_id: None,
            kind: SymbolKind::Module,
            direct_declaration: true,
            name: module_path.clone(),
            qualified_name: module_path,
            span: source_span(root),
            body_span: Some(source_span(root)),
            name_span: None,
            completeness: if !complete {
                SymbolCompleteness::Partial
            } else {
                SymbolCompleteness::Complete
            },
            modifiers: BTreeSet::new(),
            parameters: Vec::new(),
            decorators: Vec::new(),
            documentation,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        self.visit_children(root, Some(module_id), None, 0);
    }

    fn visit_children(
        &mut self,
        node: Node<'_>,
        parent_id: Option<SymbolId>,
        callable_id: Option<SymbolId>,
        depth: usize,
    ) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit_node(child, parent_id.clone(), callable_id.clone(), depth);
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
            "decorated_definition" => {
                let decorator_nodes = node
                    .named_children(&mut node.walk())
                    .filter(|child| child.kind() == "decorator")
                    .collect::<Vec<_>>();
                let decorators = decorator_nodes
                    .iter()
                    .copied()
                    .map(|child| Decorator {
                        expression: decorator_expression(child, self.source),
                        span: source_span(child),
                    })
                    .collect::<Vec<_>>();
                for decorator in decorator_nodes {
                    self.visit_children(decorator, parent_id.clone(), callable_id.clone(), depth);
                }
                let definition = node.named_children(&mut node.walk()).find(|child| {
                    matches!(child.kind(), "function_definition" | "class_definition")
                });
                if let Some(definition) = definition {
                    self.process_definition(
                        definition,
                        parent_id,
                        callable_id,
                        depth,
                        Some(node),
                        decorators,
                    );
                }
            }
            "function_definition" | "class_definition" => {
                self.process_definition(node, parent_id, callable_id, depth, None, Vec::new());
            }
            "lambda" | "lambda_within_for_in_clause" => {
                self.process_lambda(node, parent_id, callable_id);
            }
            "import_statement" | "import_from_statement" | "future_import_statement" => {
                self.extract_import(node, parent_id, callable_id);
            }
            "call" => {
                self.extract_call(node, callable_id.clone().or_else(|| parent_id.clone()));
                self.visit_children(node, parent_id, callable_id, depth);
            }
            _ => {
                if let Some(kind) = decision_kind(node, self.source) {
                    if let Some(callable_id) = &callable_id {
                        if let Some(symbol) = self.symbol_mut(callable_id) {
                            symbol.decision_events.push(DecisionEvent {
                                kind,
                                span: source_span(node),
                            });
                        }
                    }
                }
                let event_kind = nesting_kind(node.kind());
                let next_depth = if let Some(kind) = event_kind {
                    if let Some(callable_id) = &callable_id {
                        if let Some(symbol) = self.symbol_mut(callable_id) {
                            symbol.nesting_events.push(NestingEvent {
                                kind,
                                depth: depth + 1,
                                span: source_span(node),
                            });
                        }
                    }
                    depth + 1
                } else {
                    depth
                };
                self.visit_children(node, parent_id, callable_id, next_depth);
            }
        }
    }

    fn process_definition(
        &mut self,
        node: Node<'_>,
        parent_id: Option<SymbolId>,
        callable_id: Option<SymbolId>,
        depth: usize,
        wrapper: Option<Node<'_>>,
        decorators: Vec<Decorator>,
    ) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = node_text(name_node, self.source);
        let parent_kind = parent_id
            .as_ref()
            .and_then(|id| self.symbol(id).map(|symbol| symbol.kind));
        let kind = match node.kind() {
            "class_definition" => SymbolKind::Class,
            "function_definition" if parent_kind == Some(SymbolKind::Class) => SymbolKind::Method,
            _ => SymbolKind::Function,
        };
        let parent_qualified = parent_id
            .as_ref()
            .and_then(|id| self.symbol(id))
            .filter(|symbol| symbol.kind != SymbolKind::Module)
            .map(|symbol| symbol.qualified_name.clone());
        let qualified_name = match parent_qualified {
            Some(parent) if !parent.is_empty() => format!("{parent}.{name}"),
            _ => name.clone(),
        };
        let base_id = format!(
            "{}::{}:{qualified_name}",
            path_string(self.path),
            kind.as_str()
        );
        let ordinal = self.id_counts.entry(base_id.clone()).or_insert(0);
        *ordinal += 1;
        let id = SymbolId::new(format!("{base_id}#{}", *ordinal));
        let declaration = wrapper.unwrap_or(node);
        let span = source_span(declaration);
        let direct_declaration = is_direct_declaration(declaration);
        let body_span = node.child_by_field_name("body").map(source_span);
        let complete = !node.has_error() && !span_has_error(node);
        let mut modifiers = BTreeSet::new();
        if node.kind() == "function_definition"
            && self
                .source
                .get(node.start_byte()..name_node.start_byte())
                .is_some_and(|bytes| node_text_bytes(bytes).trim_start().starts_with("async"))
        {
            modifiers.insert(SymbolModifier::Async);
        }
        for decorator in &decorators {
            let expression = decorator.expression.trim_start();
            if expression.starts_with("staticmethod") {
                modifiers.insert(SymbolModifier::Static);
            }
            if expression.starts_with("classmethod") {
                modifiers.insert(SymbolModifier::ClassMethod);
            }
        }
        let parameter_node = node.child_by_field_name("parameters");
        let parameters = parameter_node
            .map(|parameters| parse_parameters(parameters, self.source))
            .unwrap_or_default();
        if let Some(parameters) = parameter_node {
            self.visit_children(parameters, parent_id.clone(), callable_id, depth);
        }
        let documentation = self.documentation_enabled.then(|| {
            node.child_by_field_name("body").map_or_else(
                || SymbolDocumentation {
                    status: DocumentationStatus::Unavailable,
                    span: None,
                    reason: Some("definition body is unavailable".to_owned()),
                },
                |body| documentation_for_body(body, complete, self.source),
            )
        });
        self.symbols.push(Symbol {
            id: id.clone(),
            parent_id: parent_id.clone(),
            kind,
            direct_declaration,
            name,
            qualified_name,
            span,
            body_span,
            name_span: Some(source_span(name_node)),
            completeness: if complete {
                SymbolCompleteness::Complete
            } else {
                SymbolCompleteness::Partial
            },
            modifiers,
            parameters,
            decorators,
            documentation,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });

        if let Some(body) = node.child_by_field_name("body") {
            let next_callable = if matches!(kind, SymbolKind::Function | SymbolKind::Method) {
                Some(id.clone())
            } else {
                None
            };
            self.visit_children(body, Some(id), next_callable, 0);
        }
    }

    fn process_lambda(
        &mut self,
        node: Node<'_>,
        parent_id: Option<SymbolId>,
        _callable_id: Option<SymbolId>,
    ) {
        let parent_qualified = parent_id
            .as_ref()
            .and_then(|id| self.symbol(id))
            .map(|symbol| symbol.qualified_name.clone())
            .unwrap_or_default();
        let position = node.start_position();
        let lambda_name = format!("<lambda>@{}:{}", position.row + 1, position.column + 1);
        let qualified_name = if parent_qualified.is_empty() {
            lambda_name.clone()
        } else {
            format!("{parent_qualified}.{lambda_name}")
        };
        let base_id = format!("{}::lambda:{qualified_name}", path_string(self.path));
        let ordinal = self.id_counts.entry(base_id.clone()).or_insert(0);
        *ordinal += 1;
        let id = SymbolId::new(format!("{base_id}#{}", *ordinal));
        let span = source_span(node);
        let body_span = node.child_by_field_name("body").map(source_span);
        let complete = !node.has_error() && !span_has_error(node);
        let parameters = node
            .child_by_field_name("parameters")
            .map(|parameters| parse_parameters(parameters, self.source))
            .unwrap_or_default();
        self.symbols.push(Symbol {
            id: id.clone(),
            parent_id,
            kind: SymbolKind::Lambda,
            direct_declaration: false,
            name: "<lambda>".to_owned(),
            qualified_name,
            span,
            body_span,
            name_span: None,
            completeness: if complete {
                SymbolCompleteness::Complete
            } else {
                SymbolCompleteness::Partial
            },
            modifiers: BTreeSet::new(),
            parameters,
            decorators: Vec::new(),
            documentation: None,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_node(body, Some(id.clone()), Some(id), 0);
        }
    }

    fn extract_import(
        &mut self,
        node: Node<'_>,
        parent_id: Option<SymbolId>,
        callable_id: Option<SymbolId>,
    ) {
        match node.kind() {
            "import_statement" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "aliased_import" {
                        let name = child
                            .child_by_field_name("name")
                            .map(|node| node_text(node, self.source));
                        let alias = child
                            .child_by_field_name("alias")
                            .map(|node| node_text(node, self.source));
                        self.push_dependency(
                            node,
                            name,
                            None,
                            alias,
                            0,
                            false,
                            parent_id.clone(),
                            callable_id.clone(),
                        );
                    } else if child.kind() == "dotted_name" {
                        self.push_dependency(
                            child,
                            Some(node_text(child, self.source)),
                            None,
                            None,
                            0,
                            false,
                            parent_id.clone(),
                            callable_id.clone(),
                        );
                    }
                }
            }
            "future_import_statement" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "aliased_import" {
                        let imported_name = child
                            .child_by_field_name("name")
                            .map(|node| node_text(node, self.source));
                        let alias = child
                            .child_by_field_name("alias")
                            .map(|node| node_text(node, self.source));
                        self.push_dependency(
                            child,
                            Some("__future__".to_owned()),
                            imported_name,
                            alias,
                            0,
                            false,
                            parent_id.clone(),
                            callable_id.clone(),
                        );
                    } else if child.kind() == "dotted_name" {
                        self.push_dependency(
                            child,
                            Some("__future__".to_owned()),
                            Some(node_text(child, self.source)),
                            None,
                            0,
                            false,
                            parent_id.clone(),
                            callable_id.clone(),
                        );
                    }
                }
            }
            "import_from_statement" => {
                let module_node = node.child_by_field_name("module_name");
                let (module, relative_level) = module_node
                    .map(|module_node| relative_module(module_node, self.source))
                    .unwrap_or((None, 0));
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if module_node.is_some_and(|module_node| {
                        module_node.start_byte() == child.start_byte()
                            && module_node.end_byte() == child.end_byte()
                    }) {
                        continue;
                    }
                    if child.kind() == "wildcard_import" {
                        self.push_dependency(
                            child,
                            module.clone(),
                            None,
                            None,
                            relative_level,
                            true,
                            parent_id.clone(),
                            callable_id.clone(),
                        );
                    } else if child.kind() == "aliased_import" {
                        let imported_name = child
                            .child_by_field_name("name")
                            .map(|node| node_text(node, self.source));
                        let alias = child
                            .child_by_field_name("alias")
                            .map(|node| node_text(node, self.source));
                        self.push_dependency(
                            child,
                            module.clone(),
                            imported_name,
                            alias,
                            relative_level,
                            false,
                            parent_id.clone(),
                            callable_id.clone(),
                        );
                    } else if child.kind() == "dotted_name" {
                        self.push_dependency(
                            child,
                            module.clone(),
                            Some(node_text(child, self.source)),
                            None,
                            relative_level,
                            false,
                            parent_id.clone(),
                            callable_id.clone(),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_dependency(
        &mut self,
        node: Node<'_>,
        module: Option<String>,
        imported_name: Option<String>,
        alias: Option<String>,
        relative_level: usize,
        wildcard: bool,
        parent_id: Option<SymbolId>,
        enclosing_symbol: Option<SymbolId>,
    ) {
        let context = self.import_context(node, parent_id.as_ref(), enclosing_symbol.as_ref());
        self.dependencies
            .push(DependencyReference::Import(ImportReference {
                module,
                imported_name,
                alias,
                relative_level,
                wildcard,
                resolution: ResolutionLevel::Syntactic,
                enclosing_symbol,
                context,
                span: source_span(node),
            }));
    }

    fn import_context(
        &self,
        node: Node<'_>,
        parent_id: Option<&SymbolId>,
        callable_id: Option<&SymbolId>,
    ) -> ImportContext {
        let scope = if callable_id.is_some() {
            ImportScope::Callable
        } else if parent_id
            .and_then(|id| self.symbol(id))
            .is_some_and(|symbol| symbol.kind == SymbolKind::Class)
        {
            ImportScope::Class
        } else {
            ImportScope::Module
        };
        let usage = if self.in_type_checking_branch(node) {
            ImportUsage::TypeCheckingOnly
        } else {
            ImportUsage::Runtime
        };
        let optional = self.in_optional_import_guard(node);
        ImportContext {
            scope,
            usage,
            requirement: if optional {
                ImportRequirement::Optional
            } else {
                ImportRequirement::Required
            },
            conditional: optional || is_conditionally_executed(node),
        }
    }

    fn extract_call(&mut self, node: Node<'_>, enclosing_symbol: Option<SymbolId>) {
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        let arguments = node.child_by_field_name("arguments");
        let mut shape = CallArgumentShape {
            positional: 0,
            keywords: Vec::new(),
            has_star_args: false,
            has_star_kwargs: false,
        };
        if let Some(arguments) = arguments {
            if arguments.kind() == "generator_expression" {
                shape.positional = 1;
            } else {
                let mut cursor = arguments.walk();
                for argument in arguments.named_children(&mut cursor) {
                    match argument.kind() {
                        "keyword_argument" => {
                            if let Some(name) = argument.child_by_field_name("name") {
                                shape.keywords.push(node_text(name, self.source));
                            }
                        }
                        "list_splat" => shape.has_star_args = true,
                        "dictionary_splat" => shape.has_star_kwargs = true,
                        _ => shape.positional += 1,
                    }
                }
            }
        }
        shape.keywords.sort();
        self.calls.push(CallReference {
            callee: node_text(function, self.source),
            components: call_components(function, self.source).unwrap_or_default(),
            enclosing_symbol,
            arguments: shape,
            span: source_span(node),
            syntax_complete: !node.has_error(),
        });
    }

    fn in_type_checking_branch(&self, node: Node<'_>) -> bool {
        let mut descendant = node;
        let mut parent = node.parent();
        while let Some(ancestor) = parent {
            if matches!(ancestor.kind(), "if_statement" | "elif_clause")
                && ancestor
                    .child_by_field_name("consequence")
                    .is_some_and(|branch| contains_node(branch, descendant))
                && ancestor
                    .child_by_field_name("condition")
                    .is_some_and(|condition| self.is_type_checking_condition(condition))
            {
                return true;
            }
            if is_scope_boundary(ancestor.kind()) {
                break;
            }
            descendant = ancestor;
            parent = ancestor.parent();
        }
        false
    }

    fn is_type_checking_condition(&self, condition: Node<'_>) -> bool {
        let text = trim_parentheses(node_text(condition, self.source));
        self.type_checking_names.contains(&text)
            || self
                .typing_aliases
                .iter()
                .any(|alias| text == format!("{alias}.TYPE_CHECKING"))
    }

    fn in_optional_import_guard(&self, node: Node<'_>) -> bool {
        let mut descendant = node;
        let mut parent = node.parent();
        while let Some(ancestor) = parent {
            if ancestor.kind() == "try_statement"
                && ancestor
                    .child_by_field_name("body")
                    .is_some_and(|body| contains_node(body, descendant))
                && try_catches_import_error(ancestor, self.source)
            {
                return true;
            }
            if is_scope_boundary(ancestor.kind()) {
                break;
            }
            descendant = ancestor;
            parent = ancestor.parent();
        }
        false
    }

    fn finish_measurements(&mut self) {
        for symbol in &mut self.symbols {
            if !matches!(
                symbol.kind,
                SymbolKind::Function | SymbolKind::Method | SymbolKind::Lambda
            ) {
                continue;
            }
            if symbol.completeness == SymbolCompleteness::Partial {
                for (id, unit) in [
                    ("function-declaration-physical-lines", "lines"),
                    ("function-body-physical-lines", "lines"),
                    ("declared-parameter-count", "parameters"),
                    ("caller-parameter-count", "parameters"),
                    ("python-max-control-flow-nesting", "levels"),
                    (PYTHON_CYCLOMATIC_COMPLEXITY, "score"),
                ] {
                    symbol.measurements.push(Measurement {
                        id: id.to_owned(),
                        definition_version: format!("{id}-v1"),
                        unit: unit.to_owned(),
                        status: MeasurementStatus::Unavailable,
                        value: None,
                        reason: Some("symbol contains parser recovery nodes".to_owned()),
                    });
                }
                continue;
            }
            let declaration_lines = line_count(symbol.span);
            let body_lines = symbol
                .body_span
                .map(line_count)
                .unwrap_or(declaration_lines);
            let declared_count = symbol.parameters.len() as u64;
            let caller_count = if symbol.kind == SymbolKind::Method
                && !symbol.modifiers.contains(&SymbolModifier::Static)
                && declared_count > 0
            {
                declared_count - 1
            } else {
                declared_count
            };
            let max_nesting = symbol
                .nesting_events
                .iter()
                .map(|event| event.depth as u64)
                .max()
                .unwrap_or(0);
            for (id, unit, value) in [
                (
                    "function-declaration-physical-lines",
                    "lines",
                    declaration_lines,
                ),
                ("function-body-physical-lines", "lines", body_lines),
                ("declared-parameter-count", "parameters", declared_count),
                ("caller-parameter-count", "parameters", caller_count),
                ("python-max-control-flow-nesting", "levels", max_nesting),
                (
                    PYTHON_CYCLOMATIC_COMPLEXITY,
                    "score",
                    1 + symbol.decision_events.len() as u64,
                ),
            ] {
                symbol.measurements.push(Measurement {
                    id: id.to_owned(),
                    definition_version: format!("{id}-v1"),
                    unit: unit.to_owned(),
                    status: MeasurementStatus::Measured,
                    value: Some(value),
                    reason: None,
                });
            }
        }
        self.symbols.sort_by(|left, right| {
            left.span
                .start_byte
                .cmp(&right.span.start_byte)
                .then_with(|| left.span.end_byte.cmp(&right.span.end_byte))
                .then_with(|| left.id.cmp(&right.id))
        });
        for symbol in &mut self.symbols {
            symbol.decision_events.sort_by(|left, right| {
                left.span
                    .start_byte
                    .cmp(&right.span.start_byte)
                    .then_with(|| left.span.end_byte.cmp(&right.span.end_byte))
                    .then_with(|| left.kind.cmp(&right.kind))
            });
        }
        self.dependencies.sort_by(|left, right| {
            left.span()
                .start_byte
                .cmp(&right.span().start_byte)
                .then_with(|| left.span().end_byte.cmp(&right.span().end_byte))
        });
        self.calls.sort_by(|left, right| {
            left.span
                .start_byte
                .cmp(&right.span.start_byte)
                .then_with(|| left.span.end_byte.cmp(&right.span.end_byte))
        });
    }

    fn symbol(&self, id: &SymbolId) -> Option<&Symbol> {
        self.symbols.iter().find(|symbol| &symbol.id == id)
    }

    fn symbol_mut(&mut self, id: &SymbolId) -> Option<&mut Symbol> {
        self.symbols.iter_mut().find(|symbol| &symbol.id == id)
    }
}

fn is_direct_declaration(declaration: Node<'_>) -> bool {
    let Some(parent) = declaration.parent() else {
        return false;
    };
    if parent.kind() == "module" {
        return true;
    }
    matches!(parent.kind(), "block" | "suite")
        && parent
            .parent()
            .is_some_and(|owner| matches!(owner.kind(), "class_definition" | "function_definition"))
}

fn documentation_for_body(body: Node<'_>, complete: bool, source: &[u8]) -> SymbolDocumentation {
    if !complete {
        return SymbolDocumentation {
            status: DocumentationStatus::Unavailable,
            span: None,
            reason: Some("symbol contains parser recovery nodes".to_owned()),
        };
    }

    let mut cursor = body.walk();
    let statement = body
        .named_children(&mut cursor)
        .find(|child| child.kind() != "comment");
    let Some(statement) = statement else {
        return SymbolDocumentation {
            status: DocumentationStatus::Missing,
            span: None,
            reason: None,
        };
    };
    if statement.kind() != "expression_statement" {
        return SymbolDocumentation {
            status: DocumentationStatus::Missing,
            span: None,
            reason: None,
        };
    }
    let mut statement_cursor = statement.walk();
    let Some(expression) = statement.named_children(&mut statement_cursor).next() else {
        return SymbolDocumentation {
            status: DocumentationStatus::Missing,
            span: None,
            reason: None,
        };
    };
    let Some(has_content) = docstring_content(expression, source) else {
        return SymbolDocumentation {
            status: DocumentationStatus::Missing,
            span: None,
            reason: None,
        };
    };

    SymbolDocumentation {
        status: if has_content {
            DocumentationStatus::Documented
        } else {
            DocumentationStatus::Missing
        },
        span: Some(source_span(expression)),
        reason: None,
    }
}

fn docstring_content(node: Node<'_>, source: &[u8]) -> Option<bool> {
    match node.kind() {
        "string" => plain_string_content(node, source),
        "concatenated_string" => {
            let mut found = false;
            let mut has_content = false;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "string" {
                    return None;
                }
                found = true;
                has_content |= plain_string_content(child, source)?;
            }
            found.then_some(has_content)
        }
        "parenthesized_expression" => {
            let mut cursor = node.walk();
            let mut children = node.named_children(&mut cursor);
            let child = children.next()?;
            if children.next().is_some() {
                None
            } else {
                docstring_content(child, source)
            }
        }
        _ => None,
    }
}

fn plain_string_content(node: Node<'_>, source: &[u8]) -> Option<bool> {
    let mut cursor = node.walk();
    let children = node.named_children(&mut cursor).collect::<Vec<_>>();
    if children.iter().any(|child| child.kind() == "interpolation") {
        return None;
    }
    let start = children
        .iter()
        .find(|child| child.kind() == "string_start")?;
    let prefix = node_text(*start, source)
        .chars()
        .take_while(|character| *character != '\'' && *character != '"')
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if prefix
        .chars()
        .any(|character| matches!(character, 'b' | 'f' | 't'))
    {
        return None;
    }
    Some(
        children
            .iter()
            .filter(|child| child.kind() == "string_content")
            .any(|child| !node_text(*child, source).trim().is_empty()),
    )
}

fn call_components(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" => Some(vec![node_text(node, source)]),
        "attribute" => {
            let mut components = call_components(node.child_by_field_name("object")?, source)?;
            components.push(node_text(node.child_by_field_name("attribute")?, source));
            Some(components)
        }
        _ => None,
    }
}

fn collect_type_checking_aliases(
    node: Node<'_>,
    source: &[u8],
    typing_aliases: &mut BTreeSet<String>,
    type_checking_names: &mut BTreeSet<String>,
) {
    if node.kind() == "import_statement" {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "aliased_import" {
                let name = child
                    .child_by_field_name("name")
                    .map(|name| node_text(name, source));
                if name.as_deref() == Some("typing") {
                    typing_aliases.insert(
                        child
                            .child_by_field_name("alias")
                            .map(|alias| node_text(alias, source))
                            .unwrap_or_else(|| "typing".to_owned()),
                    );
                }
            } else if node_text(child, source) == "typing" {
                typing_aliases.insert("typing".to_owned());
            }
        }
    } else if node.kind() == "import_from_statement"
        && node
            .child_by_field_name("module_name")
            .is_some_and(|module| node_text(module, source) == "typing")
    {
        let module = node.child_by_field_name("module_name");
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if module.is_some_and(|module| {
                module.start_byte() == child.start_byte() && module.end_byte() == child.end_byte()
            }) {
                continue;
            }
            let (name, alias) = if child.kind() == "aliased_import" {
                (
                    child
                        .child_by_field_name("name")
                        .map(|name| node_text(name, source)),
                    child
                        .child_by_field_name("alias")
                        .map(|alias| node_text(alias, source)),
                )
            } else {
                (Some(node_text(child, source)), None)
            };
            if name.as_deref() == Some("TYPE_CHECKING") {
                type_checking_names.insert(alias.unwrap_or_else(|| "TYPE_CHECKING".to_owned()));
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_type_checking_aliases(child, source, typing_aliases, type_checking_names);
    }
}

fn contains_node(container: Node<'_>, descendant: Node<'_>) -> bool {
    container.start_byte() <= descendant.start_byte()
        && container.end_byte() >= descendant.end_byte()
}

fn is_scope_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition" | "class_definition" | "lambda" | "lambda_within_for_in_clause"
    )
}

fn trim_parentheses(mut value: String) -> String {
    loop {
        let trimmed = value.trim();
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            value = trimmed[1..trimmed.len() - 1].to_owned();
        } else {
            return trimmed.to_owned();
        }
    }
}

fn try_catches_import_error(node: Node<'_>, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "except_clause")
        .filter_map(|handler| handler.child_by_field_name("value"))
        .any(|value| contains_import_error_name(value, source))
}

fn contains_import_error_name(node: Node<'_>, source: &[u8]) -> bool {
    if matches!(node.kind(), "identifier" | "dotted_name")
        && matches!(
            node_text(node, source).as_str(),
            "ImportError" | "ModuleNotFoundError"
        )
    {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| contains_import_error_name(child, source))
}

fn is_conditionally_executed(node: Node<'_>) -> bool {
    let mut descendant = node;
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if matches!(
            ancestor.kind(),
            "if_statement"
                | "elif_clause"
                | "else_clause"
                | "for_statement"
                | "while_statement"
                | "case_clause"
                | "except_clause"
        ) {
            return true;
        }
        if ancestor.kind() == "try_statement"
            && !ancestor
                .child_by_field_name("body")
                .is_some_and(|body| contains_node(body, descendant))
        {
            return true;
        }
        if is_scope_boundary(ancestor.kind()) {
            break;
        }
        descendant = ancestor;
        parent = ancestor.parent();
    }
    false
}

fn parse_parameters(node: Node<'_>, source: &[u8]) -> Vec<Parameter> {
    let mut parameters: Vec<Parameter> = Vec::new();
    let mut mode = ParameterKind::PositionalOrKeyword;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "positional_separator" => {
                for parameter in &mut parameters {
                    if parameter.kind == ParameterKind::PositionalOrKeyword {
                        parameter.kind = ParameterKind::PositionalOnly;
                    }
                }
                mode = ParameterKind::PositionalOrKeyword;
            }
            "keyword_separator" => mode = ParameterKind::KeywordOnly,
            "list_splat_pattern" => {
                parameters.push(parameter_from_node(
                    child,
                    source,
                    ParameterKind::VariadicPositional,
                    false,
                    false,
                ));
                mode = ParameterKind::KeywordOnly;
            }
            "dictionary_splat_pattern" => parameters.push(parameter_from_node(
                child,
                source,
                ParameterKind::VariadicKeyword,
                false,
                false,
            )),
            "identifier"
            | "default_parameter"
            | "typed_parameter"
            | "typed_default_parameter"
            | "tuple_pattern" => {
                parameters.push(parameter_from_node(
                    child,
                    source,
                    mode,
                    matches!(
                        child.kind(),
                        "default_parameter" | "typed_default_parameter"
                    ),
                    matches!(child.kind(), "typed_parameter" | "typed_default_parameter"),
                ));
            }
            _ => {}
        }
    }
    parameters
}

fn parameter_from_node(
    node: Node<'_>,
    source: &[u8],
    kind: ParameterKind,
    has_default: bool,
    has_annotation: bool,
) -> Parameter {
    let name_node = node
        .child_by_field_name("name")
        .or_else(|| node.named_child(0));
    Parameter {
        name: name_node
            .map(|name_node| node_text(name_node, source))
            .unwrap_or_else(|| node_text(node, source)),
        kind,
        span: source_span(node),
        has_default,
        has_annotation,
    }
}

fn relative_module(node: Node<'_>, source: &[u8]) -> (Option<String>, usize) {
    if node.kind() != "relative_import" {
        return (Some(node_text(node, source)), 0);
    }
    let text = node_text(node, source);
    let level = text
        .chars()
        .take_while(|character| *character == '.')
        .count();
    let module = text[level..].trim().to_owned();
    ((!module.is_empty()).then_some(module), level)
}

fn decorator_expression(node: Node<'_>, source: &[u8]) -> String {
    node_text(node, source)
        .trim()
        .trim_start_matches('@')
        .trim()
        .to_owned()
}

fn nesting_kind(kind: &str) -> Option<NestingEventKind> {
    match kind {
        "if_statement" => Some(NestingEventKind::Conditional),
        "for_statement" | "while_statement" => Some(NestingEventKind::Loop),
        "try_statement" => Some(NestingEventKind::ExceptionHandling),
        "with_statement" => Some(NestingEventKind::ContextManager),
        "match_statement" => Some(NestingEventKind::Match),
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => Some(NestingEventKind::Comprehension),
        _ => None,
    }
}

fn decision_kind(node: Node<'_>, source: &[u8]) -> Option<DecisionEventKind> {
    match node.kind() {
        "if_statement" | "elif_clause" => Some(DecisionEventKind::Conditional),
        "for_statement" | "while_statement" => Some(DecisionEventKind::Loop),
        "for_in_clause" => Some(DecisionEventKind::ComprehensionLoop),
        "except_clause" => Some(DecisionEventKind::ExceptionHandler),
        "case_clause" if !case_is_irrefutable(node, source) => {
            Some(DecisionEventKind::PatternBranch)
        }
        "if_clause"
            if node
                .parent()
                .is_some_and(|parent| parent.kind() == "case_clause") =>
        {
            Some(DecisionEventKind::MatchGuard)
        }
        "if_clause" => Some(DecisionEventKind::ComprehensionFilter),
        "boolean_operator" => Some(DecisionEventKind::BooleanShortCircuit),
        "conditional_expression" => Some(DecisionEventKind::ConditionalExpression),
        "assert_statement" => Some(DecisionEventKind::Assertion),
        _ => None,
    }
}

fn case_is_irrefutable(node: Node<'_>, source: &[u8]) -> bool {
    let guard = node.child_by_field_name("guard");
    let patterns = node
        .named_children(&mut node.walk())
        .filter(|child| {
            Some(child.start_byte()) != guard.map(|guard| guard.start_byte())
                && child.kind() != "block"
                && child.kind() != "suite"
        })
        .collect::<Vec<_>>();
    patterns.len() == 1 && pattern_is_irrefutable(patterns[0], source)
}

fn pattern_is_irrefutable(node: Node<'_>, source: &[u8]) -> bool {
    match node.kind() {
        "case_pattern" | "as_pattern" => node
            .named_child(0)
            .is_some_and(|child| pattern_is_irrefutable(child, source)),
        "union_pattern" => node
            .named_children(&mut node.walk())
            .any(|child| pattern_is_irrefutable(child, source)),
        "identifier" | "dotted_name" => {
            let text = node_text(node, source);
            !text.contains('.') && text != "_"
        }
        _ => node_text(node, source).trim() == "_",
    }
}

fn collect_diagnostics(
    node: Node<'_>,
    inside_error: bool,
    diagnostics: &mut Vec<AnalysisDiagnostic>,
) {
    let is_error = node.is_error();
    if is_error && !inside_error {
        diagnostics.push(AnalysisDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "parse-error".to_owned(),
            message: "the parser could not interpret this Python syntax".to_owned(),
            span: Some(source_span(node)),
        });
    } else if node.is_missing() {
        diagnostics.push(AnalysisDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "missing-syntax".to_owned(),
            message: format!("the parser expected {}", node.kind()),
            span: Some(source_span(node)),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_diagnostics(child, inside_error || is_error, diagnostics);
    }
}

fn source_span(node: Node<'_>) -> SourceSpan {
    let start = node.start_position();
    let end = node.end_position();
    SourceSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start: SourcePosition {
            line: start.row + 1,
            column: start.column + 1,
        },
        end: SourcePosition {
            line: end.row + 1,
            column: end.column + 1,
        },
    }
}

fn span_has_error(node: Node<'_>) -> bool {
    node.has_error()
}

fn line_count(span: SourceSpan) -> u64 {
    (span.end.line.saturating_sub(span.start.line) + 1) as u64
}

fn node_text(node: Node<'_>, source: &[u8]) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .map(node_text_bytes)
        .unwrap_or_default()
}

fn node_text_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn diagnostic_key(diagnostic: &AnalysisDiagnostic) -> (usize, usize, &str, &str) {
    let (start, end) = diagnostic
        .span
        .map(|span| (span.start_byte, span.end_byte))
        .unwrap_or((usize::MAX, usize::MAX));
    (start, end, &diagnostic.code, &diagnostic.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraide_core::ExplicitExportStatus;

    fn analyze(source: &[u8]) -> FileAnalysis {
        let mut analyzer = PythonAnalyzer::new().expect("Python grammar should initialize");
        analyzer.analyze(AnalysisInput {
            path: Path::new("src/example.py"),
            source,
        })
    }

    #[test]
    fn extracts_complete_module_explicit_exports_in_runtime_order() {
        let result = analyze(include_bytes!(
            "../tests/fixtures/explicit_exports_complete.py"
        ));
        let exports = result
            .facts
            .explicit_exports
            .expect("Python should report explicit export applicability");

        assert_eq!(exports.status, ExplicitExportStatus::Complete);
        assert_eq!(
            exports
                .names
                .iter()
                .map(|name| name.name.as_str())
                .collect::<Vec<_>>(),
            ["Client", "request", "Response", "Session", "get", "post"]
        );
        assert_eq!(
            exports
                .declaration_span
                .expect("declaration span")
                .start
                .line,
            1
        );
        assert!(exports.names.iter().all(|name| name.span.start.line > 0));
        assert_eq!(exports.reason, None);
    }

    #[test]
    fn reports_known_names_when_explicit_exports_are_partial() {
        let result = analyze(include_bytes!(
            "../tests/fixtures/explicit_exports_partial.py"
        ));
        let exports = result
            .facts
            .explicit_exports
            .expect("Python should report explicit export applicability");

        assert_eq!(exports.status, ExplicitExportStatus::Partial);
        assert_eq!(exports.names[0].name, "known");
        let reason = exports.reason.expect("partial extraction reason");
        assert!(reason.contains("not a fully static list or tuple"));
        assert!(reason.contains("conditional or nested"));
    }

    #[test]
    fn distinguishes_unavailable_and_not_declared_explicit_exports() {
        let unavailable = analyze(include_bytes!(
            "../tests/fixtures/explicit_exports_unavailable.py"
        ));
        let unavailable = unavailable
            .facts
            .explicit_exports
            .expect("Python should report explicit export applicability");
        assert_eq!(unavailable.status, ExplicitExportStatus::Unavailable);
        assert!(unavailable.names.is_empty());
        assert!(unavailable.reason.is_some());

        let not_declared = analyze(b"def public():\n    pass\n");
        let not_declared = not_declared
            .facts
            .explicit_exports
            .expect("Python should report explicit export applicability");
        assert_eq!(not_declared.status, ExplicitExportStatus::NotDeclared);
        assert!(not_declared.names.is_empty());
        assert_eq!(not_declared.declaration_span, None);
        assert_eq!(not_declared.reason, None);
    }

    #[test]
    fn parser_recovery_prevents_complete_explicit_export_claims() {
        let result = analyze(b"__all__ = ['known']\ndef broken(\n");
        let exports = result
            .facts
            .explicit_exports
            .expect("Python should report explicit export applicability");

        assert_eq!(exports.status, ExplicitExportStatus::Partial);
        assert_eq!(exports.names[0].name, "known");
        assert!(
            exports
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("parser recovery"))
        );
    }

    #[test]
    fn later_static_assignment_resets_uncertain_exports_but_subscript_mutation_does_not() {
        let reset = analyze(b"__all__ = build_exports()\n__all__ = ['known']\n");
        let reset = reset
            .facts
            .explicit_exports
            .expect("Python should report explicit export applicability");
        assert_eq!(reset.status, ExplicitExportStatus::Complete);
        assert_eq!(reset.names[0].name, "known");

        let subscript = analyze(b"__all__ = ['known']\n__all__[0] = dynamic_name\n");
        let subscript = subscript
            .facts
            .explicit_exports
            .expect("Python should report explicit export applicability");
        assert_eq!(subscript.status, ExplicitExportStatus::Partial);
        assert_eq!(subscript.names[0].name, "known");
        assert!(
            subscript
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("subscripted"))
        );
    }

    #[test]
    fn explicit_export_capability_and_definition_are_declared() {
        let analyzer = PythonAnalyzer::new().expect("Python grammar should initialize");

        assert!(
            analyzer
                .descriptor()
                .capabilities
                .contains(&AnalyzerCapability::ExplicitExports)
        );
        assert!(analyzer.descriptor().queries.iter().any(|query| {
            query.name == "explicit-exports" && query.version == "python-explicit-exports-v1"
        }));
    }

    #[test]
    fn extracts_symbols_parameters_decorators_imports_and_metrics() {
        let result = analyze(
            b"from __future__ import annotations\nimport os as operating_system\nfrom .models import User as Account, Team\n\n@classmethod\nasync def load(self, item, /, limit=10, *, strict=False, **options):\n    if strict:\n        for value in item:\n            print(value)\n    return item\n",
        );
        assert_eq!(result.status, FileAnalysisStatus::Successful);
        assert_eq!(result.facts.symbols.len(), 2);
        let function = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Function)
            .expect("function should be extracted");
        assert_eq!(function.parameters.len(), 5);
        assert_eq!(function.parameters[0].kind, ParameterKind::PositionalOnly);
        assert_eq!(
            function.parameters[2].kind,
            ParameterKind::PositionalOrKeyword
        );
        assert_eq!(function.parameters[3].kind, ParameterKind::KeywordOnly);
        assert_eq!(function.parameters[4].kind, ParameterKind::VariadicKeyword);
        assert_eq!(function.decorators[0].expression, "classmethod");
        assert!(function.modifiers.contains(&SymbolModifier::Async));
        assert_eq!(result.facts.dependencies.len(), 4);
        assert!(
            result
                .facts
                .dependencies
                .iter()
                .filter_map(DependencyReference::as_import)
                .any(|dependency| dependency.module.as_deref() == Some("__future__"))
        );
        assert!(function.nesting_events.len() >= 2);
        assert!(function.measurements.iter().any(|measurement| {
            measurement.id == "python-max-control-flow-nesting" && measurement.value == Some(2)
        }));
        assert!(function.measurements.iter().any(|measurement| {
            measurement.id == "declared-parameter-count" && measurement.value == Some(5)
        }));
        assert!(function.measurements.iter().any(|measurement| {
            measurement.id == PYTHON_CYCLOMATIC_COMPLEXITY && measurement.value == Some(3)
        }));
    }

    #[test]
    fn extracts_conventional_nonblank_docstrings_without_source_text() {
        let result = analyze(
            br#"# leading comment
"""Module documentation."""

class Service:
    """Service documentation."""

    def documented(self):
        r"""Method documentation."""

    @property
    async def async_property(self):
        """Async decorated method documentation."""

    def concatenated(self):
        "first " "second"

    def empty(self):
        """   """

    def bytes_only(self):
        b"not a docstring"

    def formatted(self, value):
        f"{value}"

    class Nested:
        """Nested documentation."""

def top_level():
    u"Top-level documentation."

def outer():
    """Outer documentation."""
    def nested():
        """Nested function documentation."""
    return nested

callback = lambda: None
"#,
        );
        assert_eq!(result.status, FileAnalysisStatus::Successful);
        let documentation = |name: &str| {
            result
                .facts
                .symbols
                .iter()
                .find(|symbol| symbol.qualified_name == name)
                .and_then(|symbol| symbol.documentation.as_ref())
                .unwrap_or_else(|| panic!("missing documentation fact for {name}"))
        };

        assert_eq!(
            result.facts.symbols[0]
                .documentation
                .as_ref()
                .map(|documentation| documentation.status),
            Some(DocumentationStatus::Documented)
        );
        for name in [
            "Service",
            "Service.documented",
            "Service.async_property",
            "Service.concatenated",
            "top_level",
            "outer",
            "outer.nested",
        ] {
            assert_eq!(documentation(name).status, DocumentationStatus::Documented);
            assert!(documentation(name).span.is_some());
        }
        assert_eq!(
            documentation("Service.empty").status,
            DocumentationStatus::Missing
        );
        assert!(documentation("Service.empty").span.is_some());
        for name in ["Service.bytes_only", "Service.formatted"] {
            assert_eq!(documentation(name).status, DocumentationStatus::Missing);
            assert!(documentation(name).span.is_none());
        }
        assert!(
            result
                .facts
                .symbols
                .iter()
                .filter(|symbol| symbol.kind == SymbolKind::Lambda)
                .all(|symbol| symbol.documentation.is_none())
        );
    }

    #[test]
    fn distinguishes_direct_definitions_from_conditional_definitions() {
        let result = analyze(
            br#"from typing import TYPE_CHECKING

def direct_function():
    pass

@decorator
class DirectClass:
    def direct_method(self):
        pass

    if TYPE_CHECKING:
        def conditional_method(self):
            pass

if TYPE_CHECKING:
    def type_only():
        pass

if sys.platform == "win32":
    def windows_only():
        pass

try:
    import optional
except ImportError:
    def fallback():
        pass
"#,
        );
        let direct = |name: &str| {
            result
                .facts
                .symbols
                .iter()
                .find(|symbol| symbol.qualified_name == name)
                .unwrap_or_else(|| panic!("missing symbol {name}"))
                .direct_declaration
        };

        for name in [
            "direct_function",
            "DirectClass",
            "DirectClass.direct_method",
        ] {
            assert!(direct(name), "{name} should be direct");
        }
        for name in [
            "DirectClass.conditional_method",
            "type_only",
            "windows_only",
            "fallback",
        ] {
            assert!(!direct(name), "{name} should be conditional");
        }
    }

    #[test]
    fn documentation_can_be_disabled_for_graph_only_analysis() {
        let mut analyzer =
            PythonAnalyzer::without_documentation().expect("Python grammar should initialize");
        let result = analyzer.analyze(AnalysisInput {
            path: Path::new("src/example.py"),
            source: b"\"\"\"module\"\"\"\ndef run():\n    \"\"\"run\"\"\"\n",
        });

        assert!(
            !analyzer
                .descriptor()
                .capabilities
                .contains(&AnalyzerCapability::Documentation)
        );
        assert!(
            result
                .facts
                .symbols
                .iter()
                .all(|symbol| symbol.documentation.is_none())
        );
    }

    #[test]
    fn classifies_orthogonal_import_contexts() {
        let result = analyze(
            br#"import typing as t
from typing import TYPE_CHECKING as TC
import always

class Client:
    import class_dependency

    def send(self):
        import callable_dependency

if t.TYPE_CHECKING:
    import type_dependency
else:
    import runtime_else_dependency

if TC:
    import aliased_type_dependency

try:
    import optional_dependency
except (ImportError, ValueError):
    import handler_dependency

try:
    import required_dependency
except ValueError:
    pass

for item in items:
    import loop_dependency
"#,
        );
        assert_eq!(result.status, FileAnalysisStatus::Successful);
        let dependency = |name: &str| {
            result
                .facts
                .dependencies
                .iter()
                .filter_map(DependencyReference::as_import)
                .find(|dependency| dependency.module.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing dependency {name}"))
        };

        assert_eq!(dependency("always").context, ImportContext::default());
        assert_eq!(
            dependency("class_dependency").context.scope,
            ImportScope::Class
        );
        assert_eq!(
            dependency("callable_dependency").context.scope,
            ImportScope::Callable
        );
        for name in ["type_dependency", "aliased_type_dependency"] {
            assert_eq!(
                dependency(name).context.usage,
                ImportUsage::TypeCheckingOnly
            );
            assert!(dependency(name).context.conditional);
        }
        assert_eq!(
            dependency("runtime_else_dependency").context.usage,
            ImportUsage::Runtime
        );
        assert!(dependency("runtime_else_dependency").context.conditional);
        assert_eq!(
            dependency("optional_dependency").context.requirement,
            ImportRequirement::Optional
        );
        assert!(dependency("optional_dependency").context.conditional);
        assert_eq!(
            dependency("handler_dependency").context.requirement,
            ImportRequirement::Required
        );
        assert!(dependency("handler_dependency").context.conditional);
        assert!(!dependency("required_dependency").context.conditional);
        assert!(dependency("loop_dependency").context.conditional);
    }

    #[test]
    fn extracts_normalized_call_references_and_argument_shapes() {
        let result = analyze(
            b"def run(args, kwargs):\n    helper(1, key=2, *args, **kwargs)\n    service.send()\n",
        );
        assert_eq!(result.facts.calls.len(), 2);
        let helper = &result.facts.calls[0];
        assert_eq!(helper.callee, "helper");
        assert_eq!(helper.components, ["helper"]);
        assert_eq!(helper.arguments.positional, 1);
        assert_eq!(helper.arguments.keywords, ["key"]);
        assert!(helper.arguments.has_star_args);
        assert!(helper.arguments.has_star_kwargs);
        assert!(helper.enclosing_symbol.is_some());
        assert!(helper.syntax_complete);

        assert_eq!(result.facts.calls[1].components, ["service", "send"]);
    }

    #[test]
    fn counts_explicit_python_decisions_and_preserves_event_spans() {
        let result = analyze(
            b"def choose(value, items):\n    if value and value > 0:\n        for item in items:\n            if item.ready:\n                return item\n    elif value is None:\n        assert items\n    return [item for item in items if item.ok]\n",
        );
        let function = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Function)
            .expect("function should be extracted");
        let complexity = function
            .measurements
            .iter()
            .find(|measurement| measurement.id == PYTHON_CYCLOMATIC_COMPLEXITY)
            .and_then(|measurement| measurement.value);
        assert_eq!(complexity, Some(9));
        assert_eq!(function.decision_events.len(), 8);
        assert_eq!(
            function
                .decision_events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                DecisionEventKind::Conditional,
                DecisionEventKind::BooleanShortCircuit,
                DecisionEventKind::Loop,
                DecisionEventKind::Conditional,
                DecisionEventKind::Conditional,
                DecisionEventKind::Assertion,
                DecisionEventKind::ComprehensionLoop,
                DecisionEventKind::ComprehensionFilter,
            ]
        );
        assert!(
            function
                .decision_events
                .iter()
                .all(|event| event.span.start.line > 1)
        );
    }

    #[test]
    fn excludes_implicit_and_unconditional_constructs() {
        let result = analyze(
            b"def controlled(lock, values):\n    with lock:\n        try:\n            for value in values:\n                pass\n            else:\n                pass\n        except ValueError:\n            pass\n        else:\n            pass\n        finally:\n            pass\n",
        );
        let function = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Function)
            .expect("function should be extracted");
        let complexity = function
            .measurements
            .iter()
            .find(|measurement| measurement.id == PYTHON_CYCLOMATIC_COMPLEXITY)
            .and_then(|measurement| measurement.value);
        assert_eq!(complexity, Some(3));
        assert_eq!(
            function
                .decision_events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![DecisionEventKind::Loop, DecisionEventKind::ExceptionHandler]
        );
    }

    #[test]
    fn scores_match_cases_and_lambda_as_separate_callables() {
        let result = analyze(
            b"def outer(value):\n    match value:\n        case 1:\n            return 1\n        case x:\n            return x\n    return lambda item: item if item and item.ready else None\n",
        );
        let outer = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Function)
            .expect("outer function should be extracted");
        let outer_score = outer
            .measurements
            .iter()
            .find(|measurement| measurement.id == PYTHON_CYCLOMATIC_COMPLEXITY)
            .and_then(|measurement| measurement.value);
        assert_eq!(outer_score, Some(2));
        let lambda = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Lambda)
            .expect("lambda should be extracted");
        let lambda_score = lambda
            .measurements
            .iter()
            .find(|measurement| measurement.id == PYTHON_CYCLOMATIC_COMPLEXITY)
            .and_then(|measurement| measurement.value);
        assert_eq!(lambda_score, Some(3));
        assert_eq!(lambda.parent_id.as_ref(), Some(&outer.id));
    }

    #[test]
    fn class_methods_and_nested_functions_have_distinct_owners() {
        let result = analyze(
            b"class Service:\n    @staticmethod\n    def run(value):\n        def local():\n            return value\n        return local()\n",
        );
        let class = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Class)
            .expect("class should be extracted");
        let method = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Method)
            .expect("method should be extracted");
        let local = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.name == "local")
            .expect("nested function should be extracted");
        assert_eq!(method.parent_id.as_ref(), Some(&class.id));
        assert_eq!(local.parent_id.as_ref(), Some(&method.id));
        assert!(method.modifiers.contains(&SymbolModifier::Static));
    }

    #[test]
    fn malformed_files_keep_facts_but_mark_symbols_partial() {
        let result = analyze(b"def broken():\n    if:\n        return 1\n");
        assert_eq!(result.status, FileAnalysisStatus::Partial);
        let function = result
            .facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == SymbolKind::Function)
            .expect("recovered function should be visible");
        assert_eq!(function.completeness, SymbolCompleteness::Partial);
        assert!(
            function
                .measurements
                .iter()
                .all(|measurement| measurement.status == MeasurementStatus::Unavailable)
        );
        assert_eq!(
            function
                .documentation
                .as_ref()
                .map(|documentation| documentation.status),
            Some(DocumentationStatus::Unavailable)
        );
    }

    #[test]
    fn empty_python_is_successful() {
        let result = analyze(b"");
        assert_eq!(result.status, FileAnalysisStatus::Successful);
        assert_eq!(result.facts.symbols.len(), 1);
    }

    #[test]
    fn non_utf8_source_is_reported_without_panicking() {
        let result = analyze(b"value = '\xff'\n");
        assert_eq!(result.status, FileAnalysisStatus::Partial);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "source-not-utf8")
        );
    }
}
