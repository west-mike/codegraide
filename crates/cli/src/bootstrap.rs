use std::error::Error;
use std::fmt;

use codegraide_analyzer_cpp::CppAnalyzer;
use codegraide_analyzer_python::PythonAnalyzer;
use codegraide_core::{AnalyzerRegistry, AnalyzerRegistryError, LanguageAnalyzer};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct BuiltinAnalyzerFeatures {
    pub(crate) documentation: bool,
}

#[derive(Debug)]
pub(crate) enum BuiltinAnalyzerBootstrapError {
    Initialization {
        analyzer: &'static str,
        source: Box<dyn Error>,
    },
    Registration {
        analyzer: &'static str,
        source: AnalyzerRegistryError,
    },
}

impl fmt::Display for BuiltinAnalyzerBootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initialization { analyzer, source } => {
                write!(
                    formatter,
                    "could not initialize {analyzer} analyzer: {source}"
                )
            }
            Self::Registration { analyzer, source } => {
                write!(
                    formatter,
                    "could not register {analyzer} analyzer: {source}"
                )
            }
        }
    }
}

impl Error for BuiltinAnalyzerBootstrapError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Initialization { source, .. } => Some(source.as_ref()),
            Self::Registration { source, .. } => Some(source),
        }
    }
}

pub(crate) fn build_builtin_analyzer_registry(
    features: BuiltinAnalyzerFeatures,
) -> Result<AnalyzerRegistry, BuiltinAnalyzerBootstrapError> {
    let mut registry = AnalyzerRegistry::new();
    register_builtin_analyzer(&mut registry, "C++", CppAnalyzer::new)?;
    register_builtin_analyzer(&mut registry, "Python", || {
        if features.documentation {
            PythonAnalyzer::new()
        } else {
            PythonAnalyzer::without_documentation()
        }
    })?;
    Ok(registry)
}

fn register_builtin_analyzer<A, E, F>(
    registry: &mut AnalyzerRegistry,
    analyzer: &'static str,
    factory: F,
) -> Result<(), BuiltinAnalyzerBootstrapError>
where
    A: LanguageAnalyzer + 'static,
    E: Error + 'static,
    F: FnOnce() -> Result<A, E>,
{
    let analyzer_instance =
        factory().map_err(|source| BuiltinAnalyzerBootstrapError::Initialization {
            analyzer,
            source: Box::new(source),
        })?;
    registry
        .register(Box::new(analyzer_instance))
        .map_err(|source| BuiltinAnalyzerBootstrapError::Registration { analyzer, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraide_core::{
        AnalysisFacts, AnalysisInput, AnalysisLevel, AnalyzerCapability, AnalyzerDescriptor,
        FileAnalysis, FileAnalysisStatus, GrammarDescriptor, LanguageId,
    };

    #[derive(Debug)]
    struct TestInitializationError;

    impl fmt::Display for TestInitializationError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test initialization failed")
        }
    }

    impl Error for TestInitializationError {}

    struct StubAnalyzer {
        descriptor: AnalyzerDescriptor,
    }

    impl StubAnalyzer {
        fn new(language: &str) -> Self {
            Self {
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
            }
        }
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

    #[test]
    fn registers_any_language_analyzer_implementation() {
        let mut registry = AnalyzerRegistry::new();
        register_builtin_analyzer(&mut registry, "Stub", || {
            Ok::<_, TestInitializationError>(StubAnalyzer::new("stub"))
        })
        .expect("stub analyzer should register");

        assert!(registry.analyzer_for(&LanguageId::new("stub")));
    }

    #[test]
    fn initialization_errors_preserve_analyzer_context_and_source() {
        let mut registry = AnalyzerRegistry::new();
        let error = register_builtin_analyzer(&mut registry, "Stub", || {
            Err::<StubAnalyzer, _>(TestInitializationError)
        })
        .expect_err("initialization should fail");

        assert_eq!(
            error.to_string(),
            "could not initialize Stub analyzer: test initialization failed"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("test initialization failed")
        );
    }

    #[test]
    fn duplicate_registration_errors_preserve_analyzer_context_and_source() {
        let mut registry = AnalyzerRegistry::new();
        register_builtin_analyzer(&mut registry, "Stub", || {
            Ok::<_, TestInitializationError>(StubAnalyzer::new("stub"))
        })
        .expect("first registration should succeed");

        let error = register_builtin_analyzer(&mut registry, "Stub", || {
            Ok::<_, TestInitializationError>(StubAnalyzer::new("stub"))
        })
        .expect_err("duplicate registration should fail");
        assert_eq!(
            error.to_string(),
            "could not register Stub analyzer: analyzer already registered for stub"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("analyzer already registered for stub")
        );
    }

    #[test]
    fn documentation_feature_changes_only_python_documentation_provenance() {
        let with_documentation = build_builtin_analyzer_registry(BuiltinAnalyzerFeatures {
            documentation: true,
        })
        .expect("Python analyzer with documentation should initialize");
        let without_documentation = build_builtin_analyzer_registry(BuiltinAnalyzerFeatures {
            documentation: false,
        })
        .expect("Python analyzer without documentation should initialize");
        let language = LanguageId::new("python");
        let with_descriptor = with_documentation
            .descriptor_for(&language)
            .expect("Python should be registered");
        let without_descriptor = without_documentation
            .descriptor_for(&language)
            .expect("Python should be registered");

        assert_eq!(with_descriptor.id, without_descriptor.id);
        assert_eq!(with_descriptor.version, without_descriptor.version);
        assert_eq!(with_descriptor.level, without_descriptor.level);
        assert_eq!(
            with_descriptor.grammar,
            Some(GrammarDescriptor {
                name: "tree-sitter-python".to_owned(),
                version: "0.25.0".to_owned(),
            })
        );
        assert_eq!(with_descriptor.grammar, without_descriptor.grammar);
        assert!(
            with_descriptor
                .capabilities
                .contains(&AnalyzerCapability::Documentation)
        );
        assert!(
            !without_descriptor
                .capabilities
                .contains(&AnalyzerCapability::Documentation)
        );
        let mut expected_capabilities = with_descriptor.capabilities.clone();
        expected_capabilities.remove(&AnalyzerCapability::Documentation);
        assert_eq!(expected_capabilities, without_descriptor.capabilities);
        assert!(
            with_descriptor
                .queries
                .iter()
                .any(|query| query.name == "docstrings")
        );
        let non_documentation_queries = with_descriptor
            .queries
            .iter()
            .filter(|query| query.name != "docstrings")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(non_documentation_queries, without_descriptor.queries);
    }
}
