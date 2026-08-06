use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::lines::RepositoryLineCounts;
use crate::review::{ReviewEvaluation, review_status_code};

pub const INVENTORY_REPORT_SCHEMA_VERSION: &str = "0.2.0";
pub const SYNTAX_REPORT_SCHEMA_VERSION: &str = "0.4.0";
pub const INVENTORY_REPORT_DEFINITION_VERSION: &str = "inventory-report-v1";
pub const PHYSICAL_LINE_DEFINITION_VERSION: &str = "physical-lines-v1";
pub const SYNTAX_ANALYSIS_DEFINITION_VERSION: &str = "syntax-analysis-v2";
pub const REVIEW_REPORT_SCHEMA_VERSION: &str = "0.1.0";
pub const GATE_REPORT_SCHEMA_VERSION: &str = "0.1.0";
pub const REVIEW_ANALYSIS_DEFINITION_VERSION: &str = "review-analysis-v1";

#[cfg(unix)]
const JSON_PATH_ENCODING: &str = "utf8-with-percent-encoded-bytes";
#[cfg(not(unix))]
const JSON_PATH_ENCODING: &str = "normalized-utf8";

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum FileCategory {
    Source,
    Documentation,
    Configuration,
    Data,
    Assets,
    Uncategorized,
}

impl FileCategory {
    pub const ALL: [Self; 6] = [
        Self::Source,
        Self::Documentation,
        Self::Configuration,
        Self::Data,
        Self::Assets,
        Self::Uncategorized,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Documentation => "documentation",
            Self::Configuration => "configuration",
            Self::Data => "data",
            Self::Assets => "assets",
            Self::Uncategorized => "uncategorized",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "source" => Some(Self::Source),
            "documentation" => Some(Self::Documentation),
            "configuration" => Some(Self::Configuration),
            "data" => Some(Self::Data),
            "assets" => Some(Self::Assets),
            "uncategorized" => Some(Self::Uncategorized),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct LanguageId(String);

impl LanguageId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExtensionId(String);

impl ExtensionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InventoryDiagnostic {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct InventoryJsonReport {
    pub report_schema_version: &'static str,
    pub tool: JsonTool,
    pub analysis: JsonAnalysis,
    pub inventory: JsonInventory,
    pub diagnostics: Vec<JsonDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct JsonTool {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct JsonAnalysis {
    pub kind: &'static str,
    pub definition_version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct JsonInventory {
    pub path_encoding: &'static str,
    pub inventoried_files: usize,
    pub categories: BTreeMap<String, JsonCategory>,
    pub languages: BTreeMap<String, usize>,
    pub uncategorized_extensions: BTreeMap<String, usize>,
    pub ignored: JsonIgnored,
    pub included_ignored_files: usize,
    pub line_counts: JsonLineCounts,
}

#[derive(Debug, Serialize)]
pub struct JsonCategory {
    pub count: usize,
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonIgnored {
    pub exact: bool,
    pub files: Vec<String>,
    pub directories: Vec<String>,
    pub builtin_directories: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonLineCounts {
    pub definition_version: &'static str,
    pub total: JsonLineCountValues,
    pub by_language: BTreeMap<String, JsonLineCountValues>,
}

#[derive(Debug, Serialize)]
pub struct JsonLineCountValues {
    pub files: usize,
    pub total_lines: usize,
    pub source_lines: usize,
    pub comment_lines: usize,
    pub blank_lines: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonDiagnostic {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct AnalysisJsonReport {
    pub report_schema_version: &'static str,
    pub tool: JsonTool,
    pub analysis: JsonSyntaxAnalysis,
    pub inventory: JsonAnalysisInventory,
    pub analyzers: Vec<JsonAnalyzerRun>,
    pub review: JsonReview,
    pub diagnostics: Vec<JsonAnalysisDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct ReviewJsonReport {
    pub report_schema_version: &'static str,
    pub tool: JsonTool,
    pub analysis: JsonReviewAnalysis,
    pub ranking_count: usize,
    pub finding_count: usize,
    pub review: JsonReview,
}

#[derive(Debug, Serialize)]
pub struct JsonReviewAnalysis {
    pub kind: &'static str,
    pub definition_version: &'static str,
    pub selected_files: usize,
}

#[derive(Debug, Serialize)]
pub struct GateJsonReport {
    pub report_schema_version: &'static str,
    pub tool: JsonTool,
    pub status: &'static str,
    pub exit_code: u8,
    pub finding_count: usize,
    pub top_findings: Vec<JsonGateFinding>,
}

#[derive(Debug, Serialize)]
pub struct JsonGateFinding {
    pub rule_id: String,
    pub path: Option<String>,
    pub qualified_name: Option<String>,
    pub score: Option<u64>,
    pub risk: &'static str,
    pub required_action: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct JsonSyntaxAnalysis {
    pub kind: &'static str,
    pub definition_version: &'static str,
    pub selection: JsonAnalysisSelection,
}

#[derive(Debug, Serialize)]
pub struct JsonAnalysisSelection {
    pub target_kind: &'static str,
    pub match_patterns: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonAnalysisInventory {
    pub path_encoding: &'static str,
    pub inventoried_files: usize,
    pub languages: BTreeMap<String, usize>,
    pub inventory_only_languages: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct JsonAnalyzerRun {
    pub id: String,
    pub language: String,
    pub version: String,
    pub level: &'static str,
    pub capabilities: Vec<&'static str>,
    pub grammar: Option<JsonGrammar>,
    pub queries: Vec<JsonQuery>,
    pub limitations: Vec<String>,
    pub counts: JsonAnalysisCounts,
    pub files: Vec<JsonFileAnalysis>,
}

#[derive(Debug, Serialize)]
pub struct JsonGrammar {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct JsonQuery {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct JsonAnalysisCounts {
    pub analyzed: usize,
    pub successful: usize,
    pub partial: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonFileAnalysis {
    pub path: String,
    pub status: &'static str,
    pub diagnostics: Vec<JsonAnalysisDiagnostic>,
    pub symbols: Vec<JsonSymbol>,
    pub dependencies: Vec<JsonDependencyReference>,
    pub calls: Vec<JsonCallReference>,
}

#[derive(Debug, Serialize)]
pub struct JsonSymbol {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: &'static str,
    pub name: String,
    pub qualified_name: String,
    pub span: JsonSourceSpan,
    pub body_span: Option<JsonSourceSpan>,
    pub name_span: Option<JsonSourceSpan>,
    pub completeness: &'static str,
    pub modifiers: Vec<&'static str>,
    pub parameters: Vec<JsonParameter>,
    pub decorators: Vec<JsonDecorator>,
    pub nesting_events: Vec<JsonNestingEvent>,
    pub decision_events: Vec<JsonDecisionEvent>,
    pub measurements: Vec<JsonMeasurement>,
}

#[derive(Debug, Serialize)]
pub struct JsonParameter {
    pub name: String,
    pub kind: &'static str,
    pub span: JsonSourceSpan,
    pub has_default: bool,
    pub has_annotation: bool,
}

#[derive(Debug, Serialize)]
pub struct JsonDecorator {
    pub expression: String,
    pub span: JsonSourceSpan,
}

#[derive(Debug, Serialize)]
pub struct JsonNestingEvent {
    pub kind: &'static str,
    pub depth: usize,
    pub span: JsonSourceSpan,
}

#[derive(Debug, Serialize)]
pub struct JsonDecisionEvent {
    pub kind: &'static str,
    pub span: JsonSourceSpan,
}

#[derive(Debug, Serialize)]
pub struct JsonMeasurement {
    pub id: String,
    pub definition_version: String,
    pub unit: String,
    pub status: &'static str,
    pub value: Option<u64>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonDependencyReference {
    pub kind: &'static str,
    pub module: Option<String>,
    pub imported_name: Option<String>,
    pub alias: Option<String>,
    pub relative_level: usize,
    pub wildcard: bool,
    pub resolution: &'static str,
    pub enclosing_symbol: Option<String>,
    pub scope: &'static str,
    pub usage: &'static str,
    pub requirement: &'static str,
    pub conditional: bool,
    pub span: JsonSourceSpan,
}

#[derive(Debug, Serialize)]
pub struct JsonCallReference {
    pub callee: String,
    pub components: Vec<String>,
    pub enclosing_symbol: Option<String>,
    pub positional_arguments: usize,
    pub keyword_arguments: Vec<String>,
    pub has_star_args: bool,
    pub has_star_kwargs: bool,
    pub syntax_complete: bool,
    pub span: JsonSourceSpan,
}

#[derive(Debug, Serialize)]
pub struct JsonAnalysisDiagnostic {
    pub severity: &'static str,
    pub code: String,
    pub message: String,
    pub span: Option<JsonSourceSpan>,
}

#[derive(Debug, Serialize)]
pub struct JsonReview {
    pub status: &'static str,
    pub policy: JsonReviewPolicy,
    pub coverage: JsonReviewCoverage,
    pub rankings: Vec<JsonReviewRankingEntry>,
    pub findings: Vec<JsonReviewFinding>,
}

#[derive(Debug, Serialize)]
pub struct JsonReviewPolicy {
    pub definition_version: &'static str,
    pub sources: Vec<String>,
    pub risk_bands: JsonRiskBands,
    pub complexity_review_at: u64,
    pub complexity_block_at: Option<u64>,
    pub exceptions: Vec<JsonReviewException>,
}

#[derive(Debug, Serialize)]
pub struct JsonRiskBands {
    pub moderate_at: u64,
    pub high_at: u64,
    pub critical_at: u64,
}

#[derive(Debug, Serialize)]
pub struct JsonReviewException {
    pub symbol_id: String,
    pub reason: String,
    pub approved_max: Option<u64>,
    pub unbounded: bool,
}

#[derive(Debug, Serialize)]
pub struct JsonReviewCoverage {
    pub selected_language_files: usize,
    pub unsupported_selected_files: usize,
    pub analyzed_files: usize,
    pub successful_files: usize,
    pub partial_files: usize,
    pub failed_files: usize,
    pub eligible_callables: usize,
    pub measured_callables: usize,
    pub unavailable_callables: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonReviewRankingEntry {
    pub rank: usize,
    pub path: String,
    pub symbol_id: String,
    pub qualified_name: String,
    pub kind: &'static str,
    pub span: JsonSourceSpan,
    pub score: u64,
    pub risk: &'static str,
}

#[derive(Debug, Serialize)]
pub struct JsonReviewFinding {
    pub rule_id: String,
    pub risk: &'static str,
    pub required_action: &'static str,
    pub path: Option<String>,
    pub symbol_id: Option<String>,
    pub qualified_name: Option<String>,
    pub span: Option<JsonSourceSpan>,
    pub observed_value: Option<u64>,
    pub threshold: Option<u64>,
    pub acknowledged: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct JsonSourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start: JsonSourcePosition,
    pub end: JsonSourcePosition,
}

#[derive(Debug, Serialize)]
pub struct JsonSourcePosition {
    pub line: usize,
    pub column: usize,
}

impl AnalysisJsonReport {
    pub fn from_analysis(analysis: &crate::analysis::RepositoryAnalysis) -> Self {
        Self::from_analysis_with_top(analysis, None)
    }

    pub fn from_analysis_with_top(
        analysis: &crate::analysis::RepositoryAnalysis,
        top: Option<usize>,
    ) -> Self {
        let analyzers = analysis
            .analyzers
            .iter()
            .map(|run| JsonAnalyzerRun {
                id: run.descriptor.id.clone(),
                language: run.descriptor.language.as_str().to_owned(),
                version: run.descriptor.version.clone(),
                level: run.descriptor.level.as_str(),
                capabilities: run
                    .descriptor
                    .capabilities
                    .iter()
                    .map(|capability| capability.as_str())
                    .collect(),
                grammar: run.descriptor.grammar.as_ref().map(|grammar| JsonGrammar {
                    name: grammar.name.clone(),
                    version: grammar.version.clone(),
                }),
                queries: run
                    .descriptor
                    .queries
                    .iter()
                    .map(|query| JsonQuery {
                        name: query.name.clone(),
                        version: query.version.clone(),
                    })
                    .collect(),
                limitations: run.descriptor.limitations.clone(),
                counts: JsonAnalysisCounts {
                    analyzed: run.counts.analyzed,
                    successful: run.counts.successful,
                    partial: run.counts.partial,
                    failed: run.counts.failed,
                },
                files: run
                    .files
                    .iter()
                    .map(|file| JsonFileAnalysis {
                        path: json_path(&file.path),
                        status: file.status.as_str(),
                        diagnostics: file
                            .diagnostics
                            .iter()
                            .map(JsonAnalysisDiagnostic::from_diagnostic)
                            .collect(),
                        symbols: file
                            .facts
                            .symbols
                            .iter()
                            .map(JsonSymbol::from_symbol)
                            .collect(),
                        dependencies: file
                            .facts
                            .dependencies
                            .iter()
                            .map(JsonDependencyReference::from_dependency)
                            .collect(),
                        calls: file
                            .facts
                            .calls
                            .iter()
                            .map(JsonCallReference::from_call)
                            .collect(),
                    })
                    .collect(),
            })
            .collect();

        Self {
            report_schema_version: SYNTAX_REPORT_SCHEMA_VERSION,
            tool: JsonTool {
                name: "codegraide",
                version: env!("CARGO_PKG_VERSION"),
            },
            analysis: JsonSyntaxAnalysis {
                kind: "syntax-analysis",
                definition_version: SYNTAX_ANALYSIS_DEFINITION_VERSION,
                selection: JsonAnalysisSelection {
                    target_kind: match analysis.selection.target_kind {
                        crate::analysis::AnalysisTargetKind::Directory => "directory",
                        crate::analysis::AnalysisTargetKind::File => "file",
                    },
                    match_patterns: analysis.selection.match_patterns.clone(),
                    files: analysis
                        .selection
                        .selected_files
                        .iter()
                        .map(|path| json_path(path))
                        .collect(),
                },
            },
            inventory: JsonAnalysisInventory {
                path_encoding: JSON_PATH_ENCODING,
                inventoried_files: analysis.inventoried_files,
                languages: analysis
                    .inventory_languages
                    .iter()
                    .map(|(language, count)| (language.as_str().to_owned(), *count))
                    .collect(),
                inventory_only_languages: analysis
                    .inventory_only_languages
                    .iter()
                    .map(|(language, count)| (language.as_str().to_owned(), *count))
                    .collect(),
            },
            analyzers,
            review: match top {
                Some(limit) => JsonReview::from_review_with_top(&analysis.review, Some(limit)),
                None => JsonReview::from_review(&analysis.review),
            },
            diagnostics: analysis
                .diagnostics
                .iter()
                .map(JsonAnalysisDiagnostic::from_diagnostic)
                .collect(),
        }
    }
}

impl ReviewJsonReport {
    pub fn from_analysis(
        analysis: &crate::analysis::RepositoryAnalysis,
        top: Option<usize>,
    ) -> Self {
        let complete_review = JsonReview::from_review(&analysis.review);
        let review = JsonReview::from_review_with_top(&analysis.review, top);
        Self {
            report_schema_version: REVIEW_REPORT_SCHEMA_VERSION,
            tool: JsonTool {
                name: "codegraide",
                version: env!("CARGO_PKG_VERSION"),
            },
            analysis: JsonReviewAnalysis {
                kind: "review-analysis",
                definition_version: REVIEW_ANALYSIS_DEFINITION_VERSION,
                selected_files: analysis.selection.selected_files.len(),
            },
            ranking_count: complete_review.rankings.len(),
            finding_count: complete_review.findings.len(),
            review,
        }
    }
}

impl GateJsonReport {
    pub fn from_analysis(
        analysis: &crate::analysis::RepositoryAnalysis,
        top: Option<usize>,
    ) -> Self {
        let findings = &analysis.review.findings;
        let top_findings = findings
            .iter()
            .take(top.unwrap_or(10))
            .map(|finding| JsonGateFinding {
                rule_id: finding.rule_id.clone(),
                path: finding.path.as_deref().map(json_path),
                qualified_name: finding.qualified_name.clone(),
                score: finding.observed_value,
                risk: finding.risk.as_str(),
                required_action: finding.required_action.as_str(),
                message: finding.message.clone(),
            })
            .collect();
        Self {
            report_schema_version: GATE_REPORT_SCHEMA_VERSION,
            tool: JsonTool {
                name: "codegraide",
                version: env!("CARGO_PKG_VERSION"),
            },
            status: analysis.review.status.as_str(),
            exit_code: review_status_code(analysis.review.status),
            finding_count: findings.len(),
            top_findings,
        }
    }
}

impl JsonAnalysisDiagnostic {
    fn from_diagnostic(diagnostic: &crate::analyzer::AnalysisDiagnostic) -> Self {
        Self {
            severity: diagnostic.severity.as_str(),
            code: diagnostic.code.clone(),
            message: diagnostic.message.clone(),
            span: diagnostic.span.map(|span| JsonSourceSpan {
                start_byte: span.start_byte,
                end_byte: span.end_byte,
                start: JsonSourcePosition {
                    line: span.start.line,
                    column: span.start.column,
                },
                end: JsonSourcePosition {
                    line: span.end.line,
                    column: span.end.column,
                },
            }),
        }
    }
}

impl JsonSymbol {
    fn from_symbol(symbol: &crate::analyzer::Symbol) -> Self {
        Self {
            id: symbol.id.as_str().to_owned(),
            parent_id: symbol.parent_id.as_ref().map(|id| id.as_str().to_owned()),
            kind: symbol.kind.as_str(),
            name: symbol.name.clone(),
            qualified_name: symbol.qualified_name.clone(),
            span: JsonSourceSpan::from_span(symbol.span),
            body_span: symbol.body_span.map(JsonSourceSpan::from_span),
            name_span: symbol.name_span.map(JsonSourceSpan::from_span),
            completeness: symbol.completeness.as_str(),
            modifiers: symbol
                .modifiers
                .iter()
                .map(|modifier| modifier.as_str())
                .collect(),
            parameters: symbol
                .parameters
                .iter()
                .map(JsonParameter::from_parameter)
                .collect(),
            decorators: symbol
                .decorators
                .iter()
                .map(JsonDecorator::from_decorator)
                .collect(),
            nesting_events: symbol
                .nesting_events
                .iter()
                .map(JsonNestingEvent::from_event)
                .collect(),
            decision_events: symbol
                .decision_events
                .iter()
                .map(JsonDecisionEvent::from_event)
                .collect(),
            measurements: symbol
                .measurements
                .iter()
                .map(JsonMeasurement::from_measurement)
                .collect(),
        }
    }
}

impl JsonDecisionEvent {
    fn from_event(event: &crate::analyzer::DecisionEvent) -> Self {
        Self {
            kind: event.kind.as_str(),
            span: JsonSourceSpan::from_span(event.span),
        }
    }
}

impl JsonReview {
    fn from_review(review: &ReviewEvaluation) -> Self {
        Self::from_review_with_top(review, None)
    }

    fn from_review_with_top(review: &ReviewEvaluation, top: Option<usize>) -> Self {
        let mut output = Self {
            status: review.status.as_str(),
            policy: JsonReviewPolicy {
                definition_version: review.policy.definition_version,
                sources: review.policy.sources.clone(),
                risk_bands: JsonRiskBands {
                    moderate_at: review.policy.risk_bands.moderate_at,
                    high_at: review.policy.risk_bands.high_at,
                    critical_at: review.policy.risk_bands.critical_at,
                },
                complexity_review_at: review.policy.complexity_review_at,
                complexity_block_at: review.policy.complexity_block_at,
                exceptions: review
                    .policy
                    .exceptions
                    .iter()
                    .map(|exception| JsonReviewException {
                        symbol_id: exception.symbol_id.clone(),
                        reason: exception.reason.clone(),
                        approved_max: exception.approved_max,
                        unbounded: exception.unbounded,
                    })
                    .collect(),
            },
            coverage: JsonReviewCoverage {
                selected_language_files: review.coverage.selected_language_files,
                unsupported_selected_files: review.coverage.unsupported_selected_files,
                analyzed_files: review.coverage.analyzed_files,
                successful_files: review.coverage.successful_files,
                partial_files: review.coverage.partial_files,
                failed_files: review.coverage.failed_files,
                eligible_callables: review.coverage.eligible_callables,
                measured_callables: review.coverage.measured_callables,
                unavailable_callables: review.coverage.unavailable_callables,
            },
            rankings: review
                .rankings
                .iter()
                .map(|entry| JsonReviewRankingEntry {
                    rank: entry.rank,
                    path: json_path(&entry.path),
                    symbol_id: entry.symbol_id.clone(),
                    qualified_name: entry.qualified_name.clone(),
                    kind: entry.kind.as_str(),
                    span: JsonSourceSpan::from_span(entry.span),
                    score: entry.score,
                    risk: entry.risk.as_str(),
                })
                .collect(),
            findings: review
                .findings
                .iter()
                .map(|finding| JsonReviewFinding {
                    rule_id: finding.rule_id.clone(),
                    risk: finding.risk.as_str(),
                    required_action: finding.required_action.as_str(),
                    path: finding.path.as_deref().map(json_path),
                    symbol_id: finding.symbol_id.clone(),
                    qualified_name: finding.qualified_name.clone(),
                    span: finding.span.map(JsonSourceSpan::from_span),
                    observed_value: finding.observed_value,
                    threshold: finding.threshold,
                    acknowledged: finding.acknowledged,
                    message: finding.message.clone(),
                })
                .collect(),
        };
        if let Some(limit) = top {
            output.rankings.truncate(limit);
            output.findings.truncate(limit);
        }
        output
    }
}

impl JsonParameter {
    fn from_parameter(parameter: &crate::analyzer::Parameter) -> Self {
        Self {
            name: parameter.name.clone(),
            kind: parameter.kind.as_str(),
            span: JsonSourceSpan::from_span(parameter.span),
            has_default: parameter.has_default,
            has_annotation: parameter.has_annotation,
        }
    }
}

impl JsonDecorator {
    fn from_decorator(decorator: &crate::analyzer::Decorator) -> Self {
        Self {
            expression: decorator.expression.clone(),
            span: JsonSourceSpan::from_span(decorator.span),
        }
    }
}

impl JsonNestingEvent {
    fn from_event(event: &crate::analyzer::NestingEvent) -> Self {
        Self {
            kind: event.kind.as_str(),
            depth: event.depth,
            span: JsonSourceSpan::from_span(event.span),
        }
    }
}

impl JsonMeasurement {
    fn from_measurement(measurement: &crate::analyzer::Measurement) -> Self {
        Self {
            id: measurement.id.clone(),
            definition_version: measurement.definition_version.clone(),
            unit: measurement.unit.clone(),
            status: measurement.status.as_str(),
            value: measurement.value,
            reason: measurement.reason.clone(),
        }
    }
}

impl JsonDependencyReference {
    fn from_dependency(dependency: &crate::analyzer::DependencyReference) -> Self {
        Self {
            kind: dependency.kind.as_str(),
            module: dependency.module.clone(),
            imported_name: dependency.imported_name.clone(),
            alias: dependency.alias.clone(),
            relative_level: dependency.relative_level,
            wildcard: dependency.wildcard,
            resolution: dependency.resolution.as_str(),
            enclosing_symbol: dependency
                .enclosing_symbol
                .as_ref()
                .map(|id| id.as_str().to_owned()),
            scope: dependency.context.scope.as_str(),
            usage: dependency.context.usage.as_str(),
            requirement: dependency.context.requirement.as_str(),
            conditional: dependency.context.conditional,
            span: JsonSourceSpan::from_span(dependency.span),
        }
    }
}

impl JsonCallReference {
    fn from_call(call: &crate::analyzer::CallReference) -> Self {
        Self {
            callee: call.callee.clone(),
            components: call.components.clone(),
            enclosing_symbol: call
                .enclosing_symbol
                .as_ref()
                .map(|id| id.as_str().to_owned()),
            positional_arguments: call.arguments.positional,
            keyword_arguments: call.arguments.keywords.clone(),
            has_star_args: call.arguments.has_star_args,
            has_star_kwargs: call.arguments.has_star_kwargs,
            syntax_complete: call.syntax_complete,
            span: JsonSourceSpan::from_span(call.span),
        }
    }
}

impl JsonSourceSpan {
    fn from_span(span: crate::analyzer::SourceSpan) -> Self {
        Self {
            start_byte: span.start_byte,
            end_byte: span.end_byte,
            start: JsonSourcePosition {
                line: span.start.line,
                column: span.start.column,
            },
            end: JsonSourcePosition {
                line: span.end.line,
                column: span.end.column,
            },
        }
    }
}

impl InventoryJsonReport {
    pub fn from_inventory(inventory: &RepositoryInventory) -> Self {
        let categories = FileCategory::ALL
            .into_iter()
            .map(|category| {
                let files = inventory
                    .category_files(category)
                    .iter()
                    .map(|path| json_path(path))
                    .collect::<Vec<_>>();
                (
                    category.as_str().to_owned(),
                    JsonCategory {
                        count: files.len(),
                        files,
                    },
                )
            })
            .collect();

        let languages = inventory
            .files_by_language
            .iter()
            .map(|(language, count)| (language.as_str().to_owned(), *count))
            .collect();
        let uncategorized_extensions = inventory
            .uncategorized_files_by_extension
            .iter()
            .map(|(extension, count)| (extension.as_str().to_owned(), *count))
            .collect();

        Self {
            report_schema_version: INVENTORY_REPORT_SCHEMA_VERSION,
            tool: JsonTool {
                name: "codegraide",
                version: env!("CARGO_PKG_VERSION"),
            },
            analysis: JsonAnalysis {
                kind: "inventory",
                definition_version: INVENTORY_REPORT_DEFINITION_VERSION,
            },
            inventory: JsonInventory {
                path_encoding: JSON_PATH_ENCODING,
                inventoried_files: inventory.inventoried_files,
                categories,
                languages,
                uncategorized_extensions,
                ignored: JsonIgnored {
                    exact: inventory.ignored.exact,
                    files: inventory
                        .ignored
                        .files
                        .iter()
                        .map(|path| json_path(path))
                        .collect(),
                    directories: inventory
                        .ignored
                        .directories
                        .iter()
                        .map(|path| json_path(path))
                        .collect(),
                    builtin_directories: inventory
                        .ignored
                        .builtin_directories
                        .iter()
                        .map(|path| json_path(path))
                        .collect(),
                },
                included_ignored_files: inventory.num_included_ignored_files,
                line_counts: JsonLineCounts::from_counts(&inventory.line_counts),
            },
            diagnostics: inventory
                .diagnostics
                .iter()
                .map(|diagnostic| JsonDiagnostic {
                    severity: "warning",
                    code: diagnostic.code,
                    message: diagnostic.message.clone(),
                })
                .collect(),
        }
    }
}

impl JsonLineCounts {
    fn from_counts(counts: &RepositoryLineCounts) -> Self {
        Self {
            definition_version: PHYSICAL_LINE_DEFINITION_VERSION,
            total: JsonLineCountValues::from_counts(&counts.total),
            by_language: counts
                .by_language
                .iter()
                .map(|(language, values)| {
                    (
                        language.as_str().to_owned(),
                        JsonLineCountValues::from_counts(values),
                    )
                })
                .collect(),
        }
    }
}

impl JsonLineCountValues {
    fn from_counts(counts: &crate::lines::LineCounts) -> Self {
        Self {
            files: counts.files,
            total_lines: counts.total,
            source_lines: counts.source,
            comment_lines: counts.comment,
            blank_lines: counts.blank,
        }
    }
}

pub(crate) fn json_path(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        percent_encode_path(path.as_os_str().as_bytes())
    }

    #[cfg(not(unix))]
    {
        normalize_relative_path(path)
    }
}

#[cfg(unix)]
fn percent_encode_path(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'/' {
            output.push('/');
            index += 1;
            continue;
        }

        if bytes[index] == b'%' || bytes[index] == b'\\' {
            push_percent_encoded(&mut output, bytes[index]);
            index += 1;
            continue;
        }

        match std::str::from_utf8(&bytes[index..]) {
            Ok(valid) => {
                for character in valid.chars() {
                    if character == '%' || character == '\\' {
                        push_percent_encoded(&mut output, character as u8);
                    } else {
                        output.push(character);
                    }
                }
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    let valid = std::str::from_utf8(&bytes[index..index + valid_up_to])
                        .expect("UTF-8 prefix should be valid");
                    for character in valid.chars() {
                        if character == '%' || character == '\\' {
                            push_percent_encoded(&mut output, character as u8);
                        } else {
                            output.push(character);
                        }
                    }
                    index += valid_up_to;
                } else {
                    push_percent_encoded(&mut output, bytes[index]);
                    index += 1;
                }
            }
        }
    }

    output
}

#[cfg(unix)]
fn push_percent_encoded(output: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    output.push('%');
    output.push(HEX[(byte >> 4) as usize] as char);
    output.push(HEX[(byte & 0x0F) as usize] as char);
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    use super::json_path;

    #[test]
    fn json_paths_encode_non_utf8_and_percent_bytes_losslessly() {
        let path = PathBuf::from(OsString::from_vec(b"dir/mod%\\\xFF.rs".to_vec()));

        assert_eq!(json_path(&path), "dir/mod%25%5C%FF.rs");
    }
}

#[derive(Debug, Default)]
pub struct IgnoredInventory {
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
    pub builtin_directories: Vec<PathBuf>,
    pub exact: bool,
}

impl IgnoredInventory {
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn directory_count(&self) -> usize {
        self.directories.len()
    }

    pub fn builtin_directory_count(&self) -> usize {
        self.builtin_directories.len()
    }
}

#[derive(Debug)]
pub struct RepositoryInventory {
    pub inventoried_files: usize,
    pub files_by_category: BTreeMap<FileCategory, Vec<PathBuf>>,
    pub files_by_language: BTreeMap<LanguageId, usize>,
    pub uncategorized_files_by_extension: BTreeMap<ExtensionId, usize>,
    pub line_counts: RepositoryLineCounts,
    pub ignored: IgnoredInventory,
    pub num_included_ignored_files: usize,
    pub diagnostics: Vec<InventoryDiagnostic>,
}

impl Default for RepositoryInventory {
    fn default() -> Self {
        let files_by_category = FileCategory::ALL
            .into_iter()
            .map(|category| (category, Vec::new()))
            .collect();

        Self {
            inventoried_files: 0,
            files_by_category,
            files_by_language: BTreeMap::new(),
            uncategorized_files_by_extension: BTreeMap::new(),
            line_counts: RepositoryLineCounts::default(),
            ignored: IgnoredInventory::default(),
            num_included_ignored_files: 0,
            diagnostics: Vec::new(),
        }
    }
}

impl RepositoryInventory {
    pub fn category_files(&self, category: FileCategory) -> &[PathBuf] {
        self.files_by_category
            .get(&category)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn category_count(&self, category: FileCategory) -> usize {
        self.category_files(category).len()
    }

    pub fn source_files(&self) -> usize {
        self.category_count(FileCategory::Source)
    }

    pub fn uncategorized_files(&self) -> usize {
        self.category_count(FileCategory::Uncategorized)
    }

    pub(crate) fn record_category_path(&mut self, category: FileCategory, path: PathBuf) {
        self.files_by_category
            .get_mut(&category)
            .expect("all fixed categories are initialized")
            .push(path);
    }

    pub(crate) fn validate_invariants(&self) {
        debug_assert_eq!(
            self.inventoried_files,
            self.files_by_category.values().map(Vec::len).sum::<usize>()
        );
        debug_assert_eq!(
            self.uncategorized_files(),
            self.uncategorized_files_by_extension
                .values()
                .sum::<usize>()
        );
        debug_assert_eq!(
            self.line_counts.total.source
                + self.line_counts.total.comment
                + self.line_counts.total.blank,
            self.line_counts.total.total
        );
        debug_assert_eq!(
            self.line_counts.total.files,
            self.line_counts
                .by_language
                .values()
                .map(|counts| counts.files)
                .sum::<usize>()
        );
        for counts in self.line_counts.by_language.values() {
            debug_assert_eq!(counts.source + counts.comment + counts.blank, counts.total);
        }
        for paths in self.files_by_category.values() {
            debug_assert!(paths.windows(2).all(|pair| pair[0] <= pair[1]));
            debug_assert!(paths.iter().all(|path| !path.is_absolute()));
        }
    }
}

pub(crate) fn normalize_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
