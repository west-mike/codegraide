//! Bounded factual review context. No defect predictions or test-selection policy.
use crate::git_snapshot::{GitSnapshot, SourceReference};
use crate::{
    CallResolutionOutcome, ProjectCallResolution, ProjectSymbol, ProjectSymbolId, SourceSpan,
    SymbolKind,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

pub const SCHEMA_VERSION: &str = "review-context-v1";
#[derive(Debug, Clone, Serialize)]
pub struct ContextLimits {
    pub depth: usize,
    pub max_symbols: usize,
    pub max_edges: usize,
    pub max_code_bytes: usize,
    pub include_callees: bool,
    pub show_declarations: bool,
    pub all_relations: bool,
}
impl Default for ContextLimits {
    fn default() -> Self {
        Self {
            depth: 1,
            max_symbols: 200,
            max_edges: 1000,
            max_code_bytes: 1_048_576,
            include_callees: false,
            show_declarations: false,
            all_relations: false,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Code {
    pub state: &'static str,
    pub text: Option<String>,
    pub reason: Option<&'static str>,
}
impl Code {
    pub fn from_source(source: Option<&str>, include: bool, budget: &mut usize) -> Self {
        match source {
            None => Self {
                state: "unavailable",
                text: None,
                reason: Some("invalid-or-unavailable-source-span"),
            },
            Some(_) if !include => Self {
                state: "omitted",
                text: None,
                reason: Some("callee-body-policy"),
            },
            Some(s) if s.len() > *budget => Self {
                state: "omitted",
                text: None,
                reason: Some("code-byte-limit"),
            },
            Some(s) => {
                *budget -= s.len();
                Self {
                    state: "included",
                    text: Some(s.into()),
                    reason: None,
                }
            }
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct SourceRecord {
    pub reference: String,
    pub commit: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub changed: Option<bool>,
    pub code: Code,
}
/// One traversal step explaining why a context symbol was selected. References
/// remain snapshot-specific even when its unchanged source is shared.
#[derive(Debug, Clone, Serialize)]
pub struct ContextOrigin {
    pub from: String,
    pub role: String,
    pub resolution: String,
}
#[derive(Debug, Serialize)]
pub struct SourceLocation {
    pub reference: String,
    pub commit: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}
#[derive(Debug, Serialize)]
pub struct OtherSnapshot {
    #[serde(flatten)]
    pub source: SourceLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<ContextOrigin>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub declarations: Vec<SourceLocation>,
}
#[derive(Debug, Serialize)]
pub struct ContextSymbol {
    #[serde(flatten)]
    pub source: SourceRecord,
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
    pub definition: bool,
    pub link_status: String,
    pub roles: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<ContextOrigin>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_snapshots: Vec<OtherSnapshot>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub declarations: Vec<SourceRecord>,
}
#[derive(Debug, Serialize)]
pub struct Change {
    pub status: &'static str,
    pub matching: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    pub before: Option<String>,
    pub after: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ContextEdge {
    pub snapshot: String,
    pub from: String,
    pub from_name: String,
    pub to: Option<String>,
    pub to_name: Option<String>,
    pub relation: &'static str,
    pub resolution: String,
    pub candidates: Vec<String>,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub expression: String,
    pub reason: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct ChangedFile {
    pub before: Option<String>,
    pub after: Option<String>,
    pub status: &'static str,
    pub analysis: String,
    pub changed_functions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyzerProvenance {
    pub id: String,
    pub version: String,
    pub grammar: Option<String>,
    pub grammar_version: Option<String>,
    pub queries: BTreeMap<String, String>,
}
impl From<&crate::AnalyzerDescriptor> for AnalyzerProvenance {
    fn from(d: &crate::AnalyzerDescriptor) -> Self {
        Self {
            id: d.id.clone(),
            version: d.version.clone(),
            grammar: d.grammar.as_ref().map(|g| g.name.clone()),
            grammar_version: d.grammar.as_ref().map(|g| g.version.clone()),
            queries: d
                .queries
                .iter()
                .map(|q| (q.name.clone(), q.version.clone()))
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ContextReport {
    pub schema_version: &'static str,
    pub base: Option<String>,
    pub head: String,
    pub language: &'static str,
    pub analyzers: Vec<AnalyzerProvenance>,
    pub files: Vec<ChangedFile>,
    pub changes: Vec<Change>,
    pub symbols: Vec<ContextSymbol>,
    pub relations: Vec<ContextEdge>,
    pub limits: ContextLimits,
    pub omissions: BTreeMap<String, usize>,
    pub limitations: Vec<String>,
    pub diagnostics: Vec<String>,
}

pub struct ContextSnapshot {
    pub git: GitSnapshot,
    pub symbols: BTreeMap<String, ProjectSymbol>,
    pub relations: Vec<ContextEdge>,
    pub diagnostics: Vec<String>,
    pub analyzers: Vec<AnalyzerProvenance>,
}
impl ContextSnapshot {
    pub fn new(
        git: GitSnapshot,
        symbols: Vec<ProjectSymbol>,
        calls: Vec<ProjectCallResolution>,
        diagnostics: Vec<String>,
        analyzers: Vec<crate::AnalyzerDescriptor>,
    ) -> Self {
        let ids: BTreeMap<ProjectSymbolId, String> = symbols
            .iter()
            .filter_map(|s| reference(&git, &s.path, s.span).map(|r| (s.id.clone(), r)))
            .collect();
        let mut relations = Vec::new();
        for call in calls {
            let Some(from) = ids.get(&call.source.id) else {
                continue;
            };
            let (resolution, target, candidates, reason) = match &call.outcome {
                CallResolutionOutcome::Exact(s) => ("exact", Some(s), Vec::new(), None),
                CallResolutionOutcome::Inferred {
                    target,
                    alternatives,
                    reason,
                } => (
                    "inferred",
                    Some(target),
                    alternatives.iter().collect(),
                    Some(reason.clone()),
                ),
                CallResolutionOutcome::Ambiguous(candidates) => {
                    ("ambiguous", None, candidates.iter().collect(), None)
                }
                CallResolutionOutcome::External(reason) => {
                    ("external", None, Vec::new(), Some(reason.clone()))
                }
                CallResolutionOutcome::Unresolved(reason) => {
                    ("unresolved", None, Vec::new(), Some(reason.clone()))
                }
                CallResolutionOutcome::Unavailable(reason) => {
                    ("unavailable", None, Vec::new(), Some(reason.clone()))
                }
            };
            relations.push(ContextEdge {
                snapshot: git.commit.clone(),
                from: from.clone(),
                from_name: call.source.id.qualified_name.clone(),
                to: target.and_then(|s| ids.get(&s.id)).cloned(),
                to_name: target.map(|s| s.id.qualified_name.clone()),
                relation: "call",
                resolution: resolution.into(),
                candidates: candidates
                    .iter()
                    .filter_map(|s| ids.get(&s.id).cloned())
                    .collect(),
                path: path_text(&call.source_path),
                line: call.reference.span.start.line,
                column: call.reference.span.start.column,
                expression: call.reference.expression,
                reason,
            });
        }
        relations.sort_by(|a, b| {
            (&a.from, &a.path, a.line, a.column, &a.to)
                .cmp(&(&b.from, &b.path, b.line, b.column, &b.to))
        });
        Self {
            symbols: symbols
                .into_iter()
                .filter_map(|s| ids.get(&s.id).cloned().map(|id| (id, s)))
                .collect(),
            git,
            relations,
            diagnostics,
            analyzers: analyzers.iter().map(AnalyzerProvenance::from).collect(),
        }
    }
    fn code(&self, symbol: &ProjectSymbol) -> Option<&str> {
        self.git
            .files
            .get(&symbol.path)?
            .source
            .get(symbol.span.start_byte..symbol.span.end_byte)
    }
    fn declaration_codes(
        &self,
        symbol: &ProjectSymbol,
        renames: &BTreeMap<PathBuf, PathBuf>,
    ) -> BTreeSet<(PathBuf, String)> {
        symbol
            .declarations
            .iter()
            .filter_map(|d| {
                Some((
                    renames.get(&d.path).unwrap_or(&d.path).clone(),
                    self.git
                        .files
                        .get(&d.path)?
                        .source
                        .get(d.span.start_byte..d.span.end_byte)?
                        .into(),
                ))
            })
            .collect()
    }
}
fn path_text(path: &std::path::Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
fn reference(git: &GitSnapshot, path: &std::path::Path, span: SourceSpan) -> Option<String> {
    let file = git.files.get(path)?;
    file.source.get(span.start_byte..span.end_byte)?;
    Some(
        SourceReference {
            commit: git.commit.clone(),
            object: file.object.clone(),
            path: path_text(path),
            start: span.start_byte,
            end: span.end_byte,
        }
        .encode(),
    )
}
fn callable(s: &ProjectSymbol) -> bool {
    matches!(
        s.id.kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Lambda
    )
}
fn type_symbol(s: &ProjectSymbol) -> bool {
    matches!(s.id.kind, SymbolKind::Class | SymbolKind::Struct)
}
fn signature(s: &ProjectSymbol) -> &str {
    s.signature
        .as_ref()
        .map(|s| s.normalized_key.as_str())
        .unwrap_or("")
}

/// Match exact signatures first; only pair changed signatures when one unmatched
/// function remains on each side of a path/name group. Ambiguous overload changes
/// stay additions/removals rather than inventing a correspondence.
fn changes(
    base: &ContextSnapshot,
    head: &ContextSnapshot,
    renames: &BTreeMap<PathBuf, PathBuf>,
) -> Vec<Change> {
    type Key = (PathBuf, String, SymbolKind);
    let mut groups: BTreeMap<Key, (Vec<&String>, Vec<&String>)> = BTreeMap::new();
    for (id, s) in &base.symbols {
        if callable(s) {
            groups
                .entry((
                    renames.get(&s.path).unwrap_or(&s.path).clone(),
                    s.id.qualified_name.clone(),
                    s.id.kind,
                ))
                .or_default()
                .0
                .push(id);
        }
    }
    for (id, s) in &head.symbols {
        if callable(s) {
            groups
                .entry((s.path.clone(), s.id.qualified_name.clone(), s.id.kind))
                .or_default()
                .1
                .push(id);
        }
    }
    let mut result = Vec::new();
    for (mut before, mut after) in groups.into_values() {
        let mut pairs = Vec::new();
        let signatures: BTreeSet<_> = before
            .iter()
            .map(|id| signature(&base.symbols[*id]).to_owned())
            .collect();
        for sig in signatures {
            let bs: Vec<_> = before
                .iter()
                .copied()
                .filter(|id| signature(&base.symbols[*id]) == sig)
                .collect();
            let hs: Vec<_> = after
                .iter()
                .copied()
                .filter(|id| signature(&head.symbols[*id]) == sig)
                .collect();
            if bs.len() == 1 && hs.len() == 1 {
                pairs.push((bs[0], hs[0], "signature"));
                before.retain(|id| *id != bs[0]);
                after.retain(|id| *id != hs[0]);
            }
        }
        if before.len() == 1 && after.len() == 1 {
            pairs.push((before.remove(0), after.remove(0), "unique-name"));
        }
        for (b, h, matching) in pairs {
            let bs = &base.symbols[b];
            let hs = &head.symbols[h];
            if base.code(bs) != head.code(hs)
                || base.declaration_codes(bs, renames)
                    != head.declaration_codes(hs, &BTreeMap::new())
                || bs.path != hs.path
            {
                result.push(Change {
                    status: if base.code(bs) == head.code(hs)
                        && base.declaration_codes(bs, renames)
                            == head.declaration_codes(hs, &BTreeMap::new())
                        && bs.path != hs.path
                    {
                        "renamed"
                    } else {
                        "modified"
                    },
                    matching,
                    reason: (base.code(bs) == head.code(hs)
                        && base.declaration_codes(bs, renames)
                            != head.declaration_codes(hs, &BTreeMap::new())
                        || bs.definition.is_none() && hs.definition.is_none())
                    .then_some("declaration-only"),
                    before: Some(b.clone()),
                    after: Some(h.clone()),
                });
            }
        }
        for b in before {
            result.push(Change {
                status: "removed",
                matching: "unpaired",
                reason: base.symbols[b]
                    .definition
                    .is_none()
                    .then_some("declaration-only"),
                before: Some(b.clone()),
                after: None,
            });
        }
        for h in after {
            result.push(Change {
                status: "added",
                matching: "unpaired",
                reason: head.symbols[h]
                    .definition
                    .is_none()
                    .then_some("declaration-only"),
                before: None,
                after: Some(h.clone()),
            });
        }
    }
    result
}
fn changed_files(
    base: &ContextSnapshot,
    head: &ContextSnapshot,
    renames: &BTreeMap<PathBuf, PathBuf>,
    changes: &[Change],
) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    let mut paired = BTreeSet::new();
    let mut add = |before: Option<&str>, after: Option<&str>, status| {
        let (snapshot, path) = after
            .map(|p| (head, p))
            .unwrap_or((base, before.unwrap_or_default()));
        let analysis = if snapshot.git.files.contains_key(&PathBuf::from(path)) {
            "cpp".into()
        } else {
            snapshot
                .git
                .excluded
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, reason)| reason.clone())
                .unwrap_or_else(|| "unavailable".into())
        };
        let count = changes
            .iter()
            .filter(|c| {
                c.before
                    .as_ref()
                    .and_then(|id| base.symbols.get(id))
                    .is_some_and(|s| {
                        Some(path_text(&s.path)).as_deref() == before
                            || s.declarations
                                .iter()
                                .any(|d| Some(path_text(&d.path)).as_deref() == before)
                    })
                    || c.after
                        .as_ref()
                        .and_then(|id| head.symbols.get(id))
                        .is_some_and(|s| {
                            Some(path_text(&s.path)).as_deref() == after
                                || s.declarations
                                    .iter()
                                    .any(|d| Some(path_text(&d.path)).as_deref() == after)
                        })
            })
            .count();
        files.push(ChangedFile {
            before: before.map(str::to_owned),
            after: after.map(str::to_owned),
            status,
            analysis,
            changed_functions: count,
        });
    };
    for (path, entry) in &base.git.entries {
        let renamed = renames.get(&PathBuf::from(path)).map(|p| path_text(p));
        let target = renamed.as_deref().unwrap_or(path);
        if let Some(other) = head.git.entries.get(target) {
            paired.insert(target.to_owned());
            if path != target || entry != other {
                add(
                    Some(path),
                    Some(target),
                    if path != target {
                        "renamed"
                    } else {
                        "modified"
                    },
                );
            }
        } else {
            add(Some(path), None, "removed");
        }
    }
    for path in head.git.entries.keys().filter(|p| !paired.contains(*p)) {
        add(None, Some(path), "added");
    }
    files
}

#[derive(Default)]
struct Selection {
    roles: BTreeSet<String>,
    full: bool,
    origin: Option<ContextOrigin>,
}
fn select(
    selected: &mut BTreeMap<String, Selection>,
    id: &str,
    role: &str,
    full: bool,
    limit: usize,
    origin: Option<ContextOrigin>,
) -> bool {
    if selected.len() >= limit && !selected.contains_key(id) {
        return false;
    }
    let s = selected.entry(id.into()).or_insert_with(|| Selection {
        origin,
        ..Selection::default()
    });
    s.roles.insert(role.into());
    s.full |= full;
    true
}

fn push_relation(report: &mut ContextReport, edge: ContextEdge, seeds: &BTreeSet<String>) {
    let incident = seeds.contains(&edge.from)
        || edge.to.as_ref().is_some_and(|id| seeds.contains(id))
        || edge.candidates.iter().any(|id| seeds.contains(id));
    if !report.limits.all_relations && !incident {
        *report
            .omissions
            .entry("context-relations".into())
            .or_default() += 1;
    } else if report.relations.len() >= report.limits.max_edges {
        *report.omissions.entry("relation-limit".into()).or_default() += 1;
    } else {
        report.relations.push(edge);
    }
}

type ContextGroup = (String, Selection, Vec<(String, Selection)>);
type Identity = (PathBuf, String, SymbolKind, String);
fn identity(symbol: &ProjectSymbol) -> Identity {
    (
        symbol.path.clone(),
        symbol.id.qualified_name.clone(),
        symbol.id.kind,
        signature(symbol).into(),
    )
}

/// Sharing requires unique identity in each snapshot AND identical source and
/// declaration spans. Equal text alone never establishes symbol identity.
fn group_context(
    mut selected: BTreeMap<String, Selection>,
    lookup: &BTreeMap<&str, &ContextSnapshot>,
    changed: &BTreeSet<String>,
    head: &str,
) -> Vec<ContextGroup> {
    let mut counts = BTreeMap::new();
    for (id, snapshot) in lookup {
        *counts
            .entry((
                snapshot.git.commit.as_str(),
                identity(&snapshot.symbols[*id]),
            ))
            .or_insert(0usize) += 1;
    }
    let mut candidates = BTreeMap::<Identity, Vec<String>>::new();
    for id in selected.keys() {
        let Some(snapshot) = lookup.get(id.as_str()) else {
            continue;
        };
        let symbol = &snapshot.symbols[id];
        let key = identity(symbol);
        if !changed.contains(id)
            && !matches!(symbol.link_status.as_str(), "ambiguous" | "unavailable")
            && counts.get(&(snapshot.git.commit.as_str(), key.clone())) == Some(&1)
            && snapshot.code(symbol).is_some()
        {
            candidates.entry(key).or_default().push(id.clone());
        }
    }
    let mut groups = Vec::new();
    for ids in candidates.values_mut() {
        if ids.len() != 2 {
            continue;
        }
        ids.sort_by_key(|id| (lookup[id.as_str()].git.commit != head, id.clone()));
        let a = lookup[ids[0].as_str()];
        let b = lookup[ids[1].as_str()];
        let sa = &a.symbols[&ids[0]];
        let sb = &b.symbols[&ids[1]];
        if a.git.commit == b.git.commit
            || a.code(sa) != b.code(sb)
            || a.declaration_codes(sa, &BTreeMap::new())
                != b.declaration_codes(sb, &BTreeMap::new())
            || sa.declarations.len() != sb.declarations.len()
            || sa.link_status != sb.link_status
            || sa.definition.is_some() != sb.definition.is_some()
            || sa.signature.as_ref().map(|s| &s.display)
                != sb.signature.as_ref().map(|s| &s.display)
        {
            continue;
        }
        let (Some(mut primary), Some(secondary)) =
            (selected.remove(&ids[0]), selected.remove(&ids[1]))
        else {
            continue;
        };
        primary.full |= secondary.full;
        primary.roles.extend(secondary.roles.iter().cloned());
        groups.push((ids[0].clone(), primary, vec![(ids[1].clone(), secondary)]));
    }
    groups.extend(
        selected
            .into_iter()
            .map(|(id, selection)| (id, selection, Vec::new())),
    );
    groups.sort_by_key(|(id, selection, _)| {
        (
            !selection.roles.contains("changed"),
            !selection.roles.contains("caller"),
            id.clone(),
        )
    });
    groups
}

fn source_location(
    snapshot: &ContextSnapshot,
    path: &std::path::Path,
    span: SourceSpan,
) -> SourceLocation {
    SourceLocation {
        reference: reference(&snapshot.git, path, span).unwrap_or_default(),
        commit: snapshot.git.commit.clone(),
        path: path_text(path),
        start_line: span.start.line,
        end_line: span.end.line,
    }
}

pub fn assemble_context(
    base: Option<&ContextSnapshot>,
    head: &ContextSnapshot,
    renames: &BTreeMap<PathBuf, PathBuf>,
    anchor: Option<&str>,
    limits: ContextLimits,
) -> ContextReport {
    let all_changes = base.map(|b| changes(b, head, renames)).unwrap_or_default();
    let files = base
        .map(|b| changed_files(b, head, renames, &all_changes))
        .unwrap_or_default();
    let mut report = ContextReport {
        schema_version: SCHEMA_VERSION,
        base: base.map(|b| b.git.commit.clone()),
        head: head.git.commit.clone(),
        language: "cpp",
        analyzers: head.analyzers.clone(),
        files,
        changes: Vec::new(),
        symbols: Vec::new(),
        relations: Vec::new(),
        limits,
        omissions: BTreeMap::new(),
        limitations: vec![
            "written-calls-only; macro, conditional, template and dynamic targets may be missing".into(),
            "C++ snapshots use tracked includes only; compilation databases and untracked generated files are not consulted".into(),
            "supporting types are signature-name candidates, not semantic type resolution; aliases and member-type closure are not expanded".into(),
            "cross-revision matching uses path/rename, name and unique signatures; ambiguous changes remain unpaired".into(),
            "changed files without changed functions may contain type, global, include, comment, or unsupported changes; no semantic impact inference is performed".into(),
        ],
        diagnostics: Vec::new(),
    };
    let mut selected = BTreeMap::<String, Selection>::new();
    let mut roots = Vec::new();
    let mut changed = BTreeSet::new();
    let mut counterparts = BTreeMap::new();
    for change in all_changes {
        let ids: Vec<_> = change.before.iter().chain(change.after.iter()).collect();
        for id in &ids {
            changed.insert((*id).clone());
        }
        if let (Some(b), Some(h)) = (&change.before, &change.after) {
            counterparts.insert(b.clone(), h.clone());
            counterparts.insert(h.clone(), b.clone());
        }
        if selected.len() + ids.len() > report.limits.max_symbols {
            *report
                .omissions
                .entry("changed-symbol-limit".into())
                .or_default() += ids.len();
            continue;
        }
        for id in ids {
            select(
                &mut selected,
                id,
                "changed",
                true,
                report.limits.max_symbols,
                None,
            );
            roots.push(id.clone());
        }
        report.changes.push(change);
    }
    if let Some(id) = anchor {
        select(
            &mut selected,
            id,
            "requested",
            true,
            report.limits.max_symbols,
            None,
        );
        roots.push(id.into());
    }
    let snapshots: Vec<_> = base.into_iter().chain(std::iter::once(head)).collect();
    let lookup: BTreeMap<_, _> = snapshots
        .iter()
        .flat_map(|snapshot| {
            snapshot
                .symbols
                .keys()
                .map(move |id| (id.as_str(), *snapshot))
        })
        .collect();
    let seeds: BTreeSet<_> = roots.iter().cloned().collect();
    let mut queue: VecDeque<_> = roots.into_iter().map(|id| (id, 0)).collect();
    let mut visited = BTreeSet::new();
    let mut used_edges = BTreeSet::new();
    let mut excluded_nodes = BTreeSet::new();
    while let Some((id, depth)) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let Some(snapshot) = lookup.get(id.as_str()) else {
            continue;
        };
        for (index, edge) in snapshot
            .relations
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from == id || e.to.as_deref() == Some(id.as_str()))
        {
            if used_edges.insert((snapshot.git.commit.clone(), index)) {
                push_relation(&mut report, edge.clone(), &seeds);
            }
            let neighbor = if edge.from == id {
                edge.to
                    .as_ref()
                    .map(|to| (to, "callee", report.limits.include_callees))
            } else {
                Some((&edge.from, "caller", true))
            };
            if let Some((neighbor, role, full)) = neighbor {
                if depth >= report.limits.depth {
                    if !selected.contains_key(neighbor) {
                        excluded_nodes.insert(neighbor.clone());
                    }
                    continue;
                }
                if select(
                    &mut selected,
                    neighbor,
                    role,
                    full,
                    report.limits.max_symbols,
                    Some(ContextOrigin {
                        from: id.clone(),
                        role: role.into(),
                        resolution: edge.resolution.clone(),
                    }),
                ) {
                    queue.push_back((neighbor.clone(), depth + 1));
                } else {
                    excluded_nodes.insert(neighbor.clone());
                }
            }
        }
    }
    if !excluded_nodes.is_empty() {
        report.omissions.insert(
            "unexpanded-call-symbols".into(),
            excluded_nodes
                .iter()
                .filter(|id| !selected.contains_key(*id))
                .count(),
        );
    }
    // Supporting type matches are deliberately labeled inferred/ambiguous.
    let mut requested: Vec<_> = selected.keys().cloned().collect();
    // Prefer the changed/requested function when several signatures name a type.
    requested.sort_by_key(|id| (!seeds.contains(id), id.clone()));
    for id in requested {
        let Some(snapshot) = lookup.get(id.as_str()) else {
            continue;
        };
        let s = &snapshot.symbols[&id];
        let Some(sig) = &s.signature else {
            continue;
        };
        let types = sig
            .parameters
            .iter()
            .filter_map(|p| p.type_spelling.as_deref())
            .chain(sig.return_type.as_deref())
            .collect::<Vec<_>>()
            .join(" ");
        let tokens: BTreeSet<_> = types
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| !t.is_empty())
            .collect();
        let mut candidates = BTreeMap::<String, Vec<String>>::new();
        for (tid, t) in &snapshot.symbols {
            if type_symbol(t)
                && tokens.contains(t.id.qualified_name.rsplit("::").next().unwrap_or(""))
            {
                candidates
                    .entry(t.id.qualified_name.rsplit("::").next().unwrap_or("").into())
                    .or_default()
                    .push(tid.clone());
            }
        }
        for tids in candidates.into_values() {
            for tid in &tids {
                if !select(
                    &mut selected,
                    tid,
                    "type",
                    true,
                    report.limits.max_symbols,
                    Some(ContextOrigin {
                        from: id.clone(),
                        role: "type".into(),
                        resolution: if tids.len() == 1 {
                            "inferred"
                        } else {
                            "ambiguous"
                        }
                        .into(),
                    }),
                ) {
                    *report
                        .omissions
                        .entry("type-symbol-limit".into())
                        .or_default() += 1;
                }
            }
            push_relation(
                &mut report,
                ContextEdge {
                    snapshot: snapshot.git.commit.clone(),
                    from: id.clone(),
                    from_name: s.id.qualified_name.clone(),
                    to: if tids.len() == 1 {
                        Some(tids[0].clone())
                    } else {
                        None
                    },
                    to_name: if tids.len() == 1 {
                        Some(snapshot.symbols[&tids[0]].id.qualified_name.clone())
                    } else {
                        None
                    },
                    relation: "signature-type-name",
                    resolution: if tids.len() == 1 {
                        "inferred"
                    } else {
                        "ambiguous"
                    }
                    .into(),
                    candidates: tids,
                    path: path_text(&s.path),
                    line: s.span.start.line,
                    column: s.span.start.column,
                    expression: types.clone(),
                    reason: Some("lexical signature type name; not compiler-resolved".into()),
                },
                &seeds,
            );
        }
    }
    let mut budget = report.limits.max_code_bytes;
    // Changed bodies and direct callers receive the byte budget before optional callees.
    let order = group_context(selected, &lookup, &changed, &head.git.commit);
    for (id, selection, other) in order {
        let Some(snapshot) = lookup.get(id.as_str()) else {
            continue;
        };
        let s = &snapshot.symbols[&id];
        let hide_primary_declaration = s.definition.is_none() && !report.limits.show_declarations;
        let mut source = source_record(
            snapshot,
            &s.path,
            s.span,
            changed.contains(&id),
            !hide_primary_declaration && (selection.full || changed.contains(&id)),
            &mut budget,
        );
        if hide_primary_declaration {
            source.code = Code {
                state: "omitted",
                text: None,
                reason: Some("declaration-policy"),
            };
        }
        source.reference = id.clone();
        if base.is_none() {
            source.changed = None;
        } else if type_symbol(s) {
            let other = if snapshot.git.commit == head.git.commit {
                base
            } else {
                Some(head)
            };
            source.changed = Some(other.is_none_or(|other| {
                !other.symbols.values().any(|o| {
                    o.id.qualified_name == s.id.qualified_name
                        && o.id.kind == s.id.kind
                        && (o.path == s.path
                            || renames.get(&s.path) == Some(&o.path)
                            || renames.get(&o.path) == Some(&s.path))
                        && other.code(o) == snapshot.code(s)
                })
            }));
        }
        let mut declarations = Vec::new();
        for d in s
            .declarations
            .iter()
            .filter(|_| report.limits.show_declarations)
        {
            if d.path == s.path && d.span == s.span {
                continue;
            }
            let mut item = source_record(snapshot, &d.path, d.span, false, true, &mut budget);
            if changed.contains(&id) {
                item.changed = Some(
                    counterparts
                        .get(&id)
                        .and_then(|other| {
                            lookup
                                .get(other.as_str())
                                .map(|os| (*os, &os.symbols[other]))
                        })
                        .is_none_or(|(other, other_symbol)| {
                            let text = snapshot
                                .git
                                .files
                                .get(&d.path)
                                .and_then(|f| f.source.get(d.span.start_byte..d.span.end_byte));
                            !other_symbol.declarations.iter().any(|od| {
                                other.git.files.get(&od.path).and_then(|f| {
                                    f.source.get(od.span.start_byte..od.span.end_byte)
                                }) == text
                            })
                        }),
                );
            }
            if base.is_none() {
                item.changed = None;
            }
            declarations.push(item);
        }
        report.symbols.push(ContextSymbol {
            source,
            name: s.id.qualified_name.clone(),
            kind: s.id.kind.as_str().into(),
            signature: s
                .signature
                .as_ref()
                .filter(|_| !hide_primary_declaration)
                .map(|sig| sig.display.clone()),
            definition: s.definition.is_some(),
            link_status: s.link_status.as_str().into(),
            roles: selection.roles,
            origin: selection.origin,
            other_snapshots: other
                .into_iter()
                .map(|(alias, selection)| {
                    let snapshot = lookup[alias.as_str()];
                    let symbol = &snapshot.symbols[&alias];
                    OtherSnapshot {
                        source: source_location(snapshot, &symbol.path, symbol.span),
                        origin: selection.origin,
                        declarations: symbol
                            .declarations
                            .iter()
                            .filter(|d| {
                                report.limits.show_declarations
                                    && !(d.path == symbol.path && d.span == symbol.span)
                            })
                            .map(|d| source_location(snapshot, &d.path, d.span))
                            .collect(),
                    }
                })
                .collect(),
            declarations,
        });
    }
    for snapshot in snapshots {
        report.diagnostics.extend(
            snapshot
                .diagnostics
                .iter()
                .map(|d| format!("{}: {d}", snapshot.git.commit)),
        );
        for (_, reason) in &snapshot.git.excluded {
            *report.omissions.entry(reason.clone()).or_default() += 1;
        }
    }
    let code_omitted = report
        .symbols
        .iter()
        .flat_map(|s| std::iter::once(&s.source).chain(&s.declarations))
        .filter(|s| s.code.reason == Some("code-byte-limit"))
        .count();
    if code_omitted > 0 {
        report
            .omissions
            .insert("code-byte-limit".into(), code_omitted);
    }
    report.diagnostics.sort();
    report.diagnostics.dedup();
    report
}
fn source_record(
    snapshot: &ContextSnapshot,
    path: &std::path::Path,
    span: SourceSpan,
    changed: bool,
    include: bool,
    budget: &mut usize,
) -> SourceRecord {
    let text = snapshot
        .git
        .files
        .get(path)
        .and_then(|f| f.source.get(span.start_byte..span.end_byte));
    SourceRecord {
        reference: reference(&snapshot.git, path, span).unwrap_or_default(),
        commit: snapshot.git.commit.clone(),
        path: path_text(path),
        start_line: span.start.line,
        end_line: span.end.line,
        changed: Some(changed),
        code: Code::from_source(text, include, budget),
    }
}

pub fn render_context(report: &ContextReport) -> String {
    let mut out = String::new();
    let symbols: BTreeMap<_, _> = report
        .symbols
        .iter()
        .flat_map(|s| {
            std::iter::once((s.source.reference.as_str(), s)).chain(
                s.other_snapshots
                    .iter()
                    .map(move |alias| (alias.source.reference.as_str(), s)),
            )
        })
        .collect();
    let mut rendered = BTreeSet::new();
    for change in &report.changes {
        let name = change
            .after
            .as_deref()
            .or(change.before.as_deref())
            .and_then(|id| symbols.get(id))
            .map(|s| s.name.as_str())
            .unwrap_or("function");
        let reason = change
            .reason
            .map(|reason| format!("; {reason}"))
            .unwrap_or_default();
        out.push_str(&format!("{name} [{}{reason}]\n", change.status));
        for (label, id) in [("BEFORE", &change.before), ("AFTER", &change.after)] {
            if let Some(id) = id {
                if let Some(s) = symbols.get(id.as_str()) {
                    out.push_str(&format!("{label} "));
                    write_source(&mut out, &s.source);
                    for d in &s.declarations {
                        out.push_str("declaration ");
                        write_source(&mut out, d);
                    }
                    rendered.insert(id.as_str());
                }
            } else {
                out.push_str(&format!("{label} [absent]\n"));
            }
        }
        out.push('\n');
    }
    for s in &report.symbols {
        if rendered.contains(s.source.reference.as_str()) {
            continue;
        }
        out.push_str(&format!(
            "{} [{}; {}]\n",
            s.name,
            change_label(s.source.changed),
            s.roles.iter().cloned().collect::<Vec<_>>().join(",")
        ));
        write_source(&mut out, &s.source);
        write_origin(
            &mut out,
            &s.source.reference,
            s.origin.as_ref(),
            report,
            &symbols,
        );
        for other in &s.other_snapshots {
            out.push_str(&format!(
                "same source @{} {}:{}-{}\n",
                &other.source.commit[..12],
                other.source.path,
                other.source.start_line,
                other.source.end_line
            ));
            write_origin(
                &mut out,
                &other.source.reference,
                other.origin.as_ref(),
                report,
                &symbols,
            );
            for declaration in &other.declarations {
                out.push_str(&format!(
                    "same declaration @{} {}:{}-{}\n",
                    &declaration.commit[..12],
                    declaration.path,
                    declaration.start_line,
                    declaration.end_line
                ));
            }
        }
        if s.source.code.state != "included" {
            if let Some(sig) = &s.signature {
                out.push_str(&format!("  {sig}\n"));
            }
        }
        for d in &s.declarations {
            out.push_str("declaration ");
            write_source(&mut out, d);
        }
        out.push('\n');
    }
    if !report.relations.is_empty() {
        out.push_str("Relations\n");
    }
    for e in &report.relations {
        out.push_str(&format!(
            "  {} -> {} [{}; {}] {}:{} @{}\n",
            e.from_name,
            e.to_name.as_deref().unwrap_or(&e.expression),
            e.relation,
            e.resolution,
            e.path,
            e.line,
            &e.snapshot[..12]
        ));
    }
    for file in report.files.iter().filter(|f| f.changed_functions == 0) {
        out.push_str(&format!(
            "file {} [{}; {}; no changed function body]\n",
            file.after
                .as_deref()
                .or(file.before.as_deref())
                .unwrap_or(""),
            file.status,
            file.analysis
        ));
    }
    for (kind, count) in &report.omissions {
        if *count > 0 {
            let option = if kind == "context-relations" {
                " (--all-relations)"
            } else {
                ""
            };
            out.push_str(&format!("omitted {kind}={count}{option}\n"));
        }
    }
    for diagnostic in &report.diagnostics {
        out.push_str(&format!("! {diagnostic}\n"));
    }
    out
}
fn write_origin(
    out: &mut String,
    reference: &str,
    origin: Option<&ContextOrigin>,
    report: &ContextReport,
    symbols: &BTreeMap<&str, &ContextSymbol>,
) {
    let Some(origin) = origin else {
        return;
    };
    let visible = report
        .relations
        .iter()
        .any(|edge| match origin.role.as_str() {
            "caller" => edge.from == reference && edge.to.as_deref() == Some(origin.from.as_str()),
            _ => {
                edge.from == origin.from
                    && (edge.to.as_deref() == Some(reference)
                        || edge.candidates.iter().any(|id| id == reference))
            }
        });
    if !visible {
        if let Some(parent) = symbols.get(origin.from.as_str()) {
            // The reference itself identifies the original snapshot, including aliases.
            let commit = origin.from.split(':').nth(1).unwrap_or("");
            out.push_str(&format!(
                "via {} [{}; {}] @{}\n",
                parent.name,
                origin.role,
                origin.resolution,
                &commit[..commit.len().min(12)]
            ));
        }
    }
}

fn write_source(out: &mut String, s: &SourceRecord) {
    out.push_str(&format!(
        "{}:{}-{} [{}] @{}\n",
        s.path,
        s.start_line,
        s.end_line,
        change_label(s.changed),
        &s.commit[..12]
    ));
    if let Some(text) = &s.code.text {
        for (i, line) in text.lines().enumerate() {
            out.push_str(&format!("{:>4}  {line}\n", s.start_line + i));
        }
    } else {
        out.push_str(&format!(
            "code [{}: {}]\n",
            s.code.state,
            s.code.reason.unwrap_or("unspecified")
        ));
    }
}

fn change_label(changed: Option<bool>) -> &'static str {
    match changed {
        Some(true) => "changed",
        Some(false) => "unchanged",
        None => "context",
    }
}
