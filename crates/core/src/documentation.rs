use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::analysis::AnalyzerRun;
use crate::analyzer::{
    AnalyzerCapability, DocumentationStatus, FileAnalysisStatus, SourceSpan, SymbolKind,
};
use crate::inventory::detect_language;

pub const DOCUMENTATION_COVERAGE_DEFINITION_VERSION: &str = "documentation-coverage-v1";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct DocumentationDefinition {
    pub metric_id: &'static str,
    pub version: &'static str,
    pub display_name: &'static str,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct DocumentationEligibility {
    pub is_test: bool,
    pub symbols: BTreeSet<crate::SymbolId>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum DocumentationCoverageStatus {
    Disabled,
    NotApplicable,
    Complete,
    Partial,
}

impl DocumentationCoverageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NotApplicable => "not-applicable",
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct DocumentationCounts {
    pub eligible: usize,
    pub documented: usize,
    pub missing: usize,
    pub unavailable: usize,
}

impl DocumentationCounts {
    pub fn measured(self) -> usize {
        self.documented + self.missing
    }

    pub fn coverage_basis_points(self) -> Option<u16> {
        let measured = self.measured();
        (measured > 0).then(|| ((self.documented * 10_000) / measured) as u16)
    }

    fn record(&mut self, status: DocumentationStatus) {
        self.eligible += 1;
        match status {
            DocumentationStatus::Documented => self.documented += 1,
            DocumentationStatus::Missing => self.missing += 1,
            DocumentationStatus::Unavailable => self.unavailable += 1,
        }
    }

    fn add(&mut self, other: Self) {
        self.eligible += other.eligible;
        self.documented += other.documented;
        self.missing += other.missing;
        self.unavailable += other.unavailable;
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentationSymbol {
    pub path: PathBuf,
    pub symbol_id: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub span: SourceSpan,
    pub docstring_span: Option<SourceSpan>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentationFileCoverage {
    pub path: PathBuf,
    pub status: FileAnalysisStatus,
    pub counts: DocumentationCounts,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentationCoverage {
    pub definition: Option<DocumentationDefinition>,
    pub language: Option<crate::LanguageId>,
    pub definition_version: &'static str,
    pub status: DocumentationCoverageStatus,
    pub applicable_files: usize,
    pub skipped_test_files: usize,
    pub unsupported_selected_files: usize,
    pub counts: DocumentationCounts,
    pub by_kind: BTreeMap<SymbolKind, DocumentationCounts>,
    pub files: Vec<DocumentationFileCoverage>,
    pub missing_symbols: Vec<DocumentationSymbol>,
    pub unavailable_symbols: Vec<DocumentationSymbol>,
}

impl DocumentationCoverage {
    pub fn disabled(_selected_files: &[PathBuf]) -> Self {
        Self {
            definition: None,
            language: None,
            definition_version: DOCUMENTATION_COVERAGE_DEFINITION_VERSION,
            status: DocumentationCoverageStatus::Disabled,
            applicable_files: 0,
            skipped_test_files: 0,
            unsupported_selected_files: 0,
            counts: DocumentationCounts::default(),
            by_kind: BTreeMap::new(),
            files: Vec::new(),
            missing_symbols: Vec::new(),
            unavailable_symbols: Vec::new(),
        }
    }

    pub fn threshold_is_met(&self, threshold_percent: u8) -> Option<bool> {
        if self.status != DocumentationCoverageStatus::Complete
            || self.counts.measured() == 0
            || self.counts.unavailable > 0
        {
            return None;
        }
        Some(
            self.counts.documented * 100 >= usize::from(threshold_percent) * self.counts.measured(),
        )
    }
}

pub fn evaluate_documentation_coverage(
    enabled: bool,
    include_tests: bool,
    selected_files: &[PathBuf],
    analyzers: &[AnalyzerRun],
) -> DocumentationCoverage {
    if !enabled {
        return DocumentationCoverage::disabled(selected_files);
    }

    let documentation_languages = analyzers
        .iter()
        .filter(|run| {
            run.descriptor
                .capabilities
                .contains(&AnalyzerCapability::Documentation)
        })
        .map(|run| run.descriptor.language.clone())
        .collect::<BTreeSet<_>>();
    let unsupported_selected_files = selected_files
        .iter()
        .filter(|path| {
            detect_language(path)
                .is_some_and(|language| !documentation_languages.contains(&language))
        })
        .count();

    let definitions = analyzers
        .iter()
        .filter(|run| {
            run.descriptor
                .capabilities
                .contains(&AnalyzerCapability::Documentation)
        })
        .filter_map(|run| {
            run.descriptor
                .documentation
                .map(|definition| (definition, run.descriptor.language.clone()))
        })
        .collect::<Vec<_>>();
    let sole_definition = (definitions.len() == 1).then(|| definitions[0].clone());
    let mut coverage = DocumentationCoverage {
        definition: sole_definition.as_ref().map(|(definition, _)| *definition),
        language: sole_definition
            .as_ref()
            .map(|(_, language)| language.clone()),
        definition_version: sole_definition.as_ref().map_or(
            DOCUMENTATION_COVERAGE_DEFINITION_VERSION,
            |(definition, _)| definition.version,
        ),
        status: DocumentationCoverageStatus::NotApplicable,
        applicable_files: 0,
        skipped_test_files: 0,
        unsupported_selected_files,
        counts: DocumentationCounts::default(),
        by_kind: BTreeMap::new(),
        files: Vec::new(),
        missing_symbols: Vec::new(),
        unavailable_symbols: Vec::new(),
    };
    let mut incomplete = definitions.len() != 1;

    for run in analyzers.iter().filter(|run| {
        run.descriptor
            .capabilities
            .contains(&AnalyzerCapability::Documentation)
    }) {
        for file in &run.files {
            let Some(eligibility) = &file.facts.documentation_eligibility else {
                coverage.applicable_files += 1;
                incomplete = true;
                continue;
            };
            if !include_tests && eligibility.is_test {
                coverage.skipped_test_files += 1;
                continue;
            }
            coverage.applicable_files += 1;
            incomplete |= file.status != FileAnalysisStatus::Successful;
            let mut file_counts = DocumentationCounts::default();
            for symbol in file
                .facts
                .symbols
                .iter()
                .filter(|symbol| eligibility.symbols.contains(&symbol.id))
            {
                let documentation = symbol.documentation.as_ref();
                let status = documentation
                    .map(|documentation| documentation.status)
                    .unwrap_or(DocumentationStatus::Unavailable);
                file_counts.record(status);
                coverage
                    .by_kind
                    .entry(symbol.kind)
                    .or_default()
                    .record(status);
                let evidence = DocumentationSymbol {
                    path: file.path.clone(),
                    symbol_id: symbol.id.as_str().to_owned(),
                    qualified_name: symbol.qualified_name.clone(),
                    kind: symbol.kind,
                    span: symbol.span,
                    docstring_span: documentation.and_then(|documentation| documentation.span),
                    reason: documentation
                        .and_then(|documentation| documentation.reason.clone())
                        .or_else(|| {
                            documentation
                                .is_none()
                                .then(|| "documentation fact is unavailable".to_owned())
                        }),
                };
                match status {
                    DocumentationStatus::Missing => coverage.missing_symbols.push(evidence),
                    DocumentationStatus::Unavailable => {
                        incomplete = true;
                        coverage.unavailable_symbols.push(evidence);
                    }
                    DocumentationStatus::Documented => {}
                }
            }
            // An invalid eligibility ID must never make an incomplete metric pass.
            incomplete |= file_counts.eligible != eligibility.symbols.len();
            coverage.counts.add(file_counts);
            coverage.files.push(DocumentationFileCoverage {
                path: file.path.clone(),
                status: file.status,
                counts: file_counts,
            });
        }
    }

    coverage.status = if coverage.applicable_files == 0 {
        DocumentationCoverageStatus::NotApplicable
    } else if incomplete {
        DocumentationCoverageStatus::Partial
    } else {
        DocumentationCoverageStatus::Complete
    };
    coverage
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    coverage.missing_symbols.sort_by(documentation_symbol_order);
    coverage
        .unavailable_symbols
        .sort_by(documentation_symbol_order);
    coverage
}

fn documentation_symbol_order(
    left: &DocumentationSymbol,
    right: &DocumentationSymbol,
) -> std::cmp::Ordering {
    left.path
        .cmp(&right.path)
        .then_with(|| left.span.start_byte.cmp(&right.span.start_byte))
        .then_with(|| left.symbol_id.cmp(&right.symbol_id))
}
