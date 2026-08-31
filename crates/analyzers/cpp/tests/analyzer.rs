use std::path::Path;

use codegraide_analyzer_cpp::{
    CPP_CYCLOMATIC_COMPLEXITY, CPP_MAX_CONTROL_FLOW_NESTING, CppAnalyzer,
};
use codegraide_core::{
    AnalysisInput, AnalyzerCapability, DependencyReference, FileAnalysisStatus, IncludeDelimiter,
    LanguageAnalyzer, MeasurementConcept, MeasurementStatus, SymbolKind,
};

fn analyze(name: &str, source: &[u8]) -> codegraide_core::FileAnalysis {
    let mut analyzer = CppAnalyzer::new().expect("C++ analyzer should initialize");
    analyzer.analyze(AnalysisInput {
        path: Path::new(name),
        source,
    })
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .expect("fixture should be readable")
}

#[test]
fn descriptor_declares_exact_provenance_and_capabilities() {
    let analyzer = CppAnalyzer::new().expect("C++ analyzer should initialize");
    let descriptor = analyzer.descriptor();
    assert_eq!(descriptor.id, "cpp-tree-sitter");
    assert_eq!(descriptor.language.as_str(), "cpp");
    assert_eq!(descriptor.version, "0.1.0");
    assert_eq!(descriptor.grammar.as_ref().unwrap().version, "0.23.4");
    assert!(
        descriptor
            .capabilities
            .contains(&AnalyzerCapability::Symbols)
    );
    assert!(
        !descriptor
            .capabilities
            .contains(&AnalyzerCapability::CallReferences)
    );
    assert!(
        !descriptor
            .capabilities
            .contains(&AnalyzerCapability::Documentation)
    );
    assert_eq!(descriptor.queries.len(), 4);
    assert!(descriptor.measurements.iter().any(|measurement| {
        measurement.concept == MeasurementConcept::CyclomaticComplexity
            && measurement.id == CPP_CYCLOMATIC_COMPLEXITY
    }));
}

#[test]
fn extracts_cpp_symbol_kinds_and_stable_qualified_names() {
    let result = analyze("symbols.cpp", &fixture("symbols.cpp"));
    assert_eq!(result.status, FileAnalysisStatus::Successful);
    let kinds = result
        .facts
        .symbols
        .iter()
        .map(|symbol| symbol.kind)
        .collect::<Vec<_>>();
    assert!(kinds.contains(&SymbolKind::Namespace));
    assert!(kinds.contains(&SymbolKind::Struct));
    assert!(kinds.contains(&SymbolKind::Class));
    assert!(kinds.contains(&SymbolKind::Method));
    assert!(kinds.contains(&SymbolKind::Function));
    assert!(kinds.contains(&SymbolKind::Lambda));
    assert!(
        result
            .facts
            .symbols
            .iter()
            .any(|symbol| symbol.qualified_name.contains("outer::"))
    );
    assert!(result.facts.symbols.iter().any(|symbol| {
        symbol.kind == SymbolKind::Namespace && symbol.name.starts_with("<anonymous-namespace>@")
    }));
    for callable in ["Worker", "~Worker", "operator()"] {
        assert!(
            result
                .facts
                .symbols
                .iter()
                .any(|symbol| symbol.name == callable),
            "missing {callable} definition"
        );
    }
    let overloads = result
        .facts
        .symbols
        .iter()
        .filter(|symbol| symbol.qualified_name.ends_with("overloaded"))
        .collect::<Vec<_>>();
    assert_eq!(overloads.len(), 2);
    assert_ne!(overloads[0].id, overloads[1].id);
}

#[test]
fn extracts_include_forms_and_conditional_context() {
    let result = analyze("includes.hpp", &fixture("includes.hpp"));
    let includes = result
        .facts
        .dependencies
        .iter()
        .map(|dependency| dependency.as_include().expect("include fact"))
        .collect::<Vec<_>>();
    assert_eq!(includes.len(), 4);
    assert_eq!(
        (includes[0].target.as_str(), includes[0].delimiter),
        ("vector", IncludeDelimiter::Angle)
    );
    assert_eq!(
        (includes[1].target.as_str(), includes[1].delimiter),
        ("local/widget.hpp", IncludeDelimiter::Quote)
    );
    assert_eq!(includes[2].delimiter, IncludeDelimiter::Macro);
    assert!(!includes[2].conditional);
    assert!(includes[3].conditional);
}

