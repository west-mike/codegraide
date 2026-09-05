mod call_flow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use codegraide_core::{
    AnalysisDiagnostic, AnalysisFacts, AnalysisInput, AnalysisLevel, AnalyzerCapability,
    AnalyzerDescriptor, CallArgument, CallArgumentShape, CallForm, CallReference,
    CallableParameter, CallableQualifier, CallableSignature, DecisionEvent, DecisionEventKind,
    DependencyReference, DiagnosticSeverity, FileAnalysis, FileAnalysisStatus, GrammarDescriptor,
    IncludeDelimiter, IncludeReference, LanguageAnalyzer, LanguageId, LanguageModule,
    LanguageModuleKind, Measurement, MeasurementConcept, MeasurementDescriptor, MeasurementStatus,
    ModuleExport, ModuleImport, ModuleImportKind, NestingEvent, NestingEventKind, QueryDescriptor,
    ResolutionLevel, SourcePosition, SourceSpan, Symbol, SymbolCompleteness, SymbolDeclaration,
    SymbolId, SymbolKind, SymbolOccurrenceRole, UsingReference, UsingReferenceKind,
};
use tree_sitter::{Language, Node, Parser, Query};

mod architecture;
mod call_resolution;
mod resolution;

pub use architecture::{
    ARCHITECTURE_SCHEMA_VERSION, ArchitectureError, ArchitectureGroup, apply_architecture_file,
    apply_architecture_to_resolution,
};

pub use call_resolution::{
    CPP_CALL_RESOLUTION_DEFINITION_VERSION, CPP_DECLARATION_LINK_DEFINITION_VERSION,
    CPP_SYMBOL_INDEX_DEFINITION_VERSION, CppCallResolution, resolve_cpp_calls,
};

pub use resolution::{
    CPP_HEADER_RESOLUTION_DEFINITION_VERSION, CppDependencyResolution, CppDependencyResolver,
    CppResolutionError, CppResolutionOptions, CppResolutionSummary, resolve_cpp_dependencies,
};

pub const CPP_CYCLOMATIC_COMPLEXITY: &str = "cpp-cyclomatic-complexity";
pub const CPP_CYCLOMATIC_COMPLEXITY_DEFINITION_VERSION: &str = "cpp-cyclomatic-complexity-v1";
pub const CPP_MAX_CONTROL_FLOW_NESTING: &str = "cpp-max-control-flow-nesting";
pub const CPP_MAX_CONTROL_FLOW_NESTING_DEFINITION_VERSION: &str = "cpp-max-control-flow-nesting-v1";

const ANALYZER_VERSION: &str = "0.4.0";
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
                    AnalyzerCapability::CallReferences,
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
                    QueryDescriptor {
                        name: "calls".to_owned(),
                        version: "cpp-call-references-v1".to_owned(),
                    },
                    QueryDescriptor {
                        name: "modules".to_owned(),
                        version: "cpp-modules-v1".to_owned(),
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
                    "Declarations and blocks whose shape depends on macro expansion may produce partial results; diagnostics identify macro-dependent recovery when it can be recognized from the source."
                        .to_owned(),
                    "Include fragments such as .inl and .inl.hpp files may require their enclosing source context and can produce partial results when analyzed alone."
                        .to_owned(),
                    "Templates are parsed without instantiation, and overloads retain syntax-level identities only."
                        .to_owned(),
                    "Out-of-class qualified definitions are linked to class ownership only when the project symbol index finds one unambiguous owner."
                        .to_owned(),
                    "C++20 module facts come from a comment-aware lexical scanner because the pinned grammar has no module nodes; macro-generated module syntax remains unavailable."
                        .to_owned(),
                    "Files ending in .C, .h, or .H can contain C, C++, or shared C/C++ code; they are parsed with the C++ grammar without claiming that they are C++-only."
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
        classify_recovery_diagnostics(input.source, &mut diagnostics);
        diagnostics.sort_by(diagnostic_order);

        let mut extraction = Extraction::new(input.path, input.source);
        extraction.visit_node(tree.root_node(), None, None, 0);
        extraction.finish_measurements();
        let (modules, module_imports, module_exports) = scan_cpp_modules(input.source);

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
                declarations: extraction.declarations,
                dependencies: extraction.includes,
                calls: extraction.calls,
                call_flows: extraction.call_flows,
                modules,
                module_imports,
                module_exports,
                using_references: extraction.using_references,
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

fn find_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| find_descendant(child, kind))
}

