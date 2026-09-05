# Adding a language to the graph explorers

Both tools use one offline shell. A language supplies normalized analysis and
small presentation hints; it does not copy an HTML page or implement controls.
Adding a presentation adapter does not add parser or resolver support.

## File loading contract

Dependency output is a directory with `index.html`, `python.html`, `cpp.html`
(and one additional page per selected resolver), plus the existing ownership
manifest. Each language page embeds only that language's graph. Links reload
another page; there is no combined graph payload, runtime fetch, CDN, or build
pipeline. A page can be copied and opened independently from disk.

Calls retain their existing single-language CLI selection and HTML file output.
Generate each language separately, for example:

```sh
codegraide calls PROJECT --language python --format html --output python-calls.html
codegraide calls PROJECT --language cpp --format html --output cpp-calls.html
```

Mixed-repository automatic call bundles are not introduced by this refactor.
Use distinct output paths to retain both reports. Source embedding is opt-in
with `--include-source`; it is available only in Call Graph Explorer.

## Shared code and tool boundaries

- `crates/core/src/explorer_shell.{rs,html}` composes one document, header,
  three-pane structure, search, toolbar and data block. Named slots contain
  trusted bundled assets. Escaped graph JSON is inserted last.
- `explorer_controls.css` owns shared color tokens, header, tabs, toolbar,
  resize handles, context menu and responsive shell styles.
- `explorer_runtime.js` owns depth synchronization, pointer/keyboard resizing,
  pointer cancellation and pane-toggle labels.
- `explorer_interactions.js` owns inspect/explore gestures and menus. Single
  click and Enter inspect; double click and Shift+Enter explore. Inspection
  never adds navigation history.
- `dependency_viewer.js`, `dependency_controls.js`, `dependency_explorer.js`
  own dependency layout, traversal, cycles, architecture and comparisons.
- `call_viewer.js` owns call layout, symbol navigation, evidence and source
  projections. `call_source.js` consumes normalized source spans and evidence.
- Each tool has CSS and navigator/canvas/details fragments, not a whole page.
  Language adapters do not contain markup.

The two layouts retain their existing coordinate systems, history contents and
node offsets. Their drag/pan redraws and reset behavior remain view-owned;
shared resizing delegates width constraints and redraws to each view. This
avoids treating a dependency group as a call symbol or changing graph truth.

## Dependency presentation

Implement and register the core `DependencyResolver` contract. Its descriptor
already declares `local_unit_kind`. The CLI passes the language ID and reported
unit kind to `DependencyExplorerPresentation::new`, then renders with
`render_dependency_html_with_presentation`. This works for empty graphs too;
the browser never guesses a language from the presence of file nodes.

```rust,ignore
let presentation = DependencyExplorerPresentation::new("new-language", "file");
let html = render_dependency_html_with_presentation(&view, query, &presentation)?;
```

`file` defaults to files/directories; `module` to modules/packages; other unit
kinds use units/groups. Public labels can be customized for the language.
The original renderer entry points remain available with built-in defaults.
The language's hierarchy rules must also be represented by the existing
normalized hierarchy builder. Current Python module/package segmentation is
in `html_hierarchy`; other languages currently use repository path segments.
Do not map a new language's import hierarchy to Python without evidence.

## Call presentation

Implement the analyzer and project-call-resolution contracts. The renderer
includes each local symbol's language ID. The shared browser view uses actual
module, architecture and call-flow data to expose optional perspectives.

`explorer_languages.js` is the small, statically shipped presentation adapter
map. To add syntax coloring, supply keyword/type sets, comment conventions,
optional block comments/preprocessor/triple strings, and the scope separator.
Extend its functions with a language-owned rule if the language needs more
than these proven conventions; do not add language branches to the view.
Ownership, direct-child display and support/noise heuristics live here too.

Python and C++ have adapters and fixtures. An unknown language gets escaped
plain-text source and unspecified scope, without guessed C++ highlighting or
containment. Source coloring is lightweight presentation, not semantic parsing.
Unknown-language plain text does not add token-level call marks. Exact source
spans and evidence remain available in the Calls panel.

This is static integration, not a public runtime JavaScript plugin protocol.
Stable analysis JSON schemas and metric definitions are unchanged.

## Verification for another language

Run the workspace format/check/Clippy/test sequence and:

```sh
node --test crates/core/tests/*.test.cjs
```

Add adapter fixtures for naming, comments/strings, escaping, missing optional
capabilities and source evidence. Generate a separate page alongside Python
and C++, and check that navigation does not embed the other payload. Browser
check inspection versus navigation, Back, source following inspection,
zoom-aware drag/pan, resize, keyboard menus, reset/recenter and a narrow layout.
The shared interaction tests do not replace real pointer checks.
