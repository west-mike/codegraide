use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::analysis::AnalyzerRun;
use crate::analyzer::{
    AnalyzerCapability, DocumentationStatus, FileAnalysisStatus, SourceSpan, Symbol, SymbolId,
    SymbolKind,
};
use crate::inventory::detect_language;

pub const PYTHON_DOCUMENTATION_COVERAGE_DEFINITION_VERSION: &str =
    "python-documentation-coverage-v1";

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
            definition_version: PYTHON_DOCUMENTATION_COVERAGE_DEFINITION_VERSION,
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

    let mut coverage = DocumentationCoverage {
        definition_version: PYTHON_DOCUMENTATION_COVERAGE_DEFINITION_VERSION,
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
    let mut incomplete = false;

    for run in analyzers.iter().filter(|run| {
        run.descriptor
            .capabilities
            .contains(&AnalyzerCapability::Documentation)
    }) {
        for file in &run.files {
            if !include_tests && is_conventional_python_test(&file.path) {
                coverage.skipped_test_files += 1;
                continue;
            }
            coverage.applicable_files += 1;
            incomplete |= file.status != FileAnalysisStatus::Successful;
            let symbols = file
                .facts
                .symbols
                .iter()
                .map(|symbol| (symbol.id.clone(), symbol))
                .collect::<BTreeMap<_, _>>();
            let mut file_counts = DocumentationCounts::default();
            for symbol in eligible_documentation_symbols(&file.facts.symbols, &symbols) {
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

fn is_eligible(symbol: &Symbol, symbols: &BTreeMap<SymbolId, &Symbol>) -> bool {
    match symbol.kind {
        SymbolKind::Module => symbol.parent_id.is_none(),
        SymbolKind::Class | SymbolKind::Function => {
            symbol.direct_declaration
                && symbol
                    .parent_id
                    .as_ref()
                    .and_then(|id| symbols.get(id))
                    .is_some_and(|parent| parent.kind == SymbolKind::Module)
        }
        SymbolKind::Method => {
            symbol.direct_declaration
                && symbol
                    .parent_id
                    .as_ref()
                    .and_then(|id| symbols.get(id))
                    .filter(|parent| parent.kind == SymbolKind::Class && parent.direct_declaration)
                    .and_then(|class| class.parent_id.as_ref())
                    .and_then(|id| symbols.get(id))
                    .is_some_and(|parent| parent.kind == SymbolKind::Module)
        }
        SymbolKind::Lambda => false,
    }
}

type DocumentationGroupKey = (Option<SymbolId>, SymbolKind, String);

fn eligible_documentation_symbols<'a>(
    all_symbols: &'a [Symbol],
    symbols: &BTreeMap<SymbolId, &'a Symbol>,
) -> Vec<&'a Symbol> {
    let eligible = all_symbols
        .iter()
        .filter(|symbol| is_eligible(symbol, symbols))
        .collect::<Vec<_>>();
    let overload_keys = eligible
        .iter()
        .filter(|symbol| is_overload(symbol))
        .map(|symbol| documentation_group_key(symbol))
        .collect::<BTreeSet<_>>();
    let mut emitted_overloads = BTreeSet::new();
    let mut result = Vec::new();

    for symbol in &eligible {
        let key = documentation_group_key(symbol);
        if !overload_keys.contains(&key) {
            result.push(*symbol);
            continue;
        }
        if !emitted_overloads.insert(key.clone()) {
            continue;
        }
        let group = eligible
            .iter()
            .copied()
            .filter(|candidate| documentation_group_key(candidate) == key)
            .collect::<Vec<_>>();
        result.push(overload_group_representative(&group));
    }
    result
}

fn documentation_group_key(symbol: &Symbol) -> DocumentationGroupKey {
    (
        symbol.parent_id.clone(),
        symbol.kind,
        symbol.qualified_name.clone(),
    )
}

fn is_overload(symbol: &Symbol) -> bool {
    symbol.decorators.iter().any(|decorator| {
        decorator
            .expression
            .trim()
            .rsplit('.')
            .next()
            .is_some_and(|name| name == "overload")
    })
}

fn overload_group_representative<'a>(group: &[&'a Symbol]) -> &'a Symbol {
    group
        .iter()
        .copied()
        .find(|symbol| !is_overload(symbol))
        .or_else(|| {
            group.iter().copied().find(|symbol| {
                symbol.documentation.as_ref().is_some_and(|documentation| {
                    documentation.status == DocumentationStatus::Documented
                })
            })
        })
        .or_else(|| {
            group.iter().copied().find(|symbol| {
                symbol.documentation.as_ref().is_some_and(|documentation| {
                    documentation.status == DocumentationStatus::Unavailable
                })
            })
        })
        .unwrap_or(group[0])
}