fn callable_signature(
    declaration: Node<'_>,
    declarator: Node<'_>,
    name_node: Node<'_>,
    source: &[u8],
) -> CallableSignature {
    let parameter_list = find_descendant(declarator, "parameter_list");
    let parameters = parameter_list
        .map(|list| {
            let mut cursor = list.walk();
            list.named_children(&mut cursor)
                .filter(|parameter| {
                    matches!(
                        parameter.kind(),
                        "parameter_declaration"
                            | "optional_parameter_declaration"
                            | "variadic_parameter_declaration"
                    )
                })
                .map(|parameter| callable_parameter(parameter, source))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let prefix = source
        .get(declaration.start_byte()..name_node.start_byte())
        .map(node_text_bytes)
        .unwrap_or_default();
    let return_type = normalize_type(
        prefix
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .trim_start_matches("inline ")
            .trim_start_matches("constexpr ")
            .trim_start_matches("consteval ")
            .trim_start_matches("static ")
            .trim(),
    );
    let suffix = parameter_list
        .and_then(|list| source.get(list.end_byte()..declarator.end_byte()))
        .map(node_text_bytes)
        .unwrap_or_default();
    let mut qualifiers = BTreeSet::new();
    for (needle, qualifier) in [
        ("const", CallableQualifier::Const),
        ("volatile", CallableQualifier::Volatile),
        ("noexcept", CallableQualifier::Noexcept),
    ] {
        if suffix
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|word| word == needle)
        {
            qualifiers.insert(qualifier);
        }
    }
    if suffix.contains("&&") {
        qualifiers.insert(CallableQualifier::RvalueReference);
    } else if suffix.contains('&') {
        qualifiers.insert(CallableQualifier::LvalueReference);
    }
    if prefix.split_whitespace().any(|word| word == "static") {
        qualifiers.insert(CallableQualifier::Static);
    }
    let normalized_parameters = parameters
        .iter()
        .map(|parameter| {
            parameter
                .type_spelling
                .clone()
                .unwrap_or_else(|| "?".to_owned())
        })
        .collect::<Vec<_>>()
        .join(",");
    let normalized_qualifiers = qualifiers
        .iter()
        .map(|qualifier| qualifier.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let display = node_text(declaration, source)
        .split('{')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_owned();
    CallableSignature {
        display,
        normalized_key: format!("({normalized_parameters})[{normalized_qualifiers}]"),
        return_type: (!return_type.is_empty()).then_some(return_type),
        parameters,
        qualifiers,
        template_parameter_count: template_parameter_count(declaration),
        virtual_dispatch: prefix
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|word| word == "virtual")
            || suffix.contains("override")
            || suffix.contains("final"),
    }
}

fn callable_parameter(node: Node<'_>, source: &[u8]) -> CallableParameter {
    let text = node_text(node, source);
    let has_default = text.contains('=');
    let variadic = node.kind().contains("variadic") || text.contains("...");
    let name_node = node
        .child_by_field_name("declarator")
        .and_then(declarator_name_node);
    let name = name_node.map(|name| node_text(name, source));
    let without_default = text.split('=').next().unwrap_or_default().trim();
    let type_text = name.as_deref().map_or(without_default, |name| {
        without_default
            .rfind(name)
            .map(|index| without_default[..index].trim())
            .unwrap_or(without_default)
    });
    CallableParameter {
        name,
        type_spelling: (!type_text.is_empty()).then(|| normalize_type(type_text)),
        has_default,
        variadic,
        span: source_span(node),
    }
}

fn normalize_type(value: &str) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = true;
            continue;
        }
        let punctuation = matches!(character, '<' | '>' | ',' | '*' | '&' | '[' | ']' | ':');
        if pending_space
            && !output.is_empty()
            && !punctuation
            && !output.ends_with(['<', '>', ',', '*', '&', '[', ']', ':'])
        {
            output.push(' ');
        }
        if punctuation && output.ends_with(' ') {
            output.pop();
        }
        output.push(character);
        pending_space = false;
    }
    output
}

fn recovered_export_namespace_name(text: &str) -> Option<String> {
    let rest = text.trim_start().strip_prefix("export namespace ")?;
    let name = rest
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '_' || character == ':')
        })
        .next()
        .unwrap_or_default()
        .trim_matches(':');
    (!name.is_empty()).then(|| name.to_owned())
}

fn node_text_bytes(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

fn template_parameter_count(mut node: Node<'_>) -> usize {
    while let Some(parent) = node.parent() {
        if parent.kind() == "template_declaration" {
            return parent
                .child_by_field_name("parameters")
                .map(|parameters| parameters.named_child_count())
                .unwrap_or(0);
        }
        if !matches!(parent.kind(), "declaration" | "field_declaration") {
            break;
        }
        node = parent;
    }
    0
}

fn call_arguments(node: Node<'_>, source: &[u8]) -> Vec<CallArgument> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .map(|argument| {
            let expression = node_text(argument, source);
            CallArgument {
                type_hint: simple_type_hint(argument, &expression, source),
                expression,
                span: source_span(argument),
            }
        })
        .collect()
}