#[test]
fn applies_the_cpp_complexity_and_nesting_definitions() {
    let result = analyze("control_flow.cpp", &fixture("control_flow.cpp"));
    let function = result
        .facts
        .symbols
        .iter()
        .find(|symbol| symbol.name == "decisions")
        .expect("function symbol");
    let complexity = function
        .measurements
        .iter()
        .find(|measurement| measurement.id == CPP_CYCLOMATIC_COMPLEXITY)
        .expect("complexity measurement");
    assert_eq!(complexity.value, Some(12));
    let nesting = function
        .measurements
        .iter()
        .find(|measurement| measurement.id == CPP_MAX_CONTROL_FLOW_NESTING)
        .expect("nesting measurement");
    assert_eq!(nesting.value, Some(3));
}

#[test]
fn isolates_lambda_control_flow_from_the_enclosing_function() {
    let result = analyze("symbols.cpp", &fixture("symbols.cpp"));
    let function = result
        .facts
        .symbols
        .iter()
        .find(|symbol| symbol.name == "with_lambda")
        .expect("enclosing function");
    let lambda = result
        .facts
        .symbols
        .iter()
        .find(|symbol| symbol.kind == SymbolKind::Lambda)
        .expect("lambda symbol");
    let score = |symbol: &codegraide_core::Symbol| {
        symbol
            .measurements
            .iter()
            .find(|measurement| measurement.id == CPP_CYCLOMATIC_COMPLEXITY)
            .and_then(|measurement| measurement.value)
    };
    assert_eq!(score(function), Some(1));
    assert_eq!(score(lambda), Some(2));
}

#[test]
fn conditional_preprocessing_qualifies_control_flow_measurements() {
    let result = analyze("preprocessor.cpp", &fixture("preprocessor.cpp"));
    let function = result
        .facts
        .symbols
        .iter()
        .find(|symbol| symbol.name == "configured")
        .expect("function symbol");
    for id in [CPP_CYCLOMATIC_COMPLEXITY, CPP_MAX_CONTROL_FLOW_NESTING] {
        let measurement = function
            .measurements
            .iter()
            .find(|measurement| measurement.id == id)
            .expect("control-flow measurement");
        assert_eq!(measurement.status, MeasurementStatus::Unavailable);
        assert!(measurement.value.is_none());
    }
}

#[test]
fn malformed_and_non_utf8_inputs_are_partial_without_panicking() {
    let malformed = analyze("malformed.cpp", &fixture("malformed.cpp"));
    assert_eq!(malformed.status, FileAnalysisStatus::Partial);
    assert!(!malformed.diagnostics.is_empty());

    let non_utf8 = analyze("bytes.cpp", b"int name_\xff() { return 0; }\n");
    assert_eq!(non_utf8.status, FileAnalysisStatus::Partial);
    assert!(
        non_utf8
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "source-not-utf8")
    );

    let empty = analyze("empty.cpp", b"");
    assert_eq!(empty.status, FileAnalysisStatus::Successful);
    assert!(empty.facts.symbols.is_empty());
    assert!(
        empty
            .facts
            .dependencies
            .iter()
            .all(|dependency| matches!(dependency, DependencyReference::Include(_)))
    );

    let unicode = analyze("unicode.cpp", "int café() { return 1; }\n".as_bytes());
    assert_eq!(unicode.status, FileAnalysisStatus::Successful);
    assert!(
        unicode
            .facts
            .symbols
            .iter()
            .any(|symbol| symbol.name == "café")
    );
}

#[test]
fn repeated_analysis_is_deterministic() {
    let source = fixture("symbols.cpp");
    assert_eq!(
        analyze("symbols.cpp", &source),
        analyze("symbols.cpp", &source)
    );
}
