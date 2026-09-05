use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use codegraide_core::{
    CallResolutionOutcome, ProjectCallResolution, ProjectSymbol, ProjectSymbolId,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::Deserialize;

pub const ARCHITECTURE_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ArchitectureGroup {
    pub id: String,
    pub name: String,
}

struct CompiledGroup {
    public: ArchitectureGroup,
    paths: GlobSet,
    symbols: Vec<Regex>,
    modules: Vec<Regex>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchitectureFile {
    architecture_schema_version: String,
    groups: Vec<RawGroup>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGroup {
    id: String,
    name: String,
    #[serde(default)]
    path_globs: Vec<String>,
    #[serde(default)]
    symbol_regexes: Vec<String>,
    #[serde(default)]
    module_regexes: Vec<String>,
}

#[derive(Debug)]
pub enum ArchitectureError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    InvalidJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    Invalid(String),
}

impl fmt::Display for ArchitectureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "could not read architecture file {}: {source}",
                    path.display()
                )
            }
            Self::InvalidJson { path, source } => write!(
                formatter,
                "could not parse architecture file {}: {source}",
                path.display()
            ),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ArchitectureError {}

pub fn apply_architecture_file(
    path: &Path,
    symbols: &mut [ProjectSymbol],
) -> Result<Vec<ArchitectureGroup>, ArchitectureError> {
    let contents = fs::read_to_string(path).map_err(|source| ArchitectureError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let file: ArchitectureFile =
        serde_json::from_str(&contents).map_err(|source| ArchitectureError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    if file.architecture_schema_version != ARCHITECTURE_SCHEMA_VERSION {
        return Err(ArchitectureError::Invalid(format!(
            "unsupported architecture schema version {:?}; expected {:?}",
            file.architecture_schema_version, ARCHITECTURE_SCHEMA_VERSION
        )));
    }
    let mut ids = BTreeSet::new();
    let groups = file
        .groups
        .into_iter()
        .map(|group| compile_group(group, &mut ids))
        .collect::<Result<Vec<_>, _>>()?;
    for symbol in symbols {
        let matched = groups
            .iter()
            .filter(|group| group.matches(symbol))
            .map(|group| group.public.id.clone())
            .collect::<Vec<_>>();
        symbol.primary_architecture_group = matched.first().cloned();
        symbol.architecture_groups = matched;
    }
    Ok(groups.into_iter().map(|group| group.public).collect())
}

/// Apply group tags to both the canonical index and the symbol snapshots stored on call edges.
pub fn apply_architecture_to_resolution(
    path: &Path,
    resolution: &mut crate::CppCallResolution,
) -> Result<Vec<ArchitectureGroup>, ArchitectureError> {
    let groups = apply_architecture_file(path, &mut resolution.symbols)?;
    let indexed = resolution
        .symbols
        .iter()
        .cloned()
        .map(|symbol| (symbol.id.clone(), symbol))
        .collect::<std::collections::BTreeMap<ProjectSymbolId, ProjectSymbol>>();
    for call in &mut resolution.resolutions {
        refresh_call_symbols(call, &indexed);
    }
    Ok(groups)
}

fn refresh_call_symbols(
    call: &mut ProjectCallResolution,
    indexed: &std::collections::BTreeMap<ProjectSymbolId, ProjectSymbol>,
) {
    refresh_symbol(&mut call.source, indexed);
    match &mut call.outcome {
        CallResolutionOutcome::Exact(target) => refresh_symbol(target, indexed),
        CallResolutionOutcome::Inferred {
            target,
            alternatives,
            ..
        } => {
            refresh_symbol(target, indexed);
            for alternative in alternatives {
                refresh_symbol(alternative, indexed);
            }
        }
        CallResolutionOutcome::Ambiguous(candidates) => {
            for candidate in candidates {
                refresh_symbol(candidate, indexed);
            }
        }
        CallResolutionOutcome::External(_)
        | CallResolutionOutcome::Unresolved(_)
        | CallResolutionOutcome::Unavailable(_) => {}
    }
}

fn refresh_symbol(
    symbol: &mut ProjectSymbol,
    indexed: &std::collections::BTreeMap<ProjectSymbolId, ProjectSymbol>,
) {
    if let Some(canonical) = indexed.get(&symbol.id) {
        *symbol = canonical.clone();
    }
}

fn compile_group(
    group: RawGroup,
    ids: &mut BTreeSet<String>,
) -> Result<CompiledGroup, ArchitectureError> {
    if group.id.trim().is_empty() || group.name.trim().is_empty() {
        return Err(ArchitectureError::Invalid(
            "architecture group IDs and names must not be empty".to_owned(),
        ));
    }
    if !ids.insert(group.id.clone()) {
        return Err(ArchitectureError::Invalid(format!(
            "architecture group ID {:?} is duplicated",
            group.id
        )));
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in &group.path_globs {
        builder.add(Glob::new(pattern).map_err(|error| {
            ArchitectureError::Invalid(format!(
                "architecture group {:?} has invalid path glob {:?}: {error}",
                group.id, pattern
            ))
        })?);
    }
    let paths = builder.build().map_err(|error| {
        ArchitectureError::Invalid(format!(
            "architecture group {:?} path globs could not be compiled: {error}",
            group.id
        ))
    })?;
    let symbols = compile_regexes(&group.id, "symbol", group.symbol_regexes)?;
    let modules = compile_regexes(&group.id, "module", group.module_regexes)?;
    Ok(CompiledGroup {
        public: ArchitectureGroup {
            id: group.id,
            name: group.name,
        },
        paths,
        symbols,
        modules,
    })
}

fn compile_regexes(
    group: &str,
    kind: &str,
    patterns: Vec<String>,
) -> Result<Vec<Regex>, ArchitectureError> {
    patterns
        .into_iter()
        .map(|pattern| {
            Regex::new(&pattern).map_err(|error| {
                ArchitectureError::Invalid(format!(
                    "architecture group {group:?} has invalid {kind} regex {pattern:?}: {error}"
                ))
            })
        })
        .collect()
}

impl CompiledGroup {
    fn matches(&self, symbol: &ProjectSymbol) -> bool {
        let path_match = symbol_paths(symbol)
            .iter()
            .any(|path| self.paths.is_match(path));
        let symbol_match = symbol_and_owners(&symbol.id.qualified_name)
            .iter()
            .any(|name| self.symbols.iter().any(|pattern| pattern.is_match(name)));
        let module_match = symbol
            .language_module
            .as_deref()
            .is_some_and(|module| self.modules.iter().any(|pattern| pattern.is_match(module)));
        path_match || symbol_match || module_match
    }
}

fn symbol_paths(symbol: &ProjectSymbol) -> Vec<String> {
    let mut paths = vec![symbol.path.to_string_lossy().replace('\\', "/")];
    paths.extend(
        symbol
            .declarations
            .iter()
            .map(|location| location.path.to_string_lossy().replace('\\', "/")),
    );
    if let Some(definition) = &symbol.definition {
        paths.push(definition.path.to_string_lossy().replace('\\', "/"));
    }
    paths.sort();
    paths.dedup();
    paths
}

fn symbol_and_owners(name: &str) -> Vec<&str> {
    let mut result = vec![name];
    let mut current = name;
    while let Some((owner, _)) = current.rsplit_once("::") {
        result.push(owner);
        current = owner;
    }
    result
}