fn simple_type_hint(node: Node<'_>, expression: &str, source: &[u8]) -> Option<String> {
    match node.kind() {
        "string_literal" | "concatenated_string" => Some("string-literal".to_owned()),
        "char_literal" => Some("char".to_owned()),
        "true" | "false" => Some("bool".to_owned()),
        "null" | "nullptr" => Some("nullptr".to_owned()),
        "number_literal" => {
            if expression.contains(['.', 'e', 'E']) {
                Some("floating-literal".to_owned())
            } else {
                Some("integer-literal".to_owned())
            }
        }
        "cast_expression" => node
            .child_by_field_name("type")
            .map(|kind| normalize_type(&node_text(kind, source))),
        _ => None,
    }
}

fn call_form_and_receiver(callee: &str) -> (CallForm, Option<String>) {
    if let Some((receiver, _)) = callee.rsplit_once("->") {
        return (CallForm::PointerMember, Some(receiver.trim().to_owned()));
    }
    if let Some((receiver, _)) = callee.rsplit_once('.') {
        return (CallForm::Member, Some(receiver.trim().to_owned()));
    }
    if callee.contains("::") {
        return (CallForm::Qualified, None);
    }
    (CallForm::Free, None)
}

fn cpp_call_components(callee: &str) -> Vec<String> {
    callee
        .replace("->", "::")
        .replace('.', "::")
        .split("::")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.split('<').next().unwrap_or(part).to_owned())
        .collect()
}

fn scan_cpp_modules(source: &[u8]) -> (Vec<LanguageModule>, Vec<ModuleImport>, Vec<ModuleExport>) {
    let text = String::from_utf8_lossy(source);
    let mut modules = Vec::new();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut byte_offset = 0;
    let mut in_block_comment = false;
    let mut conditional_depth = 0usize;
    let mut brace_depth = 0isize;
    let mut export_block_depth = None;

    for raw_line in text.split_inclusive('\n') {
        let line_without_newline = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let cleaned = strip_cpp_comments(line_without_newline, &mut in_block_comment);
        let trimmed = cleaned.trim();
        let span = span_for_offsets(
            source,
            byte_offset,
            byte_offset + line_without_newline.len(),
        );

        if trimmed.starts_with("#if") {
            conditional_depth += 1;
        } else if trimmed.starts_with("#endif") {
            conditional_depth = conditional_depth.saturating_sub(1);
        }

        if let Some(rest) = trimmed.strip_prefix("export module ") {
            if let Some((name, partition)) = parse_module_name(rest) {
                modules.push(LanguageModule {
                    name,
                    kind: if partition.is_some() {
                        LanguageModuleKind::InterfacePartition
                    } else {
                        LanguageModuleKind::Interface
                    },
                    partition,
                    exported: true,
                    span,
                    complete: conditional_depth == 0,
                });
            }
        } else if let Some(rest) = trimmed.strip_prefix("module ") {
            if !rest.starts_with(':')
                && let Some((name, partition)) = parse_module_name(rest)
            {
                modules.push(LanguageModule {
                    name,
                    kind: if partition.is_some() {
                        LanguageModuleKind::ImplementationPartition
                    } else {
                        LanguageModuleKind::Implementation
                    },
                    partition,
                    exported: false,
                    span,
                    complete: conditional_depth == 0,
                });
            }
        }

        let import = trimmed
            .strip_prefix("export import ")
            .map(|target| (true, target))
            .or_else(|| {
                trimmed
                    .strip_prefix("import ")
                    .map(|target| (false, target))
            });
        if let Some((exported, target)) = import {
            let target = target.trim().trim_end_matches(';').trim();
            if !target.is_empty() {
                let (kind, target) = if target.starts_with('<') && target.ends_with('>') {
                    (
                        ModuleImportKind::HeaderAngle,
                        target[1..target.len() - 1].to_owned(),
                    )
                } else if target.starts_with('"') && target.ends_with('"') {
                    (
                        ModuleImportKind::HeaderQuote,
                        target[1..target.len() - 1].to_owned(),
                    )
                } else if target.starts_with(':') {
                    (ModuleImportKind::Partition, target.to_owned())
                } else {
                    (ModuleImportKind::Named, target.to_owned())
                };
                imports.push(ModuleImport {
                    target,
                    kind,
                    exported,
                    conditional: conditional_depth > 0,
                    span,
                    complete: trimmed.ends_with(';'),
                });
            }
        }

        if trimmed.starts_with("export namespace ") && trimmed.contains('{') {
            export_block_depth = Some(brace_depth + 1);
        }
        let in_export_block = export_block_depth.is_some_and(|depth| brace_depth >= depth);
        let using_text = trimmed.strip_prefix("export using ").or_else(|| {
            in_export_block
                .then_some(trimmed)
                .and_then(|line| line.strip_prefix("using "))
        });
        if let Some(using_text) = using_text {
            let target = using_text.trim_end_matches(';').trim();
            if !target.is_empty() && !target.contains('=') {
                exports.push(ModuleExport {
                    target: target.to_owned(),
                    span,
                    complete: trimmed.ends_with(';') && conditional_depth == 0,
                });
            }
        }

        brace_depth += cleaned.matches('{').count() as isize;
        brace_depth -= cleaned.matches('}').count() as isize;
        if export_block_depth.is_some_and(|depth| brace_depth < depth) {
            export_block_depth = None;
        }
        byte_offset += raw_line.len();
    }
    modules.sort_by_key(|module| module.span.start_byte);
    imports.sort_by_key(|import| import.span.start_byte);
    exports.sort_by_key(|export| export.span.start_byte);
    (modules, imports, exports)
}

