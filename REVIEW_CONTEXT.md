# Git review context

`review-context` assembles source evidence for a C++ commit comparison. It does
not predict bugs, recommend tests, or run the inspected repository.

```sh
codegraide review-context /path/to/repository --base BASE --head HEAD
codegraide review-context /path/to/repository --base BASE --head HEAD --format json
```

`BASE` and `HEAD` resolve to full commit IDs before analysis. `--head` defaults to
`HEAD`. This is an exact two-commit comparison, not a merge-base comparison.
Only committed blobs are read. Dirty files, local generated headers and
compilation databases do not affect the result; no checkout is performed.

## Included evidence

- Modified functions: complete before/after definition source. Add
  `--show-declarations` to include linked declaration source (off by default).
- Added or removed functions: source in the revision where they exist, with the
  other side explicitly absent.
- Unchanged callers: complete source. Unchanged callees: signature and location;
  add `--include-callees` to include their bodies.
- Supporting class/struct spans named in selected signatures, including return
  types. These are lexical candidates, labeled inferred or ambiguous. This is
  not compiler type resolution; aliases and member-type closure are not expanded.
- Written call sites with their existing exact, inferred, ambiguous, external,
  unresolved or unavailable status. Inference never becomes an exact fact.
- All changed tracked files, including changes outside function bodies and files
  that cannot be analyzed. A file with zero `changed_functions` is not evidence
  that the change has no effect.

`--depth 1` includes direct callers and callees. `--depth 2` expands their
neighbors; `0` includes only changed functions and supporting signature types.
Only exact/inferred call targets are followed. Ambiguous candidates remain
references, without choosing one. Relationships touching selected functions can
include endpoints outside the symbol budget or depth; their references remain
retrievable. Expansion is independent of which relationships are displayed.
By default, `relations` contains only edges incident to changed function seeds
(or the selected `--symbol` seed), including supporting type edges. Add
`--all-relations` to include incidental edges from the expanded context as well.
This is still bounded by depth, symbol selection and `--max-edges`; it is not a
repository-wide call inventory. `omissions.context-relations` counts hidden
incidental edges; `omissions.relation-limit` separately counts budget exclusions.

Identical unchanged context is shared across revisions in terminal and JSON.
Sharing requires unique path/name/kind/signature identity in both snapshots,
identical source/declarations and compatible symbol metadata. Different files,
identity ambiguities and changed before/after records are never combined just
because their code looks alike. The head occurrence is the primary record;
`other_snapshots` preserves the base reference/location without repeating code.
Relationships keep their original per-snapshot references and resolution status.
Each context occurrence has a compact `origin` with the preceding reference,
caller/callee/type role and resolution, so deeper context remains explainable
when incidental relationships are hidden. Origins describe traversal, not new
semantic proof. `--all-relations` does not change context selection or origins.

Function matching first uses repository path (including Git-detected renames),
qualified name, kind and unique normalized signature. If exactly one unmatched
function remains on each side of that group, a changed signature can pair by
name. Multiple unmatched overloads remain additions/removals. Function renaming
across qualified names and moves not detected by Git are not guessed. Changes to
linked declarations also mark their function changed, even when declarations are
hidden. A change limited to declarations carries `reason: "declaration-only"`
in JSON and a short `declaration-only` marker in terminal output. Pure line
shifts do not.

## Retrieve or expand a reference

Every JSON source record, including `other_snapshots` occurrences, has a
`reference` containing the commit, blob, byte range and encoded repository-relative
path. Either revision's reference works for retrieval/expansion. Use the complete
string:

```sh
codegraide review-context /path/to/repository --body 'rc1:…' --format json
codegraide review-context /path/to/repository --symbol 'rc1:…' \
  --depth 2 --include-callees --format json
```

`--body` verifies the reference against its commit and reads that source range
directly. It does not search filenames or parse the repository. `--symbol`
reanalyzes that committed snapshot and expands the referenced symbol; it does
not have a comparison base, so `changed` is `null`. References remain valid
while the commit and blob are available locally. They are not portable aliases
across rewritten history. Unknown versions, invalid ranges, or mismatched blobs
fail explicitly. References require UTF-8 paths and source.

