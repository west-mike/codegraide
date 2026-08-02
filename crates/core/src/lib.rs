mod analysis;
mod analyzer;
mod config;
mod error;
mod inventory;
mod lines;
mod report;

pub use analysis::{
    AnalysisError, AnalysisOptions, AnalysisSelection, AnalysisTargetKind, AnalyzerRun,
    LanguageAnalysisCounts, RepositoryAnalysis, analyze_repository,
};
pub use analyzer::{
    AnalysisDiagnostic, AnalysisFacts, AnalysisInput, AnalysisLevel, AnalyzerCapability,
    AnalyzerDescriptor, AnalyzerRegistry, AnalyzerRegistryError, Decorator, DependencyKind,
    DependencyReference, DiagnosticSeverity, FileAnalysis, FileAnalysisStatus, GrammarDescriptor,
    LanguageAnalyzer, Measurement, MeasurementStatus, NestingEvent, NestingEventKind, Parameter,
    ParameterKind, QueryDescriptor, ResolutionLevel, SourcePosition, SourceSpan, Symbol,
    SymbolCompleteness, SymbolId, SymbolKind, SymbolModifier,
};
pub use error::InventoryError;
pub use inventory::{
    InventoryOptions, detect_language, inventory_repository, inventory_repository_with_options,
};
pub use lines::{LineCounts, RepositoryLineCounts};
pub use report::{
    AnalysisJsonReport, ExtensionId, FileCategory, IgnoredInventory, InventoryDiagnostic,
    InventoryJsonReport, LanguageId, RepositoryInventory,
};