fn strip_cpp_comments(line: &str, in_block_comment: &mut bool) -> String {
    let bytes = line.as_bytes();
    let mut output = String::with_capacity(line.len());
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        if *in_block_comment {
            if bytes.get(index..index + 2) == Some(b"*/") {
                *in_block_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if quote.is_none() && bytes.get(index..index + 2) == Some(b"//") {
            break;
        }
        if quote.is_none() && bytes.get(index..index + 2) == Some(b"/*") {
            *in_block_comment = true;
            index += 2;
            continue;
        }
        let character = bytes[index] as char;
        if matches!(character, '"' | '\'') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        }
        output.push(character);
        if character == '\\' && quote.is_some() && index + 1 < bytes.len() {
            index += 1;
            output.push(bytes[index] as char);
        }
        index += 1;
    }
    output
}

fn parse_module_name(value: &str) -> Option<(String, Option<String>)> {
    let value = value.trim().trim_end_matches(';').trim();
    if value.is_empty() {
        return None;
    }
    let (name, partition) = value
        .split_once(':')
        .map_or((value, None), |(name, partition)| {
            (name, Some(partition.to_owned()))
        });
    Some((name.to_owned(), partition))
}

fn span_for_offsets(source: &[u8], start: usize, end: usize) -> SourceSpan {
    fn position(source: &[u8], offset: usize) -> SourcePosition {
        let prefix = &source[..offset.min(source.len())];
        let line = prefix.iter().filter(|byte| **byte == b'\n').count() + 1;
        let column = prefix
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(prefix.len() + 1, |index| prefix.len() - index);
        SourcePosition { line, column }
    }
    SourceSpan {
        start_byte: start,
        end_byte: end,
        start: position(source, start),
        end: position(source, end),
    }
}

struct Extraction<'a> {
    path: &'a Path,
    source: &'a [u8],
    symbols: Vec<Symbol>,
    declarations: Vec<SymbolDeclaration>,
    includes: Vec<DependencyReference>,
    calls: Vec<CallReference>,
    call_flows: BTreeMap<SymbolId, codegraide_core::CallFlow>,
    using_references: Vec<UsingReference>,
    id_counts: BTreeMap<String, usize>,
    preprocessor_uncertain: BTreeSet<SymbolId>,
}

