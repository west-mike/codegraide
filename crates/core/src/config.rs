use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use regex::Regex;
use semver::Version;
use serde::Deserialize;

use crate::error::InventoryError;
use crate::report::{FileCategory, InventoryDiagnostic, normalize_relative_path};

const DEFAULT_CONFIG: &str = include_str!("../codegraide.json");
const SUPPORTED_CONFIG_VERSION: &str = "0.1.0";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    config_version: String,
    inventory: RawInventoryConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawInventoryConfig {
    ignore_defaults: bool,
    categories: BTreeMap<String, RawCategoryRule>,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum RuleMode {
    Extend,
    Replace,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawCategoryRule {
    mode: Option<RuleMode>,
    include_extensions: Vec<String>,
    include_filenames: Vec<String>,
    include_filename_regexes: Vec<String>,
    exclude_filenames: Vec<String>,
    exclude_filename_regexes: Vec<String>,
}

impl RawCategoryRule {
    fn merge(&mut self, other: Self) {
        self.include_extensions.extend(other.include_extensions);
        self.include_filenames.extend(other.include_filenames);
        self.include_filename_regexes
            .extend(other.include_filename_regexes);
        self.exclude_filenames.extend(other.exclude_filenames);
        self.exclude_filename_regexes
            .extend(other.exclude_filename_regexes);
    }
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum Specificity {
    Extension,
    FilenameRegex,
    ExactFilename,
}

impl Specificity {
    fn description(self) -> &'static str {
        match self {
            Self::Extension => "extension",
            Self::FilenameRegex => "filename regex",
            Self::ExactFilename => "exact filename",
        }
    }
}

#[derive(Debug)]
enum RegexTarget {
    Filename,
    RelativePath,
}

#[derive(Debug)]
struct CompiledRegex {
    raw: String,
    target: RegexTarget,
    regex: Regex,
}

impl CompiledRegex {
    fn is_match(&self, filename: Option<&str>, relative_path: &str) -> bool {
        match self.target {
            RegexTarget::Filename => filename.is_some_and(|name| self.regex.is_match(name)),
            RegexTarget::RelativePath => self.regex.is_match(relative_path),
        }
    }
}

#[derive(Debug, Default)]
struct CategoryRule {
    include_extensions: BTreeSet<String>,
    include_filenames: BTreeSet<String>,
    include_filename_regexes: Vec<CompiledRegex>,
    exclude_filenames: BTreeSet<String>,
    exclude_filename_regexes: Vec<CompiledRegex>,
}

#[derive(Debug)]
struct RuleMatch {
    specificity: Specificity,
    selector: String,
}

impl CategoryRule {
    fn matched_by(
        &self,
        filename: Option<&str>,
        extension: Option<&str>,
        relative_path: &str,
    ) -> Option<RuleMatch> {
        if self.is_excluded(filename, relative_path) {
            return None;
        }

        if let Some(name) = filename.filter(|name| self.include_filenames.contains(*name)) {
            return Some(RuleMatch {
                specificity: Specificity::ExactFilename,
                selector: name.to_owned(),
            });
        }

        if let Some(pattern) = self
            .include_filename_regexes
            .iter()
            .find(|pattern| pattern.is_match(filename, relative_path))
        {
            return Some(RuleMatch {
                specificity: Specificity::FilenameRegex,
                selector: pattern.raw.clone(),
            });
        }

        if let Some(value) = extension.filter(|value| self.include_extensions.contains(*value)) {
            return Some(RuleMatch {
                specificity: Specificity::Extension,
                selector: value.to_owned(),
            });
        }

        None
    }

    fn is_excluded(&self, filename: Option<&str>, relative_path: &str) -> bool {
        filename.is_some_and(|name| self.exclude_filenames.contains(name))
            || self
                .exclude_filename_regexes
                .iter()
                .any(|pattern| pattern.is_match(filename, relative_path))
    }
}

#[derive(Debug)]
struct Candidate {
    category: FileCategory,
    specificity: Specificity,
    selector: String,
}

#[derive(Debug)]
pub(crate) struct CategoryClassifier {
    rules: BTreeMap<FileCategory, CategoryRule>,
}

impl CategoryClassifier {
    pub(crate) fn load(
        config_path: Option<&Path>,
        emit_warnings: bool,
    ) -> Result<(Self, Vec<InventoryDiagnostic>), InventoryError> {
        let default_config = parse_config(DEFAULT_CONFIG, None)?;
        let mut raw_rules = parse_categories(default_config.inventory.categories, None)?;
        let mut diagnostics = Vec::new();

        if let Some(path) = config_path {
            let contents = fs::read_to_string(path).map_err(|error| {
                InventoryError::io(
                    format!("cannot read configuration {}", path.display()),
                    error,
                )
            })?;
            let custom_config = parse_config(&contents, Some(path))?;
            let custom_rules = parse_categories(custom_config.inventory.categories, Some(path))?;

            if custom_config.inventory.ignore_defaults {
                raw_rules.clear();
            }

            for category in FileCategory::ALL {
                let Some(mut custom_rule) = custom_rules.get(&category).cloned() else {
                    continue;
                };

                let mode = custom_rule.mode.take();
                if custom_config.inventory.ignore_defaults {
                    if mode == Some(RuleMode::Extend) && emit_warnings {
                        diagnostics.push(InventoryDiagnostic {
                            code: "config-child-extend-ignored",
                            message: format!(
                                "category {} requests mode \"extend\", but inventory.ignore_defaults is true; the child mode is ignored",
                                category.as_str()
                            ),
                        });
                    }
                    raw_rules.insert(category, custom_rule);
                    continue;
                }

                match mode.unwrap_or(RuleMode::Extend) {
                    RuleMode::Extend => raw_rules.entry(category).or_default().merge(custom_rule),
                    RuleMode::Replace => {
                        raw_rules.insert(category, custom_rule);
                    }
                }
            }
        }

        for category in FileCategory::ALL {
            raw_rules.entry(category).or_default();
        }

        validate_raw_rules(&raw_rules, config_path)?;

        let rules = raw_rules
            .into_iter()
            .map(|(category, rule)| {
                compile_rule(category, rule, config_path).map(|rule| (category, rule))
            })
            .collect::<Result<_, _>>()?;

        Ok((Self { rules }, diagnostics))
    }

    pub(crate) fn classify(
        &self,
        relative_path: &Path,
        diagnostics: &mut Vec<InventoryDiagnostic>,
        emit_warnings: bool,
    ) -> Result<FileCategory, InventoryError> {
        let filename = relative_path.file_name().and_then(|name| name.to_str());
        let extension = relative_path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        let normalized_path = normalize_relative_path(relative_path);
        let mut candidates = Vec::new();

        for category in FileCategory::ALL {
            let rule = self
                .rules
                .get(&category)
                .expect("all fixed categories have compiled rules");

            if let Some(matched_by) =
                rule.matched_by(filename, extension.as_deref(), &normalized_path)
            {
                candidates.push(Candidate {
                    category,
                    specificity: matched_by.specificity,
                    selector: matched_by.selector,
                });
            }
        }

        let Some(highest_specificity) = candidates
            .iter()
            .map(|candidate| candidate.specificity)
            .max()
        else {
            return Ok(FileCategory::Uncategorized);
        };

        let winners = candidates
            .iter()
            .filter(|candidate| candidate.specificity == highest_specificity)
            .collect::<Vec<_>>();

        if winners.len() > 1 {
            return Err(InventoryError::CategoryConflict {
                path: relative_path.to_path_buf(),
                selector: format!(
                    "multiple {} rules at the same specificity ({})",
                    highest_specificity.description(),
                    winners
                        .iter()
                        .map(|candidate| format!(
                            "{}: {:?}",
                            candidate.category.as_str(),
                            candidate.selector
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
                categories: winners
                    .iter()
                    .map(|candidate| candidate.category.as_str().to_owned())
                    .collect(),
            });
        }

        let winner = winners[0];
        if emit_warnings {
            for overridden in candidates
                .iter()
                .filter(|candidate| candidate.specificity < highest_specificity)
            {
                let remediation = match winner.specificity {
                    Specificity::ExactFilename => format!(
                        "add {:?} to {}.exclude_filenames",
                        winner.selector,
                        overridden.category.as_str()
                    ),
                    Specificity::FilenameRegex => format!(
                        "add {:?} to {}.exclude_filename_regexes",
                        winner.selector,
                        overridden.category.as_str()
                    ),
                    Specificity::Extension => {
                        unreachable!("an extension cannot override a less-specific selector")
                    }
                };
                diagnostics.push(InventoryDiagnostic {
                    code: "category-more-specific-rule-wins",
                    message: format!(
                        "{} is categorized as {} by {} {:?}, overriding {} by {} {:?}; {remediation} to make the exception explicit",
                        normalized_path,
                        winner.category.as_str(),
                        winner.specificity.description(),
                        winner.selector,
                        overridden.category.as_str(),
                        overridden.specificity.description(),
                        overridden.selector,
                    ),
                });
            }
        }

        Ok(winner.category)
    }
}

fn parse_config(contents: &str, path: Option<&Path>) -> Result<ConfigFile, InventoryError> {
    let config = serde_json::from_str::<ConfigFile>(contents).map_err(|error| {
        InventoryError::configuration(
            path.map(Path::to_path_buf),
            format!("JSON could not be parsed: {error}"),
        )
    })?;

    let version = Version::parse(&config.config_version).map_err(|error| {
        InventoryError::configuration(
            path.map(Path::to_path_buf),
            format!(
                "config_version {:?} is not valid Semantic Versioning: {error}",
                config.config_version
            ),
        )
    })?;
    let supported =
        Version::parse(SUPPORTED_CONFIG_VERSION).expect("supported config version is valid");

    if version != supported {
        return Err(InventoryError::configuration(
            path.map(Path::to_path_buf),
            format!(
                "unsupported config_version {version}; this codegraide version supports {supported}"
            ),
        ));
    }

    Ok(config)
}

fn parse_categories(
    categories: BTreeMap<String, RawCategoryRule>,
    path: Option<&Path>,
) -> Result<BTreeMap<FileCategory, RawCategoryRule>, InventoryError> {
    categories
        .into_iter()
        .map(|(name, rule)| {
            FileCategory::from_name(&name)
                .map(|category| (category, rule))
                .ok_or_else(|| {
                    InventoryError::configuration(
                        path.map(Path::to_path_buf),
                        format!(
                            "unknown category {name:?}; expected one of {}",
                            FileCategory::ALL.map(FileCategory::as_str).join(", ")
                        ),
                    )
                })
        })
        .collect()
}

fn validate_raw_rules(
    rules: &BTreeMap<FileCategory, RawCategoryRule>,
    path: Option<&Path>,
) -> Result<(), InventoryError> {
    let mut extension_owners = BTreeMap::<String, FileCategory>::new();
    let mut filename_owners = BTreeMap::<String, FileCategory>::new();
    let mut regex_owners = BTreeMap::<String, FileCategory>::new();

    for (&category, rule) in rules {
        for extension in &rule.include_extensions {
            validate_extension(extension, path)?;
            let normalized = extension.to_ascii_lowercase();
            claim_selector(
                "extension",
                normalized,
                category,
                &mut extension_owners,
                path,
            )?;
        }

        for filename in &rule.include_filenames {
            validate_filename(filename, true, path)?;
            claim_selector(
                "exact filename",
                filename.clone(),
                category,
                &mut filename_owners,
                path,
            )?;
        }

        for filename in &rule.exclude_filenames {
            validate_filename(filename, false, path)?;
        }

        let include_regexes = rule
            .include_filename_regexes
            .iter()
            .collect::<BTreeSet<_>>();
        let exclude_regexes = rule
            .exclude_filename_regexes
            .iter()
            .collect::<BTreeSet<_>>();

        if let Some(pattern) = include_regexes.intersection(&exclude_regexes).next() {
            return Err(InventoryError::configuration(
                path.map(Path::to_path_buf),
                format!(
                    "category {} includes and excludes the same filename regex {pattern:?}",
                    category.as_str()
                ),
            ));
        }

        let include_filenames = rule.include_filenames.iter().collect::<BTreeSet<_>>();
        let exclude_filenames = rule.exclude_filenames.iter().collect::<BTreeSet<_>>();
        if let Some(filename) = include_filenames.intersection(&exclude_filenames).next() {
            return Err(InventoryError::configuration(
                path.map(Path::to_path_buf),
                format!(
                    "category {} includes and excludes the same filename {filename:?}",
                    category.as_str()
                ),
            ));
        }

        for pattern in &rule.include_filename_regexes {
            validate_regex_shape(pattern, path)?;
            claim_selector(
                "filename regex",
                pattern.clone(),
                category,
                &mut regex_owners,
                path,
            )?;
        }
        for pattern in &rule.exclude_filename_regexes {
            validate_regex_shape(pattern, path)?;
        }
    }

    Ok(())
}

fn validate_extension(extension: &str, path: Option<&Path>) -> Result<(), InventoryError> {
    if extension.is_empty()
        || extension.starts_with('.')
        || extension.contains('/')
        || extension.contains('\\')
    {
        return Err(InventoryError::configuration(
            path.map(Path::to_path_buf),
            format!(
                "invalid extension {extension:?}; use a non-empty extension without a dot or path separator"
            ),
        ));
    }
    Ok(())
}

fn validate_filename(
    filename: &str,
    require_extension: bool,
    path: Option<&Path>,
) -> Result<(), InventoryError> {
    let value = Path::new(filename);
    let has_separator =
        value.components().count() != 1 || filename.contains('/') || filename.contains('\\');
    let missing_extension = require_extension && value.extension().is_none();

    if filename.is_empty() || has_separator || missing_extension {
        let requirement = if require_extension {
            "a complete filename with an extension and no directory"
        } else {
            "a filename with no directory"
        };
        return Err(InventoryError::configuration(
            path.map(Path::to_path_buf),
            format!("invalid filename {filename:?}; expected {requirement}"),
        ));
    }
    Ok(())
}

fn validate_regex_shape(pattern: &str, path: Option<&Path>) -> Result<(), InventoryError> {
    if pattern.is_empty() {
        return Err(InventoryError::configuration(
            path.map(Path::to_path_buf),
            "filename regexes cannot be empty",
        ));
    }
    if pattern.starts_with('/') || pattern.starts_with("^/") {
        return Err(InventoryError::configuration(
            path.map(Path::to_path_buf),
            format!(
                "filename regex {pattern:?} starts with '/'; use a repository-relative pattern"
            ),
        ));
    }
    Ok(())
}

fn claim_selector(
    selector_kind: &str,
    selector: String,
    category: FileCategory,
    owners: &mut BTreeMap<String, FileCategory>,
    path: Option<&Path>,
) -> Result<(), InventoryError> {
    if let Some(previous) = owners.insert(selector.clone(), category) {
        if previous != category {
            return Err(InventoryError::configuration(
                path.map(Path::to_path_buf),
                format!(
                    "{selector_kind} {selector:?} is claimed by both {} and {}",
                    previous.as_str(),
                    category.as_str()
                ),
            ));
        }
    }
    Ok(())
}

fn compile_rule(
    category: FileCategory,
    rule: RawCategoryRule,
    path: Option<&Path>,
) -> Result<CategoryRule, InventoryError> {
    Ok(CategoryRule {
        include_extensions: rule
            .include_extensions
            .into_iter()
            .map(|value| value.to_ascii_lowercase())
            .collect(),
        include_filenames: rule.include_filenames.into_iter().collect(),
        include_filename_regexes: compile_regexes(
            category,
            "include_filename_regexes",
            rule.include_filename_regexes,
            path,
        )?,
        exclude_filenames: rule.exclude_filenames.into_iter().collect(),
        exclude_filename_regexes: compile_regexes(
            category,
            "exclude_filename_regexes",
            rule.exclude_filename_regexes,
            path,
        )?,
    })
}

fn compile_regexes(
    category: FileCategory,
    field: &str,
    patterns: Vec<String>,
    path: Option<&Path>,
) -> Result<Vec<CompiledRegex>, InventoryError> {
    patterns
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|raw| {
            let target = if raw.contains('/') {
                RegexTarget::RelativePath
            } else {
                RegexTarget::Filename
            };
            let anchored = format!(r"\A(?:{raw})\z");
            let regex = Regex::new(&anchored).map_err(|error| {
                InventoryError::configuration(
                    path.map(Path::to_path_buf),
                    format!(
                        "category {} has invalid {field} pattern {raw:?}: {error}",
                        category.as_str()
                    ),
                )
            })?;

            Ok(CompiledRegex { raw, target, regex })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_regexes_match_at_any_depth_and_path_regexes_are_scoped() {
        let filename_pattern = compile_regexes(
            FileCategory::Data,
            "include_filename_regexes",
            vec![r"[0-9]{8}\.json".to_owned()],
            None,
        )
        .expect("filename pattern should compile");
        let path_pattern = compile_regexes(
            FileCategory::Data,
            "exclude_filename_regexes",
            vec![r"private/(?:.*/)?[0-9]{8}\.json".to_owned()],
            None,
        )
        .expect("path pattern should compile");

        assert!(filename_pattern[0].is_match(Some("20260728.json"), "logs/20260728.json"));
        assert!(path_pattern[0].is_match(Some("20260728.json"), "private/team/logs/20260728.json"));
        assert!(!path_pattern[0].is_match(Some("20260728.json"), "public/20260728.json"));
    }

    #[test]
    fn rejects_non_semver_config_versions() {
        let error = parse_config(
            r#"{"config_version":"0.1","inventory":{}}"#,
            Some(Path::new("rules.json")),
        )
        .expect_err("short version should be rejected");

        assert!(error.to_string().contains("Semantic Versioning"));
    }

    #[test]
    fn rejects_same_selector_in_two_categories() {
        let config = parse_config(
            r#"{
                "config_version": "0.1.0",
                "inventory": {
                    "ignore_defaults": true,
                    "categories": {
                        "documentation": {"include_extensions": ["md"]},
                        "data": {"include_extensions": ["MD"]}
                    }
                }
            }"#,
            Some(Path::new("rules.json")),
        )
        .expect("JSON shape should parse");
        let rules = parse_categories(config.inventory.categories, Some(Path::new("rules.json")))
            .expect("category names should parse");

        let error = validate_raw_rules(&rules, Some(Path::new("rules.json")))
            .expect_err("duplicate extension should be rejected");

        assert!(error.to_string().contains("documentation"));
        assert!(error.to_string().contains("data"));
    }
}
