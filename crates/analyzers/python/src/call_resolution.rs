use std::collections::BTreeMap;
use std::path::PathBuf;

use codegraide_core::{
    CallResolutionOutcome, DependencyResolutionOutcome, DependencyTarget, ModuleId,
    ProjectCallResolution, ProjectSymbol, ProjectSymbolId, ProjectSymbolLocation,
    RepositoryAnalysis, SymbolKind, SymbolLinkStatus,
};

use crate::PythonDependencyResolution;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PythonCallResolution {
    pub symbols: Vec<ProjectSymbol>,
    pub resolutions: Vec<ProjectCallResolution>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
enum ImportBinding {
    Module(String),
    Symbol { module: String, qualified: String },
    External,
}

pub fn resolve_python_calls(
    analysis: &RepositoryAnalysis,
    dependencies: &PythonDependencyResolution,
) -> PythonCallResolution {
    let module_by_path = dependencies
        .local_modules
        .iter()
        .map(|module| (module.path.clone(), module.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut symbols = Vec::new();
    let mut by_syntax_id = BTreeMap::<(PathBuf, String), ProjectSymbol>::new();
    for run in &analysis.analyzers {
        if run.descriptor.language.as_str() != "python" {
            continue;
        }
        for file in &run.files {
            let Some(module) = module_by_path.get(&file.path) else {
                continue;
            };
            let mut ordinals = BTreeMap::<(String, SymbolKind), usize>::new();
            for symbol in &file.facts.symbols {
                let qualified_name = if symbol.kind == SymbolKind::Module {
                    "<module>".to_owned()
                } else {
                    symbol.qualified_name.clone()
                };
                let ordinal = ordinals
                    .entry((qualified_name.clone(), symbol.kind))
                    .or_default();
                *ordinal += 1;
                let project = ProjectSymbol {
                    call_flow: None,
                    id: ProjectSymbolId {
                        language: module.language().clone(),
                        module: module.clone(),
                        qualified_name,
                        kind: symbol.kind,
                        duplicate_ordinal: *ordinal,
                    },
                    path: file.path.clone(),
                    span: symbol.span,
                    signature: None,
                    declarations: Vec::new(),
                    definition: Some(ProjectSymbolLocation {
                        path: file.path.clone(),
                        span: symbol.span,
                    }),
                    link_status: SymbolLinkStatus::DefinitionOnly,
                    language_module: None,
                    architecture_groups: Vec::new(),
                    primary_architecture_group: None,
                };
                by_syntax_id.insert(
                    (file.path.clone(), symbol.id.as_str().to_owned()),
                    project.clone(),
                );
                symbols.push(project);
            }
        }
    }
    symbols.sort();
    let by_module_qualified = symbols.iter().fold(
        BTreeMap::<(String, String), Vec<ProjectSymbol>>::new(),
        |mut result, symbol| {
            result
                .entry((
                    symbol.id.module.qualified_name().to_owned(),
                    symbol.id.qualified_name.clone(),
                ))
                .or_default()
                .push(symbol.clone());
            result
        },
    );
    let bindings = import_bindings(dependencies);
    let mut resolutions = Vec::new();
    let mut diagnostics = Vec::new();
    for run in &analysis.analyzers {
        if run.descriptor.language.as_str() != "python" {
            continue;
        }
        for file in &run.files {
            let Some(module) = module_by_path.get(&file.path) else {
                continue;
            };
            for call in &file.facts.calls {
                let Some(enclosing) = call
                    .enclosing_symbol
                    .as_ref()
                    .and_then(|id| by_syntax_id.get(&(file.path.clone(), id.as_str().to_owned())))
                else {
                    diagnostics.push(format!(
                        "call {} at {}:{} has no enclosing project symbol",
                        call.callee,
                        file.path.display(),
                        call.span.start.line
                    ));
                    continue;
                };
                let outcome = resolve_call(
                    call,
                    enclosing,
                    module,
                    bindings.get(&file.path),
                    &by_module_qualified,
                );
                resolutions.push(ProjectCallResolution {
                    source: enclosing.clone(),
                    source_path: file.path.clone(),
                    reference: call.clone(),
                    outcome,
                });
            }
        }
    }
    resolutions.sort_by(|left, right| {
        left.source_path.cmp(&right.source_path).then_with(|| {
            left.reference
                .span
                .start_byte
                .cmp(&right.reference.span.start_byte)
        })
    });
    PythonCallResolution {
        symbols,
        resolutions,
        diagnostics,
    }
}

fn import_bindings(
    dependencies: &PythonDependencyResolution,
) -> BTreeMap<PathBuf, BTreeMap<String, ImportBinding>> {
    let mut result = BTreeMap::<PathBuf, BTreeMap<String, ImportBinding>>::new();
    for resolution in &dependencies.resolutions {
        let reference = resolution
            .reference
            .as_import()
            .expect("Python dependency resolution contains imports");
        let key = reference
            .alias
            .clone()
            .or_else(|| reference.imported_name.clone())
            .or_else(|| {
                reference
                    .module
                    .as_deref()
                    .and_then(|module| module.split('.').next())
                    .map(str::to_owned)
            });
        let Some(key) = key else { continue };
        let binding = match &resolution.outcome {
            DependencyResolutionOutcome::Exact(DependencyTarget::LocalModule(target)) => {
                local_binding(
                    target.id.qualified_name(),
                    reference.imported_name.as_deref(),
                )
            }
            DependencyResolutionOutcome::Ambiguous { candidates, .. }
                if candidates
                    .iter()
                    .all(|target| matches!(target, DependencyTarget::LocalModule(_))) =>
            {
                let Some(module) = candidates.iter().find_map(|target| match target {
                    DependencyTarget::LocalModule(target) => Some(target.id.qualified_name()),
                    _ => None,
                }) else {
                    continue;
                };
                local_binding(module, reference.imported_name.as_deref())
            }
            DependencyResolutionOutcome::Exact(_) => ImportBinding::External,
            DependencyResolutionOutcome::Ambiguous { candidates, .. }
                if candidates
                    .iter()
                    .all(|target| !matches!(target, DependencyTarget::LocalModule(_))) =>
            {
                ImportBinding::External
            }
            DependencyResolutionOutcome::Unresolved { .. } if reference.relative_level == 0 => {
                ImportBinding::External
            }
            _ => continue,
        };
        result
            .entry(resolution.source_path.clone())
            .or_default()
            .insert(key, binding);
    }
    result
}

fn local_binding(module: &str, imported_name: Option<&str>) -> ImportBinding {
    if let Some(imported) = imported_name {
        if module == imported || module.ends_with(&format!(".{imported}")) {
            ImportBinding::Module(module.to_owned())
        } else {
            ImportBinding::Symbol {
                module: module.to_owned(),
                qualified: imported.to_owned(),
            }
        }
    } else {
        ImportBinding::Module(module.to_owned())
    }
}

fn resolve_call(
    call: &codegraide_core::CallReference,
    enclosing: &ProjectSymbol,
    module: &ModuleId,
    bindings: Option<&BTreeMap<String, ImportBinding>>,
    catalog: &BTreeMap<(String, String), Vec<ProjectSymbol>>,
) -> CallResolutionOutcome {
    if call.components.is_empty() || !call.syntax_complete {
        return CallResolutionOutcome::Unresolved(call.callee.clone());
    }
    let module_name = module.qualified_name();
    if call.components.len() == 1 {
        let name = &call.components[0];
        if let Some(binding) = bindings.and_then(|bindings| bindings.get(name)) {
            return resolve_binding(binding, &[], &call.callee, catalog);
        }
        let mut qualified_candidates = Vec::new();
        if enclosing.id.qualified_name != "<module>" {
            let mut parts = enclosing.id.qualified_name.split('.').collect::<Vec<_>>();
            if enclosing.id.qualified_name.rsplit('.').next() == Some(name.as_str()) {
                qualified_candidates.push(enclosing.id.qualified_name.clone());
            }
            loop {
                qualified_candidates.push(format!("{}.{}", parts.join("."), name));
                if parts.len() <= 1 {
                    break;
                }
                parts.pop();
            }
        }
        qualified_candidates.push(name.clone());
        for qualified in qualified_candidates {
            if let Some(candidates) = catalog.get(&(module_name.to_owned(), qualified)) {
                return local_outcome(candidates);
            }
        }
        return CallResolutionOutcome::Unresolved(call.callee.clone());
    }
    let first = &call.components[0];
    let rest = &call.components[1..];
    if matches!(first.as_str(), "self" | "cls")
        && matches!(enclosing.id.kind, SymbolKind::Method | SymbolKind::Class)
    {
        let class = enclosing
            .id
            .qualified_name
            .rsplit_once('.')
            .map(|(class, _)| class)
            .unwrap_or(&enclosing.id.qualified_name);
        let qualified = format!("{class}.{}", rest.join("."));
        return catalog
            .get(&(module_name.to_owned(), qualified))
            .map_or_else(
                || CallResolutionOutcome::Unresolved(call.callee.clone()),
                |candidates| local_outcome(candidates),
            );
    }
    if let Some(binding) = bindings.and_then(|bindings| bindings.get(first)) {
        return resolve_binding(binding, rest, &call.callee, catalog);
    }
    CallResolutionOutcome::Unresolved(call.callee.clone())
}

fn resolve_binding(
    binding: &ImportBinding,
    rest: &[String],
    spelling: &str,
    catalog: &BTreeMap<(String, String), Vec<ProjectSymbol>>,
) -> CallResolutionOutcome {
    match binding {
        ImportBinding::External => CallResolutionOutcome::External(spelling.to_owned()),
        ImportBinding::Module(module) if !rest.is_empty() => {
            catalog.get(&(module.clone(), rest.join("."))).map_or_else(
                || CallResolutionOutcome::Unresolved(spelling.to_owned()),
                |candidates| local_outcome(candidates),
            )
        }
        ImportBinding::Symbol { module, qualified } if rest.is_empty() => catalog
            .get(&(module.clone(), qualified.clone()))
            .map_or_else(
                || CallResolutionOutcome::Unresolved(spelling.to_owned()),
                |candidates| local_outcome(candidates),
            ),
        ImportBinding::Symbol { module, qualified } => catalog
            .get(&(module.clone(), format!("{qualified}.{}", rest.join("."))))
            .map_or_else(
                || CallResolutionOutcome::Unresolved(spelling.to_owned()),
                |candidates| local_outcome(candidates),
            ),
        ImportBinding::Module(_) => CallResolutionOutcome::Unresolved(spelling.to_owned()),
    }
}

fn local_outcome(candidates: &[ProjectSymbol]) -> CallResolutionOutcome {
    if candidates.len() == 1 {
        CallResolutionOutcome::Exact(candidates[0].clone())
    } else {
        CallResolutionOutcome::Ambiguous(candidates.to_vec())
    }
}