impl<'a> Extraction<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        Self {
            path,
            source,
            symbols: Vec::new(),
            declarations: Vec::new(),
            includes: Vec::new(),
            calls: Vec::new(),
            call_flows: BTreeMap::new(),
            using_references: Vec::new(),
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
            "declaration" | "field_declaration" => {
                self.process_declaration(node, parent_id.clone());
            }
            "lambda_expression" => {
                self.process_lambda(node, parent_id);
                return;
            }
            "call_expression" => {
                self.process_call(node, callable_id.clone().or_else(|| parent_id.clone()));
            }
            "new_expression" => {
                self.process_constructor_call(
                    node,
                    callable_id.clone().or_else(|| parent_id.clone()),
                );
            }
            "init_declarator" => {
                self.process_direct_initialization(
                    node,
                    callable_id.clone().or_else(|| parent_id.clone()),
                );
            }
            "compound_literal_expression" => {
                self.process_braced_constructor(
                    node,
                    callable_id.clone().or_else(|| parent_id.clone()),
                );
            }
            "field_initializer" => {
                self.process_field_initializer(
                    node,
                    callable_id.clone().or_else(|| parent_id.clone()),
                );
            }
            "using_declaration"
            | "using_directive"
            | "alias_declaration"
            | "namespace_alias_definition" => self.process_using(node),
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
            callable_signature: None,
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
        let recovered_text = node_text(declaration_wrapper(node), self.source);
        if let Some(name) = recovered_export_namespace_name(&recovered_text) {
            self.process_recovered_export_namespace(node, parent_id, name);
            return;
        }
        let Some(declarator) = node.child_by_field_name("declarator") else {
            return;
        };
        let Some((name_node, name)) = callable_name(node, declarator, self.source) else {
            return;
        };
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
        let signature = callable_signature(declaration, declarator, name_node, self.source);
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
            callable_signature: Some(signature),
            documentation: None,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        if let Some(body) = body {
            self.call_flows
                .insert(id.clone(), call_flow::extract(body, self.source));
            self.visit_node(body, Some(id.clone()), Some(id), 0);
        }
    }

    fn process_recovered_export_namespace(
        &mut self,
        node: Node<'_>,
        parent_id: Option<SymbolId>,
        name: String,
    ) {
        let qualified_name = self.qualified_name(parent_id.as_ref(), &name);
        let id = self.symbol_id(SymbolKind::Namespace, &qualified_name);
        let body = node.child_by_field_name("body");
        self.symbols.push(Symbol {
            id: id.clone(),
            parent_id,
            kind: SymbolKind::Namespace,
            direct_declaration: !has_conditional_preprocessor_ancestor(node),
            name,
            qualified_name,
            span: source_span(node),
            body_span: body.map(source_span),
            name_span: None,
            completeness: completeness(node),
            modifiers: BTreeSet::new(),
            parameters: Vec::new(),
            decorators: Vec::new(),
            callable_signature: None,
            documentation: None,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        if let Some(body) = body {
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                self.visit_node(child, Some(id.clone()), None, 0);
            }
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
            callable_signature: None,
            documentation: None,
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        });
        if let Some(body) = body {
            self.visit_node(body, Some(id.clone()), Some(id), 0);
        }
    }

    fn process_declaration(&mut self, node: Node<'_>, parent_id: Option<SymbolId>) {
        let Some(declarator) = find_descendant(node, "function_declarator")
            .or_else(|| find_descendant(node, "operator_cast"))
        else {
            self.process_forward_declaration(node, parent_id);
            return;
        };
        let Some((name_node, name)) = callable_name(node, declarator, self.source) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let parent_kind = parent_id
            .as_ref()
            .and_then(|id| self.symbol(id))
            .map(|symbol| symbol.kind);
        let kind = if matches!(parent_kind, Some(SymbolKind::Class | SymbolKind::Struct)) {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };
        self.declarations.push(SymbolDeclaration {
            parent_id: parent_id.clone(),
            kind,
            name: name.clone(),
            qualified_name: self.qualified_name(parent_id.as_ref(), &name),
            role: SymbolOccurrenceRole::Declaration,
            span: source_span(node),
            name_span: Some(source_span(name_node)),
            completeness: completeness(node),
            callable_signature: Some(callable_signature(node, declarator, name_node, self.source)),
        });
    }

    fn process_forward_declaration(&mut self, node: Node<'_>, parent_id: Option<SymbolId>) {
        let text = node_text(node, self.source);
        let trimmed = text.trim_start();
        let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("class ") {
            (SymbolKind::Class, rest)
        } else if let Some(rest) = trimmed.strip_prefix("struct ") {
            (SymbolKind::Struct, rest)
        } else {
            return;
        };
        let name = rest
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .next()
            .unwrap_or_default();
        if name.is_empty() || !trimmed.trim_end().ends_with(';') {
            return;
        }
        self.declarations.push(SymbolDeclaration {
            parent_id: parent_id.clone(),
            kind,
            name: name.to_owned(),
            qualified_name: self.qualified_name(parent_id.as_ref(), name),
            role: SymbolOccurrenceRole::Declaration,
            span: source_span(node),
            name_span: None,
            completeness: completeness(node),
            callable_signature: None,
        });
    }

    fn process_call(&mut self, node: Node<'_>, enclosing_symbol: Option<SymbolId>) {
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        let callee = node_text(function, self.source);
        let (mut form, mut receiver) = call_form_and_receiver(&callee);
        let callee_terminal = cpp_call_components(&callee).pop();
        if self.symbols.iter().any(|symbol| {
            matches!(symbol.kind, SymbolKind::Class | SymbolKind::Struct)
                && callee_terminal
                    .as_deref()
                    .is_some_and(|name| symbol.name == name)
        }) {
            form = CallForm::Constructor;
        }
        let mut receiver_type_hint = receiver.as_deref().and_then(|receiver| {
            self.infer_receiver_type(receiver, node, enclosing_symbol.as_ref())
        });
        if form == CallForm::Free
            && let Some(variable_type) =
                self.infer_receiver_type(&callee, node, enclosing_symbol.as_ref())
        {
            form = if variable_type.contains("(*") {
                CallForm::Unknown
            } else {
                CallForm::Functor
            };
            receiver = Some(callee.clone());
            receiver_type_hint = Some(variable_type);
        }
        let arguments = node.child_by_field_name("arguments");
        let argument_details = arguments
            .map(|arguments| call_arguments(arguments, self.source))
            .unwrap_or_default();
        let macro_uncertain =
            looks_like_macro_identifier(callee.rsplit("::").next().unwrap_or(&callee).as_bytes());
        self.calls.push(CallReference {
            expression: node_text(node, self.source),
            components: cpp_call_components(&callee),
            callee,
            enclosing_symbol,
            arguments: CallArgumentShape {
                positional: argument_details.len(),
                keywords: Vec::new(),
                has_star_args: false,
                has_star_kwargs: false,
            },
            argument_details,
            form,
            receiver,
            receiver_type_hint,
            span: source_span(node),
            syntax_complete: completeness(node) == SymbolCompleteness::Complete
                && !has_conditional_preprocessor_ancestor(node),
            preprocessing_uncertain: macro_uncertain || has_conditional_preprocessor_ancestor(node),
        });
    }

    fn infer_receiver_type(
        &self,
        receiver: &str,
        call: Node<'_>,
        enclosing_symbol: Option<&SymbolId>,
    ) -> Option<String> {
        if receiver == "this" {
            return enclosing_symbol
                .and_then(|id| self.symbol(id))
                .and_then(|symbol| symbol.qualified_name.rsplit_once("::"))
                .map(|(owner, _)| owner.to_owned());
        }
        if receiver
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && character != '_')
        {
            return None;
        }
        let start = enclosing_symbol
            .and_then(|id| self.symbol(id))
            .map(|symbol| symbol.span.start_byte)
            .unwrap_or(0);
        let prefix = String::from_utf8_lossy(
            self.source
                .get(start..call.start_byte())
                .unwrap_or_default(),
        );
        for segment in prefix.rsplit([';', '{', '}', '\n']) {
            let Some(index) = segment.rfind(receiver) else {
                continue;
            };
            let before = segment[..index].trim();
            let after = segment[index + receiver.len()..].trim_start();
            let boundary_before = index == 0
                || !segment.as_bytes()[index - 1].is_ascii_alphanumeric()
                    && segment.as_bytes()[index - 1] != b'_';
            let boundary_after = after.is_empty()
                || after.starts_with(['=', '(', '{', ')', ',', '['])
                || after.starts_with("->")
                || after.starts_with('.');
            if !boundary_before || !boundary_after || before.is_empty() {
                continue;
            }
            let type_spelling = before
                .split_whitespace()
                .filter(|word| !matches!(*word, "const" | "volatile" | "static"))
                .collect::<Vec<_>>()
                .join(" ");
            if type_spelling == "auto" || type_spelling.contains('=') {
                return None;
            }
            return Some(normalize_type(&type_spelling));
        }
        None
    }

    fn process_constructor_call(&mut self, node: Node<'_>, enclosing_symbol: Option<SymbolId>) {
        let Some(type_node) = node.child_by_field_name("type") else {
            return;
        };
        let callee = node_text(type_node, self.source);
        let arguments = node
            .child_by_field_name("arguments")
            .or_else(|| find_descendant(node, "argument_list"));
        let argument_details = arguments
            .map(|arguments| call_arguments(arguments, self.source))
            .unwrap_or_default();
        self.calls.push(CallReference {
            expression: node_text(node, self.source),
            components: cpp_call_components(&callee),
            callee,
            enclosing_symbol,
            arguments: CallArgumentShape {
                positional: argument_details.len(),
                keywords: Vec::new(),
                has_star_args: false,
                has_star_kwargs: false,
            },
            argument_details,
            form: CallForm::Constructor,
            receiver: None,
            receiver_type_hint: None,
            span: source_span(node),
            syntax_complete: completeness(node) == SymbolCompleteness::Complete
                && !has_conditional_preprocessor_ancestor(node),
            preprocessing_uncertain: has_conditional_preprocessor_ancestor(node),
        });
    }

    fn process_direct_initialization(
        &mut self,
        node: Node<'_>,
        enclosing_symbol: Option<SymbolId>,
    ) {
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if !matches!(value.kind(), "argument_list" | "initializer_list") {
            return;
        }
        let Some(declaration) = node.parent() else {
            return;
        };
        let Some(type_node) = declaration.child_by_field_name("type") else {
            return;
        };
        let callee = node_text(type_node, self.source);
        if callee == "auto" {
            return;
        }
        self.push_constructor_reference(node, value, callee, enclosing_symbol);
    }

    fn process_braced_constructor(&mut self, node: Node<'_>, enclosing_symbol: Option<SymbolId>) {
        let (Some(type_node), Some(value)) = (
            node.child_by_field_name("type"),
            node.child_by_field_name("value"),
        ) else {
            return;
        };
        self.push_constructor_reference(
            node,
            value,
            node_text(type_node, self.source),
            enclosing_symbol,
        );
    }

    fn process_field_initializer(&mut self, node: Node<'_>, enclosing_symbol: Option<SymbolId>) {
        let mut cursor = node.walk();
        let children = node.named_children(&mut cursor).collect::<Vec<_>>();
        let Some(name) = children.first().copied() else {
            return;
        };
        let Some(arguments) = children
            .iter()
            .copied()
            .find(|child| matches!(child.kind(), "argument_list" | "initializer_list"))
        else {
            return;
        };
        self.push_constructor_reference(
            node,
            arguments,
            node_text(name, self.source),
            enclosing_symbol,
        );
    }

    fn push_constructor_reference(
        &mut self,
        node: Node<'_>,
        arguments: Node<'_>,
        callee: String,
        enclosing_symbol: Option<SymbolId>,
    ) {
        let argument_details = call_arguments(arguments, self.source);
        self.calls.push(CallReference {
            expression: node_text(node, self.source),
            components: cpp_call_components(&callee),
            callee,
            enclosing_symbol,
            arguments: CallArgumentShape {
                positional: argument_details.len(),
                keywords: Vec::new(),
                has_star_args: false,
                has_star_kwargs: false,
            },
            argument_details,
            form: CallForm::Constructor,
            receiver: None,
            receiver_type_hint: None,
            span: source_span(node),
            syntax_complete: completeness(node) == SymbolCompleteness::Complete
                && !has_conditional_preprocessor_ancestor(node),
            preprocessing_uncertain: has_conditional_preprocessor_ancestor(node),
        });
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

    fn process_using(&mut self, node: Node<'_>) {
        let text = node_text(node, self.source);
        let trimmed = text.trim().trim_end_matches(';').trim();
        let parsed = if let Some(target) = trimmed.strip_prefix("using namespace ") {
            Some((UsingReferenceKind::Namespace, target.trim(), None))
        } else if let Some(rest) = trimmed.strip_prefix("using ") {
            if let Some((alias, target)) = rest.split_once('=') {
                Some((UsingReferenceKind::Alias, target.trim(), Some(alias.trim())))
            } else {
                Some((UsingReferenceKind::Declaration, rest.trim(), None))
            }
        } else if let Some(rest) = trimmed.strip_prefix("namespace ") {
            rest.split_once('=').map(|(alias, target)| {
                (UsingReferenceKind::Alias, target.trim(), Some(alias.trim()))
            })
        } else {
            None
        };
        if let Some((kind, target, alias)) = parsed
            && !target.is_empty()
        {
            self.using_references.push(UsingReference {
                kind,
                target: target.to_owned(),
                alias: alias.map(str::to_owned),
                span: source_span(node),
                complete: completeness(node) == SymbolCompleteness::Complete
                    && !has_conditional_preprocessor_ancestor(node),
            });
        }
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
        self.declarations.sort_by(|left, right| {
            left.span
                .start_byte
                .cmp(&right.span.start_byte)
                .then_with(|| left.qualified_name.cmp(&right.qualified_name))
        });
        self.calls.sort_by_key(|call| call.span.start_byte);
        self.using_references
            .sort_by_key(|reference| reference.span.start_byte);
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

// tree-sitter can read `Owner::operator Type()` as a return type `Owner::operator`
// and a function named `Type`. Preserve the operator so it cannot match Type().
fn callable_name<'tree>(
    node: Node<'tree>,
    declarator: Node<'tree>,
    source: &[u8],
) -> Option<(Node<'tree>, String)> {
    if let Some(cast) = find_descendant(declarator, "operator_cast") {
        let name_node = cast.child_by_field_name("type")?;
        let spelling = node_text(declarator, source);
        return Some((name_node, spelling.split('(').next()?.trim().to_owned()));
    }
    let name_node = function_name_node(declarator_name_node(declarator)?, source);
    let mut name = node_text(name_node, source);
    if let Some(kind) = node.child_by_field_name("type") {
        let spelling = node_text(kind, source);
        if let Some(owner) = spelling.strip_suffix("::operator") {
            name = format!("{owner}::operator {name}");
        }
    }
    Some((name_node, name))
}

fn function_name_node<'tree>(node: Node<'tree>, source: &[u8]) -> Node<'tree> {
    let text = node_text(node, source);
    let Some(prefix) = text.split_ascii_whitespace().next() else {
        return node;
    };
    if !text.contains('\n')
        || text.contains("::")
        || !looks_like_macro_identifier(prefix.as_bytes())
    {
        return node;
    }

    last_identifier_descendant(node).unwrap_or(node)
}