pub fn is_conventional_python_test(path: &std::path::Path) -> bool {
    if path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .any(|component| matches!(component, "test" | "tests"))
    {
        return true;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == "conftest.py" || name.starts_with("test_") || name.ends_with("_test.py")
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::LanguageAnalysisCounts;
    use crate::analyzer::{
        AnalysisFacts, AnalysisLevel, AnalyzerDescriptor, Decorator, FileAnalysis,
        SymbolCompleteness, SymbolDocumentation,
    };
    use crate::report::LanguageId;

    fn span(line: usize) -> SourceSpan {
        SourceSpan {
            start_byte: line,
            end_byte: line + 1,
            start: crate::analyzer::SourcePosition { line, column: 1 },
            end: crate::analyzer::SourcePosition { line, column: 2 },
        }
    }

    fn symbol(
        id: &str,
        parent: Option<&str>,
        kind: SymbolKind,
        status: Option<DocumentationStatus>,
        line: usize,
    ) -> Symbol {
        Symbol {
            id: SymbolId::new(id),
            parent_id: parent.map(SymbolId::new),
            kind,
            direct_declaration: true,
            name: id.to_owned(),
            qualified_name: id.to_owned(),
            span: span(line),
            body_span: Some(span(line)),
            name_span: None,
            completeness: SymbolCompleteness::Complete,
            modifiers: BTreeSet::new(),
            parameters: Vec::new(),
            decorators: Vec::new(),
            documentation: status.map(|status| SymbolDocumentation {
                status,
                span: (status != DocumentationStatus::Unavailable).then(|| span(line)),
                reason: (status == DocumentationStatus::Unavailable)
                    .then(|| "parser recovery".to_owned()),
            }),
            nesting_events: Vec::new(),
            decision_events: Vec::new(),
            measurements: Vec::new(),
        }
    }

    fn analyzer_run(file_status: FileAnalysisStatus) -> AnalyzerRun {
        AnalyzerRun {
            descriptor: AnalyzerDescriptor {
                id: "python-test".to_owned(),
                language: LanguageId::new("python"),
                version: "0.2.0".to_owned(),
                level: AnalysisLevel::Syntax,
                capabilities: [AnalyzerCapability::Documentation].into_iter().collect(),
                grammar: None,
                queries: Vec::new(),
                limitations: Vec::new(),
            },
            counts: LanguageAnalysisCounts {
                analyzed: 1,
                successful: usize::from(file_status == FileAnalysisStatus::Successful),
                partial: usize::from(file_status == FileAnalysisStatus::Partial),
                failed: usize::from(file_status == FileAnalysisStatus::Failed),
            },
            files: vec![FileAnalysis {
                path: PathBuf::from("src/example.py"),
                status: file_status,
                diagnostics: Vec::new(),
                facts: AnalysisFacts {
                    symbols: vec![
                        symbol(
                            "module",
                            None,
                            SymbolKind::Module,
                            Some(DocumentationStatus::Documented),
                            1,
                        ),
                        symbol(
                            "Service",
                            Some("module"),
                            SymbolKind::Class,
                            Some(DocumentationStatus::Missing),
                            2,
                        ),
                        symbol(
                            "Service.run",
                            Some("Service"),
                            SymbolKind::Method,
                            Some(DocumentationStatus::Documented),
                            3,
                        ),
                        symbol(
                            "top",
                            Some("module"),
                            SymbolKind::Function,
                            Some(DocumentationStatus::Missing),
                            4,
                        ),
                        symbol(
                            "top.nested",
                            Some("top"),
                            SymbolKind::Function,
                            Some(DocumentationStatus::Missing),
                            5,
                        ),
                        symbol(
                            "Service.Nested",
                            Some("Service"),
                            SymbolKind::Class,
                            Some(DocumentationStatus::Missing),
                            6,
                        ),
                        symbol("lambda", Some("top"), SymbolKind::Lambda, None, 7),
                    ],
                    dependencies: Vec::new(),
                    calls: Vec::new(),
                },
            }],
        }
    }

    #[test]
    fn coverage_includes_only_top_level_symbols_and_direct_methods() {
        let coverage = evaluate_documentation_coverage(
            true,
            false,
            &[
                PathBuf::from("src/example.py"),
                PathBuf::from("src/main.rs"),
            ],
            &[analyzer_run(FileAnalysisStatus::Successful)],
        );

        assert_eq!(coverage.status, DocumentationCoverageStatus::Complete);
        assert_eq!(coverage.applicable_files, 1);
        assert_eq!(coverage.unsupported_selected_files, 1);
        assert_eq!(coverage.counts.eligible, 4);
        assert_eq!(coverage.counts.documented, 2);
        assert_eq!(coverage.counts.missing, 2);
        assert_eq!(coverage.counts.coverage_basis_points(), Some(5_000));
        assert_eq!(coverage.missing_symbols.len(), 2);
        assert!(coverage.threshold_is_met(50).unwrap());
        assert!(!coverage.threshold_is_met(51).unwrap());
    }

    #[test]
    fn exact_basis_points_and_partial_thresholds_do_not_optimistically_pass() {
        let counts = DocumentationCounts {
            eligible: 3,
            documented: 2,
            missing: 1,
            unavailable: 0,
        };
        assert_eq!(counts.coverage_basis_points(), Some(6_666));

        let mut coverage = evaluate_documentation_coverage(
            true,
            false,
            &[PathBuf::from("src/example.py")],
            &[analyzer_run(FileAnalysisStatus::Partial)],
        );
        assert_eq!(coverage.status, DocumentationCoverageStatus::Partial);
        assert_eq!(coverage.threshold_is_met(1), None);
        coverage.status = DocumentationCoverageStatus::Complete;
        assert_eq!(coverage.threshold_is_met(50), Some(true));
    }

    #[test]
    fn overload_declarations_count_as_one_logical_symbol() {
        let mut run = analyzer_run(FileAnalysisStatus::Successful);
        for (ordinal, status) in [
            DocumentationStatus::Missing,
            DocumentationStatus::Missing,
            DocumentationStatus::Missing,
        ]
        .into_iter()
        .enumerate()
        {
            let mut overload = symbol(
                &format!("parse-overload-{ordinal}"),
                Some("module"),
                SymbolKind::Function,
                Some(status),
                10 + ordinal,
            );
            overload.qualified_name = "parse".to_owned();
            overload.decorators.push(Decorator {
                expression: "typing.overload".to_owned(),
                span: span(10 + ordinal),
            });
            run.files[0].facts.symbols.push(overload);
        }
        let mut implementation = symbol(
            "parse-implementation",
            Some("module"),
            SymbolKind::Function,
            Some(DocumentationStatus::Documented),
            20,
        );
        implementation.qualified_name = "parse".to_owned();
        run.files[0].facts.symbols.push(implementation);

        let coverage = evaluate_documentation_coverage(
            true,
            false,
            &[PathBuf::from("src/example.py")],
            &[run],
        );

        assert_eq!(coverage.counts.eligible, 5);
        assert_eq!(coverage.counts.documented, 3);
        assert_eq!(coverage.counts.missing, 2);
        assert!(
            coverage
                .missing_symbols
                .iter()
                .all(|symbol| symbol.qualified_name != "parse")
        );
    }

    #[test]
    fn conventional_test_files_are_skipped_unless_included() {
        let mut run = analyzer_run(FileAnalysisStatus::Successful);
        run.files[0].path = PathBuf::from("tests/test_example.py");

        let skipped = evaluate_documentation_coverage(
            true,
            false,
            &[PathBuf::from("tests/test_example.py")],
            &[run.clone()],
        );
        assert_eq!(skipped.status, DocumentationCoverageStatus::NotApplicable);
        assert_eq!(skipped.applicable_files, 0);
        assert_eq!(skipped.skipped_test_files, 1);
        assert_eq!(skipped.counts, DocumentationCounts::default());

        let included = evaluate_documentation_coverage(
            true,
            true,
            &[PathBuf::from("tests/test_example.py")],
            &[run],
        );
        assert_eq!(included.status, DocumentationCoverageStatus::Complete);
        assert_eq!(included.applicable_files, 1);
        assert_eq!(included.skipped_test_files, 0);
        assert_eq!(included.counts.eligible, 4);
    }

    #[test]
    fn recognizes_conventional_python_test_paths() {
        for path in [
            "tests/service.py",
            "test/service.py",
            "src/test_service.py",
            "src/service_test.py",
            "src/conftest.py",
        ] {
            assert!(is_conventional_python_test(std::path::Path::new(path)));
        }
        assert!(!is_conventional_python_test(std::path::Path::new(
            "src/service.py"
        )));
    }
}
