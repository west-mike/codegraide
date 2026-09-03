use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use codegraide_core::{
    CallForm, CallReference, CallResolutionOutcome, CallableSignature, DependencyResolutionOutcome,
    DependencyTarget, LanguageId, ModuleId, ProjectCallResolution, ProjectLanguageModule,
    ProjectSymbol, ProjectSymbolId, ProjectSymbolLocation, RepositoryAnalysis, SymbolKind,
    SymbolLinkStatus, UsingReference, UsingReferenceKind,
};

use crate::CppDependencyResolution;

pub const CPP_SYMBOL_INDEX_DEFINITION_VERSION: &str = "cpp-symbol-index-v1";
pub const CPP_DECLARATION_LINK_DEFINITION_VERSION: &str = "cpp-declaration-definition-linking-v1";
pub const CPP_CALL_RESOLUTION_DEFINITION_VERSION: &str = "cpp-call-resolution-v1";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CppCallResolution {
    pub symbols: Vec<ProjectSymbol>,
    pub resolutions: Vec<ProjectCallResolution>,
    pub modules: Vec<ProjectLanguageModule>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct SymbolKey {
    qualified_name: String,
    kind: SymbolKind,
    signature_key: String,
}

#[derive(Debug, Clone)]
struct DefinitionOccurrence {
    path: PathBuf,
    span: codegraide_core::SourceSpan,
    syntax_id: String,
}

#[derive(Debug, Clone, Default)]
struct IndexEntry {
    definitions: Vec<DefinitionOccurrence>,
    declarations: Vec<ProjectSymbolLocation>,
    signature: Option<CallableSignature>,
    language_module: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct CandidateRank {
    qualification: u8,
    visibility: u8,
    signature: u8,
}

pub fn resolve_cpp_calls(
    analysis: &RepositoryAnalysis,
    dependencies: &CppDependencyResolution,
) -> CppCallResolution {
    let class_names = analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
        .flat_map(|run| run.files.iter())
        .flat_map(|file| file.facts.symbols.iter())
        .filter(|symbol| matches!(symbol.kind, SymbolKind::Class | SymbolKind::Struct))
        .map(|symbol| symbol.qualified_name.clone())
        .collect::<BTreeSet<_>>();

    let module_by_path = analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
        .flat_map(|run| run.files.iter())
        .filter_map(|file| {
            file.facts
                .modules
                .first()
                .map(|module| (file.path.clone(), module.name.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let exported_module_by_symbol = analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
        .flat_map(|run| run.files.iter())
        .filter_map(|file| {
            file.facts
                .modules
                .first()
                .map(|module| (module.name.clone(), &file.facts.module_exports))
        })
        .flat_map(|(module, exports)| {
            exports
                .iter()
                .filter(|export| export.complete)
                .map(move |export| (export.target.clone(), module.clone()))
        })
        .collect::<BTreeMap<_, _>>();

    let mut entries = BTreeMap::<SymbolKey, IndexEntry>::new();
    for run in analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
    {
        for file in &run.files {
            for symbol in &file.facts.symbols {
                let kind = canonical_kind(symbol.kind, &symbol.qualified_name, &class_names);
                let key = SymbolKey {
                    qualified_name: symbol.qualified_name.clone(),
                    kind,
                    signature_key: symbol
                        .callable_signature
                        .as_ref()
                        .map(|signature| signature.normalized_key.clone())
                        .unwrap_or_default(),
                };
                let entry = entries.entry(key).or_default();
                entry.signature = symbol.callable_signature.clone().or(entry.signature.take());
                entry.language_module = module_by_path
                    .get(&file.path)
                    .cloned()
                    .or_else(|| {
                        exported_module_by_symbol
                            .get(&symbol.qualified_name)
                            .cloned()
                    })
                    .or(entry.language_module.take());
                entry.definitions.push(DefinitionOccurrence {
                    path: file.path.clone(),
                    span: symbol.span,
                    syntax_id: symbol.id.as_str().to_owned(),
                });
                if symbol.kind == SymbolKind::Method {
                    entry.declarations.push(ProjectSymbolLocation {
                        path: file.path.clone(),
                        span: symbol.span,
                    });
                }
            }
            for declaration in &file.facts.declarations {
                let kind =
                    canonical_kind(declaration.kind, &declaration.qualified_name, &class_names);
                let key = SymbolKey {
                    qualified_name: declaration.qualified_name.clone(),
                    kind,
                    signature_key: declaration
                        .callable_signature
                        .as_ref()
                        .map(|signature| signature.normalized_key.clone())
                        .unwrap_or_default(),
                };
                let entry = entries.entry(key).or_default();
                entry.signature = declaration
                    .callable_signature
                    .clone()
                    .or(entry.signature.take());
                entry.language_module = module_by_path
                    .get(&file.path)
                    .cloned()
                    .or_else(|| {
                        exported_module_by_symbol
                            .get(&declaration.qualified_name)
                            .cloned()
                    })
                    .or(entry.language_module.take());
                entry.declarations.push(ProjectSymbolLocation {
                    path: file.path.clone(),
                    span: declaration.span,
                });
            }
        }
    }

    let mut overload_counts = BTreeMap::<(String, SymbolKind), usize>::new();
    let mut symbols = Vec::new();
    let mut by_syntax_id = BTreeMap::<(PathBuf, String), usize>::new();
    for (key, mut entry) in entries {
        entry.definitions.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.span.start_byte.cmp(&right.span.start_byte))
        });
        entry.declarations.sort();
        entry.declarations.dedup();
        let ordinal = overload_counts
            .entry((key.qualified_name.clone(), key.kind))
            .or_default();
        *ordinal += 1;
        let primary = entry
            .definitions
            .first()
            .map(|definition| ProjectSymbolLocation {
                path: definition.path.clone(),
                span: definition.span,
            })
            .or_else(|| entry.declarations.first().cloned())
            .expect("indexed C++ symbol has an occurrence");
        let link_status = match (entry.declarations.is_empty(), entry.definitions.len()) {
            (false, 1) => SymbolLinkStatus::Linked,
            (false, 0) => SymbolLinkStatus::DeclarationOnly,
            (true, 1) => SymbolLinkStatus::DefinitionOnly,
            (_, count) if count > 1 => SymbolLinkStatus::Ambiguous,
            _ => SymbolLinkStatus::Unavailable,
        };
        let project = ProjectSymbol {
            id: ProjectSymbolId {
                language: LanguageId::new("cpp"),
                module: ModuleId::new(LanguageId::new("cpp"), "<project>"),
                qualified_name: key.qualified_name,
                kind: key.kind,
                duplicate_ordinal: *ordinal,
            },
            path: primary.path.clone(),
            span: primary.span,
            signature: entry.signature,
            declarations: entry.declarations,
            definition: (entry.definitions.len() == 1).then(|| ProjectSymbolLocation {
                path: entry.definitions[0].path.clone(),
                span: entry.definitions[0].span,
            }),
            link_status,
            language_module: entry.language_module,
            architecture_groups: Vec::new(),
            primary_architecture_group: None,
        };
        let index = symbols.len();
        for definition in entry.definitions {
            by_syntax_id.insert((definition.path, definition.syntax_id), index);
        }
        symbols.push(project);
    }

    // Calls in global initializers, class initializers, and macro-generated test bodies do not
    // always have a callable syntax owner. Keep them visible under a stable per-file node.
    let mut file_initializers = BTreeMap::new();
    for run in analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
    {
        for file in &run.files {
            let Some(first_call) = file.facts.calls.first() else {
                continue;
            };
            let qualified_name = format!(
                "@file::{}::initialization",
                file.path.to_string_lossy().replace(['\\', '/'], "::")
            );
            symbols.push(ProjectSymbol {
                id: ProjectSymbolId {
                    language: LanguageId::new("cpp"),
                    module: ModuleId::new(LanguageId::new("cpp"), "<project>"),
                    qualified_name,
                    kind: SymbolKind::Function,
                    duplicate_ordinal: 1,
                },
                path: file.path.clone(),
                span: first_call.span,
                signature: None,
                declarations: Vec::new(),
                definition: Some(ProjectSymbolLocation {
                    path: file.path.clone(),
                    span: first_call.span,
                }),
                link_status: SymbolLinkStatus::DefinitionOnly,
                language_module: module_by_path.get(&file.path).cloned(),
                architecture_groups: Vec::new(),
                primary_architecture_group: None,
            });
        }
    }
    symbols.sort();

    for (index, symbol) in symbols.iter().enumerate() {
        if symbol.id.qualified_name.starts_with("@file::") {
            file_initializers.insert(symbol.path.clone(), index);
        }
    }

    // Sorting changed indexes, so rebuild the raw-definition lookup against canonical keys.
    by_syntax_id.clear();
    for run in analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
    {
        for file in &run.files {
            for symbol in &file.facts.symbols {
                let kind = canonical_kind(symbol.kind, &symbol.qualified_name, &class_names);
                let signature_key = symbol
                    .callable_signature
                    .as_ref()
                    .map(|signature| signature.normalized_key.as_str())
                    .unwrap_or_default();
                if let Some((index, _)) = symbols.iter().enumerate().find(|(_, candidate)| {
                    candidate.id.qualified_name == symbol.qualified_name
                        && candidate.id.kind == kind
                        && candidate
                            .signature
                            .as_ref()
                            .map(|signature| signature.normalized_key.as_str())
                            .unwrap_or_default()
                            == signature_key
                }) {
                    by_syntax_id.insert((file.path.clone(), symbol.id.as_str().to_owned()), index);
                }
            }
        }
    }

    let mut visibility = build_visibility(dependencies);
    add_module_visibility(analysis, &symbols, &mut visibility);
    let using_by_path = analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
        .flat_map(|run| run.files.iter())
        .map(|file| (file.path.clone(), file.facts.using_references.clone()))
        .collect::<BTreeMap<_, _>>();
    let by_terminal = symbols.iter().enumerate().fold(
        BTreeMap::<String, Vec<usize>>::new(),
        |mut catalog, (index, symbol)| {
            catalog
                .entry(terminal_name(&symbol.id.qualified_name).to_owned())
                .or_default()
                .push(index);
            catalog
        },
    );
    let mut resolutions = Vec::new();
    let diagnostics = Vec::new();
    for run in analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
    {
        for file in &run.files {
            for call in &file.facts.calls {
                let source_index = call
                    .enclosing_symbol
                    .as_ref()
                    .and_then(|id| by_syntax_id.get(&(file.path.clone(), id.as_str().to_owned())))
                    .or_else(|| file_initializers.get(&file.path))
                    .expect("a C++ file with calls has a synthetic initialization owner");
                let source = symbols[*source_index].clone();
                let outcome = resolve_call(
                    call,
                    &source,
                    &file.path,
                    &symbols,
                    &by_terminal,
                    &visibility,
                    using_by_path
                        .get(&file.path)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                );
                resolutions.push(ProjectCallResolution {
                    source,
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
    let modules = analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
        .flat_map(|run| run.files.iter())
        .flat_map(|file| {
            file.facts
                .modules
                .iter()
                .cloned()
                .map(|module| ProjectLanguageModule {
                    path: file.path.clone(),
                    module,
                    imports: file.facts.module_imports.clone(),
                    exports: file.facts.module_exports.clone(),
                })
        })
        .collect();
    CppCallResolution {
        symbols,
        resolutions,
        modules,
        diagnostics,
    }
}

fn canonical_kind(
    kind: SymbolKind,
    qualified_name: &str,
    class_names: &BTreeSet<String>,
) -> SymbolKind {
    if kind == SymbolKind::Function
        && qualified_name
            .rsplit_once("::")
            .is_some_and(|(owner, _)| class_names.contains(owner))
    {
        SymbolKind::Method
    } else {
        kind
    }
}

fn build_visibility(
    dependencies: &CppDependencyResolution,
) -> BTreeMap<PathBuf, BTreeMap<PathBuf, u8>> {
    let mut adjacency = BTreeMap::<PathBuf, Vec<(PathBuf, u8)>>::new();
    for resolution in &dependencies.resolutions {
        let (target, rank) = match &resolution.outcome {
            DependencyResolutionOutcome::Exact(DependencyTarget::LocalModule(target)) => {
                (&target.path, 1)
            }
            DependencyResolutionOutcome::Inferred {
                target: DependencyTarget::LocalModule(target),
                ..
            } => (&target.path, 2),
            _ => continue,
        };
        adjacency
            .entry(resolution.source_path.clone())
            .or_default()
            .push((target.clone(), rank));
    }
    let mut result = BTreeMap::new();
    for source in dependencies
        .local_modules
        .iter()
        .map(|module| module.path.clone())
    {
        let mut visible = BTreeMap::from([(source.clone(), 0)]);
        let mut queue = VecDeque::from([(source.clone(), 0u8)]);
        while let Some((current, inherited_rank)) = queue.pop_front() {
            for (target, edge_rank) in adjacency.get(&current).into_iter().flatten() {
                let rank = inherited_rank.max(*edge_rank);
                if visible.get(target).is_none_or(|existing| rank < *existing) {
                    visible.insert(target.clone(), rank);
                    queue.push_back((target.clone(), rank));
                }
            }
        }
        result.insert(source, visible);
    }
    result
}

fn add_module_visibility(
    analysis: &RepositoryAnalysis,
    symbols: &[ProjectSymbol],
    visibility: &mut BTreeMap<PathBuf, BTreeMap<PathBuf, u8>>,
) {
    let paths_by_module = symbols.iter().fold(
        BTreeMap::<String, BTreeSet<PathBuf>>::new(),
        |mut result, symbol| {
            if let Some(module) = &symbol.language_module {
                result
                    .entry(module.clone())
                    .or_default()
                    .insert(symbol.path.clone());
            }
            result
        },
    );
    for run in analysis
        .analyzers
        .iter()
        .filter(|run| run.descriptor.language.as_str() == "cpp")
    {
        for file in &run.files {
            let visible = visibility.entry(file.path.clone()).or_default();
            for import in &file.facts.module_imports {
                let rank = u8::from(import.conditional || !import.complete);
                for path in paths_by_module.get(&import.target).into_iter().flatten() {
                    visible
                        .entry(path.clone())
                        .and_modify(|current| *current = (*current).min(rank))
                        .or_insert(rank);
                }
            }
        }
    }
}

fn resolve_call(
    call: &CallReference,
    source: &ProjectSymbol,
    source_path: &PathBuf,
    symbols: &[ProjectSymbol],
    by_terminal: &BTreeMap<String, Vec<usize>>,
    visibility: &BTreeMap<PathBuf, BTreeMap<PathBuf, u8>>,
    using_references: &[UsingReference],
) -> CallResolutionOutcome {
    if !call.syntax_complete || call.preprocessing_uncertain {
        return CallResolutionOutcome::Unavailable(
            "parser recovery or conditional preprocessing affects this call".to_owned(),
        );
    }
    if call.form == CallForm::Unknown {
        return CallResolutionOutcome::Unavailable(
            "function-pointer or otherwise unknown callable ownership".to_owned(),
        );
    }
    let active_using = using_references
        .iter()
        .filter(|reference| reference.span.start_byte < call.span.start_byte)
        .collect::<Vec<_>>();
    let alias_terminal = active_using.iter().find_map(|reference| {
        (reference.kind == UsingReferenceKind::Alias
            && reference.alias.as_deref() == call.components.last().map(String::as_str))
        .then(|| terminal_name(&reference.target))
    });
    let terminal = if call.form == CallForm::Functor {
        "operator()"
    } else {
        alias_terminal.unwrap_or_else(|| {
            call.components
                .last()
                .map(String::as_str)
                .unwrap_or(call.callee.as_str())
        })
    };
    let mut ranked = by_terminal
        .get(terminal)
        .into_iter()
        .flatten()
        .filter_map(|index| {
            let candidate = &symbols[*index];
            if !callable_kind_compatible(call, candidate) {
                return None;
            }
            let visibility_rank = candidate_visibility_rank(
                candidate,
                source_path,
                call.span.start_byte,
                visibility.get(source_path)?,
            )?;
            let signature_rank = signature_rank(call, candidate.signature.as_ref())?;
            let qualification = qualification_rank(call, source, candidate, using_references)?;
            Some((
                CandidateRank {
                    qualification,
                    visibility: visibility_rank,
                    signature: signature_rank,
                },
                candidate.clone(),
            ))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let Some((best_rank, best)) = ranked.first().cloned() else {
        if call.callee.starts_with("std::") {
            return CallResolutionOutcome::External(call.callee.clone());
        }
        return CallResolutionOutcome::Unresolved(format!(
            "no visible signature-compatible target for {}",
            call.callee
        ));
    };
    let best_ties = ranked
        .iter()
        .take_while(|(rank, _)| *rank == best_rank)
        .map(|(_, candidate)| candidate.clone())
        .collect::<Vec<_>>();
    if best_ties.len() > 1 {
        return CallResolutionOutcome::Ambiguous(
            ranked.into_iter().map(|(_, candidate)| candidate).collect(),
        );
    }
    let alternatives = ranked
        .into_iter()
        .skip(1)
        .map(|(_, candidate)| candidate)
        .collect::<Vec<_>>();
    let exact = best_rank.qualification == 0
        && best_rank.visibility <= 1
        && best_rank.signature == 0
        && best.link_status != SymbolLinkStatus::Ambiguous
        && !best
            .signature
            .as_ref()
            .is_some_and(|signature| signature.virtual_dispatch);
    if exact {
        CallResolutionOutcome::Exact(best)
    } else {
        let virtual_note = if best
            .signature
            .as_ref()
            .is_some_and(|signature| signature.virtual_dispatch)
        {
            "; virtual dispatch may select an override"
        } else {
            ""
        };
        CallResolutionOutcome::Inferred {
            target: best,
            alternatives,
            reason: format!(
                "best syntactic candidate (qualification={}, visibility={}, signature={}){}",
                best_rank.qualification, best_rank.visibility, best_rank.signature, virtual_note
            ),
        }
    }
}

fn callable_kind_compatible(call: &CallReference, candidate: &ProjectSymbol) -> bool {
    if !matches!(
        candidate.id.kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Lambda
    ) {
        return false;
    }
    let name = terminal_name(&candidate.id.qualified_name);
    match call.form {
        CallForm::Constructor => candidate
            .id
            .qualified_name
            .rsplit_once("::")
            .is_some_and(|(owner, _)| terminal_name(owner) == name),
        CallForm::Functor => name == "operator()",
        _ => true,
    }
}

fn candidate_visibility_rank(
    candidate: &ProjectSymbol,
    source_path: &PathBuf,
    call_start: usize,
    visible: &BTreeMap<PathBuf, u8>,
) -> Option<u8> {
    candidate
        .declarations
        .iter()
        .chain(candidate.definition.iter())
        .filter(|location| location.path != *source_path || location.span.start_byte < call_start)
        .map(|location| &location.path)
        .chain(
            ((candidate.path != *source_path || candidate.span.start_byte < call_start)
                && candidate.declarations.is_empty()
                && candidate.definition.is_none())
            .then_some(&candidate.path),
        )
        .filter_map(|path| visible.get(path).copied())
        .min()
}

fn qualification_rank(
    call: &CallReference,
    source: &ProjectSymbol,
    candidate: &ProjectSymbol,
    using_references: &[UsingReference],
) -> Option<u8> {
    let candidate_name = &candidate.id.qualified_name;
    let normalized_callee = call.callee.replace("->", "::").replace('.', "::");
    if call.form == CallForm::Constructor
        && let Some((owner, constructor)) = candidate_name.rsplit_once("::")
        && terminal_name(owner) == terminal_name(&normalized_callee)
        && constructor == terminal_name(owner)
    {
        return Some(u8::from(!owner.ends_with(&normalized_callee)));
    }
    if normalized_callee.contains("::")
        && (candidate_name == &normalized_callee
            || candidate_name.ends_with(&format!("::{normalized_callee}")))
    {
        return Some(0);
    }
    let active_using = using_references
        .iter()
        .filter(|reference| reference.span.start_byte < call.span.start_byte)
        .collect::<Vec<_>>();
    for reference in &active_using {
        let rank = u8::from(!reference.complete);
        match reference.kind {
            UsingReferenceKind::Declaration
                if terminal_name(&reference.target) == terminal_name(&normalized_callee)
                    && (candidate_name == &reference.target
                        || candidate_name.ends_with(&format!("::{}", reference.target))) =>
            {
                return Some(rank);
            }
            UsingReferenceKind::Namespace
                if candidate_name.starts_with(&format!("{}::", reference.target)) =>
            {
                return Some(rank);
            }
            UsingReferenceKind::Alias
                if reference.alias.as_deref().is_some_and(|alias| {
                    normalized_callee == alias
                        || normalized_callee.starts_with(&format!("{alias}::"))
                }) && candidate_name.starts_with(&reference.target) =>
            {
                return Some(rank);
            }
            _ => {}
        }
    }
    if matches!(
        call.form,
        CallForm::Member | CallForm::PointerMember | CallForm::Functor
    ) {
        let owner = candidate_name.rsplit_once("::").map(|(owner, _)| owner);
        if let Some(receiver_type) = call.receiver_type_hint.as_deref()
            && owner.is_some_and(|owner| {
                owner == receiver_type || terminal_name(owner) == terminal_name(receiver_type)
            })
        {
            return Some(0);
        }
        if let Some(receiver_type) = call.receiver_type_hint.as_deref()
            && active_using.iter().any(|reference| {
                reference.kind == UsingReferenceKind::Alias
                    && reference.alias.as_deref() == Some(receiver_type)
                    && owner.is_some_and(|owner| owner == reference.target)
            })
        {
            return Some(0);
        }
        if call.receiver.as_deref() == Some("this")
            && source
                .id
                .qualified_name
                .rsplit_once("::")
                .is_some_and(|(source_owner, _)| owner == Some(source_owner))
        {
            return Some(0);
        }
        return Some(2);
    }
    let mut scope = source
        .id
        .qualified_name
        .rsplit_once("::")
        .map(|(scope, _)| scope);
    if scope.is_none() && !candidate_name.contains("::") {
        return Some(0);
    }
    while let Some(current) = scope {
        if candidate_name.as_str() == format!("{current}::{}", terminal_name(candidate_name)) {
            return Some(0);
        }
        scope = current.rsplit_once("::").map(|(parent, _)| parent);
    }
    Some(2)
}

fn signature_rank(call: &CallReference, signature: Option<&CallableSignature>) -> Option<u8> {
    let Some(signature) = signature else {
        return Some(2);
    };
    let required = signature
        .parameters
        .iter()
        .filter(|parameter| !parameter.has_default && !parameter.variadic)
        .count();
    let maximum = signature
        .parameters
        .iter()
        .all(|parameter| !parameter.variadic)
        .then_some(signature.parameters.len());
    let count = call.arguments.positional;
    if count < required || maximum.is_some_and(|maximum| count > maximum) {
        return None;
    }
    if call.argument_details.is_empty() && signature.parameters.is_empty() {
        return Some(0);
    }
    if call.argument_details.is_empty() || signature.parameters.is_empty() {
        return Some(1);
    }
    let mut known = 0;
    for (argument, parameter) in call.argument_details.iter().zip(&signature.parameters) {
        let (Some(argument), Some(parameter)) = (
            argument.type_hint.as_deref(),
            parameter.type_spelling.as_deref(),
        ) else {
            continue;
        };
        if !type_compatible(argument, parameter) {
            return None;
        }
        known += 1;
    }
    Some(u8::from(known != call.argument_details.len()))
}

fn type_compatible(argument: &str, parameter: &str) -> bool {
    let parameter = parameter.replace([' ', '\t', '\n', '\r'], "");
    match argument {
        "string-literal" => {
            parameter.contains("string")
                || parameter.contains("char*")
                || parameter.contains("char[")
                || parameter.contains("string_view")
        }
        "bool" => parameter.contains("bool"),
        "integer-literal" => ["int", "long", "short", "size_t", "uint", "char"]
            .iter()
            .any(|name| parameter.contains(name)),
        "floating-literal" => {
            parameter.contains("float")
                || parameter.contains("double")
                || parameter.contains("longdouble")
        }
        "null-pointer" => parameter.contains('*') || parameter.contains("nullptr_t"),
        declared => {
            let declared = declared.replace([' ', '\t', '\n', '\r'], "");
            parameter.trim_matches(['&', '*']) == declared.trim_matches(['&', '*'])
                || parameter.ends_with(&declared)
        }
    }
}

fn terminal_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}