fn last_identifier_descendant(node: Node<'_>) -> Option<Node<'_>> {
    let mut result = matches!(node.kind(), "identifier" | "field_identifier").then_some(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(identifier) = last_identifier_descendant(child) {
            result = Some(identifier);
        }
    }
    result
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
            message: format!(
                "C++ parser recovered while parsing {}",
                recovery_context(node)
            ),
            span: Some(source_span(node)),
        });
    } else if node.is_missing() {
        diagnostics.push(AnalysisDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: "missing-syntax".to_owned(),
            message: format!(
                "C++ parser inserted missing {} syntax while parsing {}",
                node.kind(),
                recovery_context(node)
            ),
            span: Some(source_span(node)),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_diagnostics(child, ancestor_is_error || is_error, diagnostics);
    }
}

fn recovery_context(node: Node<'_>) -> &'static str {
    let mut ancestor = node.parent();
    while let Some(current) = ancestor {
        if matches!(current.kind(), "preproc_def" | "preproc_function_def") {
            return "a macro definition";
        }
        ancestor = current.parent();
    }

    let mut ancestor = node.parent();
    while let Some(current) = ancestor {
        let context = match current.kind() {
            "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_else" => {
                Some("conditional compilation")
            }
            "lambda_expression" => Some("a lambda"),
            "function_definition" => Some("a function definition"),
            "template_declaration" => Some("a template declaration"),
            "class_specifier" => Some("a class"),
            "struct_specifier" => Some("a struct"),
            "parameter_declaration" | "optional_parameter_declaration" => Some("a parameter"),
            "argument_list" | "template_argument_list" => Some("function arguments"),
            "declaration" | "field_declaration" | "alias_declaration" => Some("a declaration"),
            "if_statement" | "switch_statement" => Some("a condition"),
            "for_statement" | "for_range_loop" | "while_statement" | "do_statement" => {
                Some("a loop")
            }
            "expression_statement" | "binary_expression" | "call_expression" => {
                Some("an expression")
            }
            _ => None,
        };
        if let Some(context) = context {
            return context;
        }
        ancestor = current.parent();
    }
    "C++ source"
}

fn classify_recovery_diagnostics(source: &[u8], diagnostics: &mut [AnalysisDiagnostic]) {
    let lines = source.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    let macro_definition_lines = continued_macro_definition_lines(&lines);

    for diagnostic in diagnostics {
        let Some(span) = diagnostic.span else {
            continue;
        };
        if macro_definition_lines.contains(&span.start.line) {
            diagnostic.code = "macro-definition-recovery".to_owned();
            diagnostic.message =
                "Macro definition body could not be fully parsed without preprocessing".to_owned();
        } else if span_contains_macro_dependent_syntax(span, &lines) {
            diagnostic.code = "macro-dependent-recovery".to_owned();
            diagnostic.message =
                "C++ syntax at this location depends on macro expansion".to_owned();
        }
    }
}

fn continued_macro_definition_lines(lines: &[&[u8]]) -> BTreeSet<usize> {
    let mut result = BTreeSet::new();
    let mut continuing = false;

    for (index, line) in lines.iter().enumerate() {
        let line_number = index + 1;
        let trimmed_start = trim_ascii_start(line);
        let starts_definition = trimmed_start
            .strip_prefix(b"#")
            .map(trim_ascii_start)
            .is_some_and(|directive| {
                directive
                    .strip_prefix(b"define")
                    .is_some_and(|remainder| remainder.first().is_some_and(u8::is_ascii_whitespace))
            });
        if continuing || starts_definition {
            result.insert(line_number);
        }
        continuing = (continuing || starts_definition) && trim_ascii_end(line).ends_with(b"\\");
    }

    result
}

fn span_contains_macro_dependent_syntax(span: SourceSpan, lines: &[&[u8]]) -> bool {
    let start = span.start.line.saturating_sub(1);
    let end = span.end.line.min(span.start.line.saturating_add(3));
    lines
        .get(start..end)
        .is_some_and(|span_lines| span_lines.iter().any(|line| looks_like_macro_line(line)))
}

fn looks_like_macro_line(line: &[u8]) -> bool {
    let line = trim_ascii_start(line);
    let identifier_end = line
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        .unwrap_or(line.len());
    let identifier = &line[..identifier_end];
    if !looks_like_macro_identifier(identifier) {
        return false;
    }

    let remainder = trim_ascii_start(&line[identifier_end..]);
    remainder.is_empty()
        || remainder.starts_with(b"(")
        || remainder.starts_with(b"{")
        || remainder.starts_with(b";")
}

fn looks_like_macro_identifier(identifier: &[u8]) -> bool {
    identifier.len() >= 2
        && identifier[0].is_ascii_uppercase()
        && identifier.iter().any(u8::is_ascii_alphabetic)
        && identifier
            .iter()
            .all(|byte| !byte.is_ascii_alphabetic() || byte.is_ascii_uppercase())
}

fn trim_ascii_start(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    value
}

fn trim_ascii_end(mut value: &[u8]) -> &[u8] {
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
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