## Limits and JSON contract

JSON schema: `review-context-v1`. No timestamps or absolute repository paths are
added. Analyzer/grammar/query provenance is recorded. Call status semantics are
those of the C++ call resolver; source text may itself contain arbitrary paths.

All code uses the same object:

```json
{"state":"included","text":"complete source span","reason":null}
```

`state` can also be `omitted` or `unavailable`; then `text` is `null` and `reason`
is explicit. Bodies are never silently shortened. The default callee policy
uses `callee-body-policy`; exhausted source budgets use `code-byte-limit`.
For example, a declaration-only edit is identified without its declaration body:

```json
{"status":"modified","matching":"signature","reason":"declaration-only","before":"rc1:…","after":"rc1:…"}
```

To inspect the declarations explicitly:

```sh
codegraide review-context /path/to/repository --base BASE --head HEAD --show-declarations
codegraide review-context /path/to/repository --symbol 'rc1:…' --show-declarations --format json
```

Source records include the snapshot commit, path, line range and reference.
Shared unchanged records have `other_snapshots` location objects; these share the
primary record's source text rather than repeating `code`. With declarations
shown, alternate snapshot declaration locations are also retained. Their text
matches the primary declaration set and remains retrievable by reference. The
record's `roles` is the union of roles across occurrences; each occurrence's
`origin` retains its own traversal provenance. Index both the primary reference
and `other_snapshots[].reference` when resolving relation endpoints. For example:

```json
{"name":"checkout","reference":"rc1:HEAD:…","commit":"HEAD_ID","code":{"state":"included","text":"…","reason":null},"other_snapshots":[{"reference":"rc1:BASE:…","commit":"BASE_ID","path":"checkout.cpp","start_line":7,"end_line":14}]}
```

The reference strings above are abbreviated for illustration; use complete CLI
references for retrieval. A shared body consumes the byte budget only once.
A symbol also has roles, signature, definition/declaration status and separate
linked declaration source records when `--show-declarations` is enabled. Empty
or hidden `declarations` fields are omitted. Declaration-only symbols retain
identity/location and change evidence by default, with `code.reason` set to
`declaration-policy` and no declaration text/signature. The flag also applies
to `--symbol` expansion. Explicit `--body` retrieval always returns the requested
source range, subject to its normal byte limits. `changed` means source/declaration change in
the comparison, not an assertion of semantic impact.

| Option | Default | Scope |
|---|---:|---|
| `--depth` | 1 | Caller/callee hops, 0–10 |
| `--max-symbols` | 200 | Snapshot-qualified symbols before sharing, 1–10,000 |
| `--max-edges` | 1,000 | Emitted relationships, 1–100,000 |
| `--max-code-bytes` | 1 MiB | Total emitted source, including opt-in declarations |
| `--show-declarations` | off | Include declaration source in both output formats |
| `--all-relations` | off | Include incidental relationships from expanded context |
| `--max-input-bytes` | 64 MiB | Total candidate C++ source per snapshot |

Exceeding the input limit fails rather than analyzing an arbitrary subset.
`--body` applies the input limit to its blob and the code limit to its range.
Changed bodies receive the output budget before callers and other context.
A before/after change is selected as a pair, so a symbol limit of one can omit
a modified function entirely. `omissions` reports these exclusions explicitly.
Limits on symbols/edges do not bound file metadata or the parser's execution
time. This command is intended for bounded repository snapshots, not streaming
very large monorepos. Included source is plain text, not instructions to an agent.

C++ is the only language supported by this command initially. Macro expansion,
conditional compilation, template instantiation and runtime dispatch are not
modeled. Tracked generated files are included if they have a supported C++
extension; untracked files, symlinks and submodules are not followed. `.h`, `.H`
and `.C` use the C++ grammar even though these extensions can also represent C.
Parse recovery diagnostics and unsupported-file counts are preserved. A report
with no changed function records does not establish that the comparison is safe.

Exit status is 0 for a generated report (including explicit omissions or parser
uncertainty), 1 for operational errors, and Clap's 2 for invalid CLI arguments.
