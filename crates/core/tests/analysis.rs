use std::path::Path;

use codegraide_core::{
    AnalysisInput, AnalysisLevel, AnalysisOptions, AnalyzerCapability, AnalyzerDescriptor,
    AnalyzerRegistry, DiagnosticSeverity, FileAnalysis, FileAnalysisStatus, LanguageAnalyzer,
    LanguageId, analyze_repository,
};
use tempfile::tempdir;

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
            diagnostics: vec![codegraide_core::AnalysisDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "stub".to_owned(),
                message: "test diagnostic".to_owned(),
                span: None,
            }],
            facts: codegraide_core::AnalysisFacts::default(),
        }
    }
}

fn python_stub() -> AnalyzerRegistry {
    let mut registry = AnalyzerRegistry::new();
    registry
        .register(Box::new(StubAnalyzer {
            descriptor: AnalyzerDescriptor {
                id: "test-python".to_owned(),
                language: LanguageId::new("python"),
                version: "0.1.0".to_owned(),
                level: AnalysisLevel::Syntax,
                capabilities: [AnalyzerCapability::Parse].into_iter().collect(),
                grammar: None,
                queries: Vec::new(),
                limitations: Vec::new(),
            },
        }))
        .expect("stub registration should succeed");
    registry
}

#[test]
fn directory_analysis_selects_full_match_paths_and_preserves_inventory_only_languages() {
    let repository = tempdir().expect("temporary repository should be created");
    std::fs::create_dir(repository.path().join("src")).expect("source directory");
    std::fs::write(repository.path().join("src/main.py"), "print('main')\n")
        .expect("Python fixture");
    std::fs::write(repository.path().join("src/test.py"), "print('test')\n")
        .expect("Python fixture");
    std::fs::write(repository.path().join("src/main.rs"), "fn main() {}\n").expect("Rust fixture");

    let mut registry = python_stub();
    let analysis = analyze_repository(
        &AnalysisOptions {
            target: repository.path().to_path_buf(),
            match_patterns: vec![r"src/main\.py".to_owned()],
            include_ignored: Vec::new(),
            review: Default::default(),
        },
        &mut registry,
    )
    .expect("analysis should succeed");

    assert_eq!(
        analysis.selection.selected_files,
        [std::path::PathBuf::from("src/main.py")]
    );
    assert_eq!(analysis.analyzers[0].counts.analyzed, 1);
    assert_eq!(
        analysis.inventory_only_languages[&LanguageId::new("rust")],
        1
    );
    assert!(analysis.diagnostics.is_empty());
}

#[test]
fn an_invalid_regex_is_a_fatal_input_error() {
    let repository = tempdir().expect("temporary repository should be created");
    let mut registry = python_stub();
    let error = analyze_repository(
        &AnalysisOptions {
            target: repository.path().to_path_buf(),
            match_patterns: vec!["[".to_owned()],
            include_ignored: Vec::new(),
            review: Default::default(),
        },
        &mut registry,
    )
    .expect_err("invalid regex should fail");

    assert!(error.to_string().contains("invalid --match regex"));
}

#[test]
fn a_directory_without_supported_files_is_successful_with_a_diagnostic() {
    let repository = tempdir().expect("temporary repository should be created");
    std::fs::write(repository.path().join("main.rs"), "fn main() {}\n").expect("Rust fixture");
    let mut registry = python_stub();

    let analysis = analyze_repository(
        &AnalysisOptions {
            target: repository.path().to_path_buf(),
            ..AnalysisOptions::default()
        },
        &mut registry,
    )
    .expect("unsupported languages should not fail the repository run");

    assert_eq!(analysis.analyzers.len(), 1);
    assert_eq!(analysis.analyzers[0].counts.analyzed, 0);
    assert_eq!(analysis.diagnostics[0].code, "no-supported-files");
    assert_eq!(
        analysis.selection.root,
        repository.path().canonicalize().unwrap()
    );
    assert!(Path::new("main.rs").is_relative());
}
