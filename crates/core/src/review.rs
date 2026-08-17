use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::analysis::AnalyzerRun;
use crate::analyzer::{FileAnalysisStatus, SourceSpan, SymbolKind};
use crate::documentation::DocumentationCoverage;
use crate::inventory::detect_language;

pub const REVIEW_POLICY_VERSION: &str = "0.2.0";
pub const REVIEW_POLICY_DEFINITION_VERSION: &str = "review-policy-v2";
pub const PYTHON_CYCLOMATIC_COMPLEXITY: &str = "python-cyclomatic-complexity";
pub const PYTHON_CYCLOMATIC_COMPLEXITY_DEFINITION_VERSION: &str = "python-cyclomatic-complexity-v1";

#[derive(Debug, Clone, Default)]
pub struct ReviewOptions {
    pub policy_path: Option<PathBuf>,
    pub complexity_review_at: Option<u64>,
    pub complexity_block_at: Option<u64>,
    pub no_complexity_block: bool,
    pub documentation_review_below: Option<u8>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum RiskLevel {
    Low,
    Moderate,
    High,
    Critical,
    Unknown,
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
            Self::Critical => "critical",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum RequiredAction {
    None,
    HumanReview,
    Block,
}

impl RequiredAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::HumanReview => "human-review",
            Self::Block => "block",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReviewStatus {
    Pass,
    HumanReviewRequired,
    Blocked,
}

impl ReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::HumanReviewRequired => "human-review-required",
            Self::Blocked => "blocked",
        }
    }
}

