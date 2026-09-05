use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::report::LanguageId;

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum AnalysisLevel {
    Syntax,
    ProjectResolution,
    SemanticEnrichment,
}

impl AnalysisLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::ProjectResolution => "project-resolution",
            Self::SemanticEnrichment => "semantic-enrichment",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum AnalyzerCapability {
    Parse,
    Symbols,
    Documentation,
    DependencyReferences,
    CallReferences,
    DecisionEvents,
    NestingEvents,
    Measurements,
    ExplicitExports,
}

impl AnalyzerCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Symbols => "symbols",
            Self::Documentation => "documentation",
            Self::DependencyReferences => "dependency-references",
            Self::CallReferences => "call-references",
            Self::DecisionEvents => "decision-events",
            Self::NestingEvents => "nesting-events",
            Self::Measurements => "measurements",
            Self::ExplicitExports => "explicit-exports",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GrammarDescriptor {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct QueryDescriptor {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum MeasurementConcept {
    DeclarationPhysicalLines,
    BodyPhysicalLines,
    DeclaredParameterCount,
    CallerParameterCount,
    MaxControlFlowNesting,
    CyclomaticComplexity,
}

impl MeasurementConcept {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeclarationPhysicalLines => "declaration-physical-lines",
            Self::BodyPhysicalLines => "body-physical-lines",
            Self::DeclaredParameterCount => "declared-parameter-count",
            Self::CallerParameterCount => "caller-parameter-count",
            Self::MaxControlFlowNesting => "max-control-flow-nesting",
            Self::CyclomaticComplexity => "cyclomatic-complexity",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MeasurementDescriptor {
    pub concept: MeasurementConcept,
    pub id: String,
    pub definition_version: String,
    pub unit: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AnalyzerDescriptor {
    pub id: String,
    pub language: LanguageId,
    pub version: String,
    pub level: AnalysisLevel,
    pub capabilities: BTreeSet<AnalyzerCapability>,
    pub grammar: Option<GrammarDescriptor>,
    pub queries: Vec<QueryDescriptor>,
    pub measurements: Vec<MeasurementDescriptor>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct AnalysisInput<'a> {
    pub path: &'a Path,
    pub source: &'a [u8],
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

impl DiagnosticSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct SourcePosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start: SourcePosition,
    pub end: SourcePosition,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct SymbolId(String);

impl SymbolId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum SymbolKind {
    Module,
    Namespace,
    Class,
    Struct,
    Function,
    Method,
    Lambda,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Namespace => "namespace",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Function => "function",
            Self::Method => "method",
            Self::Lambda => "lambda",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum SymbolCompleteness {
    Complete,
    Partial,
}

impl SymbolCompleteness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum SymbolModifier {
    Async,
    Static,
    ClassMethod,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum CallableQualifier {
    Const,
    Volatile,
    LvalueReference,
    RvalueReference,
    Static,
    Noexcept,
}

impl CallableQualifier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Const => "const",
            Self::Volatile => "volatile",
            Self::LvalueReference => "lvalue-reference",
            Self::RvalueReference => "rvalue-reference",
            Self::Static => "static",
            Self::Noexcept => "noexcept",
        }
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct CallableParameter {
    pub name: Option<String>,
    pub type_spelling: Option<String>,
    pub has_default: bool,
    pub variadic: bool,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct CallableSignature {
    pub display: String,
    pub normalized_key: String,
    pub return_type: Option<String>,
    pub parameters: Vec<CallableParameter>,
    pub qualifiers: BTreeSet<CallableQualifier>,
    pub template_parameter_count: usize,
    pub virtual_dispatch: bool,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum SymbolOccurrenceRole {
    Declaration,
    Definition,
}

impl SymbolOccurrenceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::Definition => "definition",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SymbolDeclaration {
    pub parent_id: Option<SymbolId>,
    pub kind: SymbolKind,
    pub name: String,
    pub qualified_name: String,
    pub role: SymbolOccurrenceRole,
    pub span: SourceSpan,
    pub name_span: Option<SourceSpan>,
    pub completeness: SymbolCompleteness,
    pub callable_signature: Option<CallableSignature>,
}

impl SymbolModifier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Async => "async",
            Self::Static => "staticmethod",
            Self::ClassMethod => "classmethod",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ParameterKind {
    PositionalOnly,
    PositionalOrKeyword,
    VariadicPositional,
    KeywordOnly,
    VariadicKeyword,
}

impl ParameterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PositionalOnly => "positional-only",
            Self::PositionalOrKeyword => "positional-or-keyword",
            Self::VariadicPositional => "variadic-positional",
            Self::KeywordOnly => "keyword-only",
            Self::VariadicKeyword => "variadic-keyword",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub kind: ParameterKind,
    pub span: SourceSpan,
    pub has_default: bool,
    pub has_annotation: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Decorator {
    pub expression: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum NestingEventKind {
    Conditional,
    Loop,
    Switch,
    ExceptionHandling,
    ContextManager,
    Match,
    Comprehension,
}

impl NestingEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conditional => "conditional",
            Self::Loop => "loop",
            Self::Switch => "switch",
            Self::ExceptionHandling => "exception-handling",
            Self::ContextManager => "context-manager",
            Self::Match => "match",
            Self::Comprehension => "comprehension",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DecisionEventKind {
    Conditional,
    Loop,
    ExceptionHandler,
    PatternBranch,
    MatchGuard,
    BooleanShortCircuit,
    ConditionalExpression,
    ComprehensionLoop,
    ComprehensionFilter,
    Assertion,
    SwitchCase,
}

impl DecisionEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conditional => "conditional",
            Self::Loop => "loop",
            Self::ExceptionHandler => "exception-handler",
            Self::PatternBranch => "pattern-branch",
            Self::MatchGuard => "match-guard",
            Self::BooleanShortCircuit => "boolean-short-circuit",
            Self::ConditionalExpression => "conditional-expression",
            Self::ComprehensionLoop => "comprehension-loop",
            Self::ComprehensionFilter => "comprehension-filter",
            Self::Assertion => "assertion",
            Self::SwitchCase => "switch-case",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DecisionEvent {
    pub kind: DecisionEventKind,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NestingEvent {
    pub kind: NestingEventKind,
    pub depth: usize,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum MeasurementStatus {
    Measured,
    Unavailable,
}

impl MeasurementStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Measurement {
    pub id: String,
    pub definition_version: String,
    pub unit: String,
    pub status: MeasurementStatus,
    pub value: Option<u64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DocumentationStatus {
    Documented,
    Missing,
    Unavailable,
}

impl DocumentationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Documented => "documented",
            Self::Missing => "missing",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SymbolDocumentation {
    pub status: DocumentationStatus,
    pub span: Option<SourceSpan>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DependencyKind {
    Import,
    Include,
}

impl DependencyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Include => "include",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResolutionLevel {
    Syntactic,
    Project,
    Semantic,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ImportScope {
    Module,
    Class,
    Callable,
}

impl ImportScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Class => "class",
            Self::Callable => "callable",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ImportUsage {
    Runtime,
    TypeCheckingOnly,
}

impl ImportUsage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::TypeCheckingOnly => "type-checking-only",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ImportRequirement {
    Required,
    Optional,
}

impl ImportRequirement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct ImportContext {
    pub scope: ImportScope,
    pub usage: ImportUsage,
    pub requirement: ImportRequirement,
    pub conditional: bool,
}

impl Default for ImportContext {
    fn default() -> Self {
        Self {
            scope: ImportScope::Module,
            usage: ImportUsage::Runtime,
            requirement: ImportRequirement::Required,
            conditional: false,
        }
    }
}

impl ResolutionLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Syntactic => "syntactic",
            Self::Project => "project-resolution",
            Self::Semantic => "semantic-enrichment",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ImportReference {
    pub module: Option<String>,
    pub imported_name: Option<String>,
    pub alias: Option<String>,
    pub relative_level: usize,
    pub wildcard: bool,
    pub resolution: ResolutionLevel,
    pub enclosing_symbol: Option<SymbolId>,
    pub context: ImportContext,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum IncludeDelimiter {
    Angle,
    Quote,
    Macro,
}

impl IncludeDelimiter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Angle => "angle",
            Self::Quote => "quote",
            Self::Macro => "macro",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IncludeReference {
    pub target: String,
    pub delimiter: IncludeDelimiter,
    pub conditional: bool,
    pub resolution: ResolutionLevel,
    pub enclosing_symbol: Option<SymbolId>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DependencyReference {
    Import(ImportReference),
    Include(IncludeReference),
}

impl DependencyReference {
    pub fn kind(&self) -> DependencyKind {
        match self {
            Self::Import(_) => DependencyKind::Import,
            Self::Include(_) => DependencyKind::Include,
        }
    }

    pub fn as_import(&self) -> Option<&ImportReference> {
        match self {
            Self::Import(reference) => Some(reference),
            Self::Include(_) => None,
        }
    }

    pub fn as_include(&self) -> Option<&IncludeReference> {
        match self {
            Self::Include(reference) => Some(reference),
            Self::Import(_) => None,
        }
    }

    pub fn resolution(&self) -> ResolutionLevel {
        match self {
            Self::Import(reference) => reference.resolution,
            Self::Include(reference) => reference.resolution,
        }
    }

    pub fn enclosing_symbol(&self) -> Option<&SymbolId> {
        match self {
            Self::Import(reference) => reference.enclosing_symbol.as_ref(),
            Self::Include(reference) => reference.enclosing_symbol.as_ref(),
        }
    }

    pub fn span(&self) -> SourceSpan {
        match self {
            Self::Import(reference) => reference.span,
            Self::Include(reference) => reference.span,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallArgumentShape {
    pub positional: usize,
    pub keywords: Vec<String>,
    pub has_star_args: bool,
    pub has_star_kwargs: bool,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum CallForm {
    Unknown,
    Free,
    Qualified,
    Member,
    PointerMember,
    Functor,
    Constructor,
}

impl CallForm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Free => "free",
            Self::Qualified => "qualified",
            Self::Member => "member",
            Self::PointerMember => "pointer-member",
            Self::Functor => "functor",
            Self::Constructor => "constructor",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallArgument {
    pub expression: String,
    pub type_hint: Option<String>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CallReference {
    pub expression: String,
    pub callee: String,
    pub components: Vec<String>,
    pub enclosing_symbol: Option<SymbolId>,
    pub arguments: CallArgumentShape,
    pub argument_details: Vec<CallArgument>,
    pub form: CallForm,
    pub receiver: Option<String>,
    pub receiver_type_hint: Option<String>,
    pub span: SourceSpan,
    pub syntax_complete: bool,
    pub preprocessing_uncertain: bool,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum LanguageModuleKind {
    Interface,
    Implementation,
    InterfacePartition,
    ImplementationPartition,
}

impl LanguageModuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interface => "interface",
            Self::Implementation => "implementation",
            Self::InterfacePartition => "interface-partition",
            Self::ImplementationPartition => "implementation-partition",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LanguageModule {
    pub name: String,
    pub partition: Option<String>,
    pub kind: LanguageModuleKind,
    pub exported: bool,
    pub span: SourceSpan,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ModuleImportKind {
    Named,
    Partition,
    HeaderAngle,
    HeaderQuote,
}

impl ModuleImportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::Partition => "partition",
            Self::HeaderAngle => "header-angle",
            Self::HeaderQuote => "header-quote",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ModuleImport {
    pub target: String,
    pub kind: ModuleImportKind,
    pub exported: bool,
    pub conditional: bool,
    pub span: SourceSpan,
    pub complete: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ModuleExport {
    pub target: String,
    pub span: SourceSpan,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum UsingReferenceKind {
    Declaration,
    Namespace,
    Alias,
}

impl UsingReferenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::Namespace => "namespace",
            Self::Alias => "alias",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UsingReference {
    pub kind: UsingReferenceKind,
    pub target: String,
    pub alias: Option<String>,
    pub span: SourceSpan,
    pub complete: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Symbol {
    pub id: SymbolId,
    pub parent_id: Option<SymbolId>,
    pub kind: SymbolKind,
    pub direct_declaration: bool,
    pub name: String,
    pub qualified_name: String,
    pub span: SourceSpan,
    pub body_span: Option<SourceSpan>,
    pub name_span: Option<SourceSpan>,
    pub completeness: SymbolCompleteness,
    pub modifiers: BTreeSet<SymbolModifier>,
    pub parameters: Vec<Parameter>,
    pub decorators: Vec<Decorator>,
    pub callable_signature: Option<CallableSignature>,
    pub documentation: Option<SymbolDocumentation>,
    pub nesting_events: Vec<NestingEvent>,
    pub decision_events: Vec<DecisionEvent>,
    pub measurements: Vec<Measurement>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct AnalysisFacts {
    pub call_flows: std::collections::BTreeMap<SymbolId, crate::CallFlow>,
    pub symbols: Vec<Symbol>,
    pub declarations: Vec<SymbolDeclaration>,
    pub dependencies: Vec<DependencyReference>,
    pub calls: Vec<CallReference>,
    pub modules: Vec<LanguageModule>,
    pub module_imports: Vec<ModuleImport>,
    pub module_exports: Vec<ModuleExport>,
    pub using_references: Vec<UsingReference>,
    pub explicit_exports: Option<ExplicitExports>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AnalysisDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum FileAnalysisStatus {
    Successful,
    Partial,
    Failed,
}

impl FileAnalysisStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Successful => "successful",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FileAnalysis {
    pub path: PathBuf,
    pub status: FileAnalysisStatus,
    pub diagnostics: Vec<AnalysisDiagnostic>,
    pub facts: AnalysisFacts,
}

pub trait LanguageAnalyzer {
    fn descriptor(&self) -> &AnalyzerDescriptor;

    fn analyze(&mut self, input: AnalysisInput<'_>) -> FileAnalysis;
}

pub struct AnalyzerRegistry {
    analyzers: BTreeMap<LanguageId, Box<dyn LanguageAnalyzer>>,
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        Self {
            analyzers: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        analyzer: Box<dyn LanguageAnalyzer>,
    ) -> Result<(), AnalyzerRegistryError> {
        let language = analyzer.descriptor().language.clone();
        if self.analyzers.contains_key(&language) {
            return Err(AnalyzerRegistryError::DuplicateLanguage(language));
        }

        self.analyzers.insert(language, analyzer);
        Ok(())
    }

    pub fn analyzer_for(&self, language: &LanguageId) -> bool {
        self.analyzers.contains_key(language)
    }

    pub fn descriptor_for(&self, language: &LanguageId) -> Option<&AnalyzerDescriptor> {
        self.analyzers
            .get(language)
            .map(|analyzer| analyzer.descriptor())
    }

    pub fn analyze(
        &mut self,
        language: &LanguageId,
        input: AnalysisInput<'_>,
    ) -> Option<FileAnalysis> {
        self.analyzers
            .get_mut(language)
            .map(|analyzer| analyzer.analyze(input))
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &AnalyzerDescriptor> {
        self.analyzers
            .values()
            .map(|analyzer| analyzer.descriptor())
    }
}

impl Default for AnalyzerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AnalyzerRegistryError {
    DuplicateLanguage(LanguageId),
}

impl fmt::Display for AnalyzerRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateLanguage(language) => {
                write!(
                    formatter,
                    "analyzer already registered for {}",
                    language.as_str()
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExplicitExportStatus {
    NotDeclared,
    Complete,
    Partial,
    Unavailable,
}

impl ExplicitExportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotDeclared => "not-declared",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExplicitExportName {
    pub name: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExplicitExports {
    pub status: ExplicitExportStatus,
    pub names: Vec<ExplicitExportName>,
    pub declaration_span: Option<SourceSpan>,
    pub reason: Option<String>,
}

impl std::error::Error for AnalyzerRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubAnalyzer {
        descriptor: AnalyzerDescriptor,
    }

    impl LanguageAnalyzer for StubAnalyzer {
        fn descriptor(&self) -> &AnalyzerDescriptor {
            &self.descriptor
        }

        fn analyze(&mut self, input: AnalysisInput<'_>) -> FileAnalysis {
            FileAnalysis {
                path: input.path.to_path_buf(),
                status: FileAnalysisStatus::Successful,
                diagnostics: Vec::new(),
                facts: AnalysisFacts::default(),
            }
        }
    }

    fn stub(language: &str) -> Box<dyn LanguageAnalyzer> {
        Box::new(StubAnalyzer {
            descriptor: AnalyzerDescriptor {
                id: format!("stub-{language}"),
                language: LanguageId::new(language),
                version: "0.1.0".to_owned(),
                level: AnalysisLevel::Syntax,
                capabilities: [AnalyzerCapability::Parse].into_iter().collect(),
                grammar: None,
                queries: Vec::new(),
                measurements: Vec::new(),
                limitations: Vec::new(),
            },
        })
    }

    #[test]
    fn rejects_duplicate_language_registration() {
        let mut registry = AnalyzerRegistry::new();
        registry
            .register(stub("python"))
            .expect("first registration");

        let error = registry
            .register(stub("python"))
            .expect_err("duplicate registration should fail");

        assert_eq!(error.to_string(), "analyzer already registered for python");
    }

    #[test]
    fn explicit_export_contract_names_are_stable() {
        let statuses = [
            (ExplicitExportStatus::NotDeclared, "not-declared"),
            (ExplicitExportStatus::Complete, "complete"),
            (ExplicitExportStatus::Partial, "partial"),
            (ExplicitExportStatus::Unavailable, "unavailable"),
        ];

        for (status, expected) in statuses {
            assert_eq!(status.as_str(), expected);
        }

        assert_eq!(
            AnalyzerCapability::ExplicitExports.as_str(),
            "explicit-exports"
        );
    }
}