pub fn review_status_code(status: ReviewStatus) -> u8 {
    match status {
        ReviewStatus::Pass => 0,
        ReviewStatus::HumanReviewRequired => 2,
        ReviewStatus::Blocked => 3,
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct RiskBands {
    pub moderate_at: u64,
    pub high_at: u64,
    pub critical_at: u64,
}

impl Default for RiskBands {
    fn default() -> Self {
        Self {
            moderate_at: 6,
            high_at: 11,
            critical_at: 21,
        }
    }
}

impl RiskBands {
    fn for_score(self, score: u64) -> RiskLevel {
        if score >= self.critical_at {
            RiskLevel::Critical
        } else if score >= self.high_at {
            RiskLevel::High
        } else if score >= self.moderate_at {
            RiskLevel::Moderate
        } else {
            RiskLevel::Low
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewException {
    pub symbol_id: String,
    pub reason: String,
    pub approved_max: Option<u64>,
    pub unbounded: bool,
}

impl ReviewException {
    fn acknowledges(&self, score: u64) -> bool {
        self.unbounded || self.approved_max.is_some_and(|limit| score <= limit)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewPolicy {
    pub definition_version: &'static str,
    pub sources: Vec<String>,
    pub risk_bands: RiskBands,
    pub complexity_review_at: u64,
    pub complexity_block_at: Option<u64>,
    pub documentation_review_below: Option<u8>,
    pub exceptions: Vec<ReviewException>,
}

impl ReviewPolicy {
    pub fn resolve(options: &ReviewOptions) -> Result<Self, ReviewPolicyError> {
        let mut policy = Self {
            definition_version: REVIEW_POLICY_DEFINITION_VERSION,
            sources: vec!["built-in".to_owned()],
            risk_bands: RiskBands::default(),
            complexity_review_at: 11,
            complexity_block_at: None,
            documentation_review_below: None,
            exceptions: Vec::new(),
        };

        if let Some(path) = &options.policy_path {
            let contents = fs::read_to_string(path).map_err(|source| ReviewPolicyError::Io {
                path: path.clone(),
                source,
            })?;
            let raw = serde_json::from_str::<RawPolicy>(&contents).map_err(|source| {
                ReviewPolicyError::Invalid {
                    path: Some(path.clone()),
                    message: source.to_string(),
                }
            })?;
            if !matches!(raw.policy_version.as_str(), "0.1.0" | REVIEW_POLICY_VERSION) {
                return Err(ReviewPolicyError::Invalid {
                    path: Some(path.clone()),
                    message: format!(
                        "unsupported policy_version {}; expected 0.1.0 or {}",
                        raw.policy_version, REVIEW_POLICY_VERSION
                    ),
                });
            }
            if raw.policy_version == "0.1.0"
                && raw.documentation_coverage.human_review_below.is_some()
            {
                return Err(ReviewPolicyError::Invalid {
                    path: Some(path.clone()),
                    message: "documentation_coverage requires policy_version 0.2.0".to_owned(),
                });
            }
            if let Some(value) = raw.cyclomatic_complexity.human_review_at {
                policy.complexity_review_at = value;
            }
            if raw.cyclomatic_complexity.block_at.is_some() {
                policy.complexity_block_at = raw.cyclomatic_complexity.block_at;
            }
            if let Some(bands) = raw.cyclomatic_complexity.risk_bands {
                policy.risk_bands = RiskBands {
                    moderate_at: bands.moderate_at.unwrap_or(policy.risk_bands.moderate_at),
                    high_at: bands.high_at.unwrap_or(policy.risk_bands.high_at),
                    critical_at: bands.critical_at.unwrap_or(policy.risk_bands.critical_at),
                };
            }
            policy.exceptions = raw
                .cyclomatic_complexity
                .exceptions
                .into_iter()
                .map(ReviewException::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            policy.documentation_review_below = raw.documentation_coverage.human_review_below;
            policy.sources.push("policy-file".to_owned());
        }

        if let Some(value) = options.complexity_review_at {
            policy.complexity_review_at = value;
            policy.sources.push("cli".to_owned());
        }
        if options.complexity_block_at.is_some() {
            policy.complexity_block_at = options.complexity_block_at;
            if !policy.sources.iter().any(|source| source == "cli") {
                policy.sources.push("cli".to_owned());
            }
        }
        if options.no_complexity_block {
            if options.complexity_block_at.is_some() {
                return Err(ReviewPolicyError::Invalid {
                    path: None,
                    message: "--no-complexity-block cannot be combined with --complexity-block-at"
                        .to_owned(),
                });
            }
            policy.complexity_block_at = None;
            if !policy.sources.iter().any(|source| source == "cli") {
                policy.sources.push("cli".to_owned());
            }
        }
        if options.documentation_review_below.is_some() {
            policy.documentation_review_below = options.documentation_review_below;
            if !policy.sources.iter().any(|source| source == "cli") {
                policy.sources.push("cli".to_owned());
            }
        }
        let mut unique_sources = Vec::with_capacity(policy.sources.len());
        for source in policy.sources {
            if !unique_sources.iter().any(|known| known == &source) {
                unique_sources.push(source);
            }
        }
        policy.sources = unique_sources;
        policy.validate().map(|()| policy)
    }

    fn validate(&self) -> Result<(), ReviewPolicyError> {
        if self.complexity_review_at == 0 {
            return Err(ReviewPolicyError::Invalid {
                path: None,
                message: "complexity review threshold must be at least 1".to_owned(),
            });
        }
        if self
            .complexity_block_at
            .is_some_and(|threshold| threshold < self.complexity_review_at)
        {
            return Err(ReviewPolicyError::Invalid {
                path: None,
                message: "complexity block threshold cannot be below the review threshold"
                    .to_owned(),
            });
        }
        if self.risk_bands.moderate_at < 2
            || self.risk_bands.moderate_at >= self.risk_bands.high_at
            || self.risk_bands.high_at >= self.risk_bands.critical_at
        {
            return Err(ReviewPolicyError::Invalid {
                path: None,
                message: "risk bands must be strictly increasing and start at 2 or above"
                    .to_owned(),
            });
        }
        if self
            .documentation_review_below
            .is_some_and(|threshold| !(1..=100).contains(&threshold))
        {
            return Err(ReviewPolicyError::Invalid {
                path: None,
                message: "documentation review threshold must be between 1 and 100".to_owned(),
            });
        }
        Ok(())
    }

    fn exception_for(&self, symbol_id: &str) -> Option<&ReviewException> {
        self.exceptions
            .iter()
            .find(|exception| exception.symbol_id == symbol_id)
    }
}

#[derive(Debug)]
pub enum ReviewPolicyError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Invalid {
        path: Option<PathBuf>,
        message: String,
    },
}

impl fmt::Display for ReviewPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "cannot read policy {}: {source}", path.display())
            }
            Self::Invalid {
                path: Some(path),
                message,
            } => write!(formatter, "invalid policy {}: {message}", path.display()),
            Self::Invalid {
                path: None,
                message,
            } => write!(formatter, "invalid review policy: {message}"),
        }
    }
}

impl std::error::Error for ReviewPolicyError {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    policy_version: String,
    #[serde(default)]
    cyclomatic_complexity: RawComplexityPolicy,
    #[serde(default)]
    documentation_coverage: RawDocumentationPolicy,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawDocumentationPolicy {
    human_review_below: Option<u8>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawComplexityPolicy {
    human_review_at: Option<u64>,
    block_at: Option<u64>,
    risk_bands: Option<RawRiskBands>,
    exceptions: Vec<RawException>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawRiskBands {
    moderate_at: Option<u64>,
    high_at: Option<u64>,
    critical_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawException {
    symbol_id: String,
    reason: String,
    approved_max: Option<u64>,
    unbounded: Option<bool>,
}

impl TryFrom<RawException> for ReviewException {
    type Error = ReviewPolicyError;

    fn try_from(value: RawException) -> Result<Self, Self::Error> {
        let unbounded = value.unbounded.unwrap_or(false);
        if value.symbol_id.is_empty() || value.reason.trim().is_empty() {
            return Err(ReviewPolicyError::Invalid {
                path: None,
                message: "review exceptions require a symbol_id and nonblank reason".to_owned(),
            });
        }
        if value.approved_max.is_some() == unbounded {
            return Err(ReviewPolicyError::Invalid {
                path: None,
                message: format!(
                    "exception for {} must specify exactly one of approved_max or unbounded=true",
                    value.symbol_id
                ),
            });
        }
        if value.approved_max == Some(0) {
            return Err(ReviewPolicyError::Invalid {
                path: None,
                message: "approved_max must be at least 1".to_owned(),
            });
        }
        Ok(Self {
            symbol_id: value.symbol_id,
            reason: value.reason,
            approved_max: value.approved_max,
            unbounded,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewCoverage {
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewRankingEntry {
    pub rank: usize,
    pub path: PathBuf,
    pub symbol_id: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub span: SourceSpan,
    pub score: u64,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewFinding {
    pub rule_id: String,
    pub risk: RiskLevel,
    pub required_action: RequiredAction,
    pub path: Option<PathBuf>,
    pub symbol_id: Option<String>,
    pub qualified_name: Option<String>,
    pub span: Option<SourceSpan>,
    pub observed_value: Option<u64>,
    pub threshold: Option<u64>,
    pub unit: Option<&'static str>,
    pub acknowledged: bool,
    pub message: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewEvaluation {
    pub status: ReviewStatus,
    pub policy: ReviewPolicy,
    pub coverage: ReviewCoverage,
    pub rankings: Vec<ReviewRankingEntry>,
    pub findings: Vec<ReviewFinding>,
}

pub fn evaluate_review(
    selected_files: &[PathBuf],
    analyzers: &[AnalyzerRun],
    documentation: &DocumentationCoverage,
    policy: ReviewPolicy,
) -> ReviewEvaluation {
    let analyzer_languages = analyzers
        .iter()
        .map(|run| run.descriptor.language.clone())
        .collect::<BTreeSet<_>>();
    let selected_language_files = selected_files
        .iter()
        .filter(|path| detect_language(path).is_some())
        .count();
    let unsupported_selected_files = selected_files
        .iter()
        .filter(|path| {
            detect_language(path).is_some_and(|language| !analyzer_languages.contains(&language))
        })
        .count();

    let mut coverage = ReviewCoverage {
        selected_language_files,
        unsupported_selected_files,
        analyzed_files: 0,
        successful_files: 0,
        partial_files: 0,
        failed_files: 0,
        eligible_callables: 0,
        measured_callables: 0,
        unavailable_callables: 0,
    };
    let mut rankings = Vec::new();
    let mut findings = Vec::new();

    for run in analyzers {
        for file in &run.files {
            coverage.analyzed_files += 1;
            match file.status {
                FileAnalysisStatus::Successful => coverage.successful_files += 1,
                FileAnalysisStatus::Partial => coverage.partial_files += 1,
                FileAnalysisStatus::Failed => coverage.failed_files += 1,
            }
            if file.status != FileAnalysisStatus::Successful {
                findings.push(ReviewFinding {
                    rule_id: "analysis-incomplete".to_owned(),
                    risk: RiskLevel::Unknown,
                    required_action: RequiredAction::HumanReview,
                    path: Some(file.path.clone()),
                    symbol_id: None,
                    qualified_name: None,
                    span: None,
                    observed_value: None,
                    threshold: None,
                    unit: None,
                    acknowledged: false,
                    message: format!(
                        "analysis is {} for {}; complexity evidence is incomplete",
                        file.status.as_str(),
                        file.path.display()
                    ),
                });
            }
            for symbol in &file.facts.symbols {
                if !matches!(
                    symbol.kind,
                    SymbolKind::Function | SymbolKind::Method | SymbolKind::Lambda
                ) {
                    continue;
                }
                coverage.eligible_callables += 1;
                let measurement = symbol
                    .measurements
                    .iter()
                    .find(|measurement| measurement.id == PYTHON_CYCLOMATIC_COMPLEXITY);
                let Some(measurement) = measurement else {
                    coverage.unavailable_callables += 1;
                    findings.push(ReviewFinding {
                        rule_id: "complexity-unavailable".to_owned(),
                        risk: RiskLevel::Unknown,
                        required_action: RequiredAction::HumanReview,
                        path: Some(file.path.clone()),
                        symbol_id: Some(symbol.id.as_str().to_owned()),
                        qualified_name: Some(symbol.qualified_name.clone()),
                        span: Some(symbol.span),
                        observed_value: None,
                        threshold: None,
                        unit: Some("score"),
                        acknowledged: false,
                        message: "cyclomatic complexity is unavailable for this callable"
                            .to_owned(),
                    });
                    continue;
                };
                let Some(score) = measurement.value else {
                    coverage.unavailable_callables += 1;
                    findings.push(ReviewFinding {
                        rule_id: "complexity-unavailable".to_owned(),
                        risk: RiskLevel::Unknown,
                        required_action: RequiredAction::HumanReview,
                        path: Some(file.path.clone()),
                        symbol_id: Some(symbol.id.as_str().to_owned()),
                        qualified_name: Some(symbol.qualified_name.clone()),
                        span: Some(symbol.span),
                        observed_value: None,
                        threshold: None,
                        unit: Some("score"),
                        acknowledged: false,
                        message: "cyclomatic complexity is unavailable for this callable"
                            .to_owned(),
                    });
                    continue;
                };
                coverage.measured_callables += 1;
                rankings.push(ReviewRankingEntry {
                    rank: 0,
                    path: file.path.clone(),
                    symbol_id: symbol.id.as_str().to_owned(),
                    qualified_name: symbol.qualified_name.clone(),
                    kind: symbol.kind,
                    span: symbol.span,
                    score,
                    risk: policy.risk_bands.for_score(score),
                });
                if score < policy.complexity_review_at {
                    continue;
                }
                let exception = policy.exception_for(symbol.id.as_str());
                let acknowledged = exception.is_some_and(|exception| exception.acknowledges(score));
                let required_action = if acknowledged {
                    RequiredAction::None
                } else if policy
                    .complexity_block_at
                    .is_some_and(|threshold| score >= threshold)
                {
                    RequiredAction::Block
                } else {
                    RequiredAction::HumanReview
                };
                let message = match exception {
                    Some(exception) if acknowledged => format!(
                        "cyclomatic complexity {score} is acknowledged by policy: {}",
                        exception.reason
                    ),
                    Some(exception) => format!(
                        "cyclomatic complexity {score} exceeds the approved exception for {}",
                        exception.reason
                    ),
                    None => format!(
                        "cyclomatic complexity {score} meets or exceeds the review threshold {}",
                        policy.complexity_review_at
                    ),
                };
                findings.push(ReviewFinding {
                    rule_id: "cyclomatic-complexity-threshold".to_owned(),
                    risk: policy.risk_bands.for_score(score),
                    required_action,
                    path: Some(file.path.clone()),
                    symbol_id: Some(symbol.id.as_str().to_owned()),
                    qualified_name: Some(symbol.qualified_name.clone()),
                    span: Some(symbol.span),
                    observed_value: Some(score),
                    threshold: Some(match required_action {
                        RequiredAction::Block => policy
                            .complexity_block_at
                            .unwrap_or(policy.complexity_review_at),
                        _ => policy.complexity_review_at,
                    }),
                    unit: Some("score"),
                    acknowledged,
                    message,
                });
            }
        }
    }

    if selected_language_files == 0 {
        findings.push(ReviewFinding {
            rule_id: "analysis-incomplete".to_owned(),
            risk: RiskLevel::Unknown,
            required_action: RequiredAction::HumanReview,
            path: None,
            symbol_id: None,
            qualified_name: None,
            span: None,
            observed_value: None,
            threshold: None,
            unit: None,
            acknowledged: false,
            message: "no recognized source files were selected for review".to_owned(),
        });
    } else if unsupported_selected_files > 0 {
        findings.push(ReviewFinding {
            rule_id: "analysis-incomplete".to_owned(),
            risk: RiskLevel::Unknown,
            required_action: RequiredAction::HumanReview,
            path: None,
            symbol_id: None,
            qualified_name: None,
            span: None,
            observed_value: None,
            threshold: None,
            unit: None,
            acknowledged: false,
            message: format!(
                "{} selected source file(s) have no registered analyzer",
                unsupported_selected_files
            ),
        });
    }

    if let Some(threshold) = policy.documentation_review_below {
        match documentation.threshold_is_met(threshold) {
            Some(true) => {}
            Some(false) => {
                let basis_points = documentation
                    .counts
                    .coverage_basis_points()
                    .expect("measured documentation coverage has a percentage");
                findings.push(ReviewFinding {
                    rule_id: "python-documentation-coverage-below-threshold".to_owned(),
                    risk: RiskLevel::Unknown,
                    required_action: RequiredAction::HumanReview,
                    path: None,
                    symbol_id: None,
                    qualified_name: None,
                    span: None,
                    observed_value: Some(u64::from(basis_points)),
                    threshold: Some(u64::from(threshold) * 100),
                    unit: Some("basis-points"),
                    acknowledged: false,
                    message: format!(
                        "Python documentation coverage is {}.{:02}% ({}/{}) and is below the {}% review threshold",
                        basis_points / 100,
                        basis_points % 100,
                        documentation.counts.documented,
                        documentation.counts.measured(),
                        threshold
                    ),
                });
            }
            None => findings.push(ReviewFinding {
                rule_id: "python-documentation-coverage-unavailable".to_owned(),
                risk: RiskLevel::Unknown,
                required_action: RequiredAction::HumanReview,
                path: None,
                symbol_id: None,
                qualified_name: None,
                span: None,
                observed_value: documentation
                    .counts
                    .coverage_basis_points()
                    .map(u64::from),
                threshold: Some(u64::from(threshold) * 100),
                unit: Some("basis-points"),
                acknowledged: false,
                message: format!(
                    "Python documentation coverage cannot be evaluated against the {threshold}% review threshold because analysis is {}",
                    documentation.status.as_str()
                ),
            }),
        }
    }

    rankings.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.span.start_byte.cmp(&right.span.start_byte))
            .then_with(|| left.symbol_id.cmp(&right.symbol_id))
    });
    for (index, ranking) in rankings.iter_mut().enumerate() {
        ranking.rank = index + 1;
    }
    findings.sort_by(|left, right| {
        right
            .required_action
            .cmp(&left.required_action)
            .then_with(|| right.risk.cmp(&left.risk))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.symbol_id.cmp(&right.symbol_id))
            .then_with(|| left.rule_id.cmp(&right.rule_id))
    });
    let status = if findings
        .iter()
        .any(|finding| finding.required_action == RequiredAction::Block)
    {
        ReviewStatus::Blocked
    } else if findings
        .iter()
        .any(|finding| finding.required_action == RequiredAction::HumanReview)
    {
        ReviewStatus::HumanReviewRequired
    } else {
        ReviewStatus::Pass
    };

    ReviewEvaluation {
        status,
        policy,
        coverage,
        rankings,
        findings,
    }
}
