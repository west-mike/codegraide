# codegraide

codegraide is an early-stage Rust CLI for understanding source repositories through code-quality metrics, dependency graphs, change hotspots, and database ERDs.

The end goal is to give developers and automated agents enough measurable evidence to identify good and bad code, explain why, and find the code that deserves attention.

The project is currently being developed and is in its pre-alpha stage.

## Workspace

- `crates/cli`: command-line interface, presentation, and exit behavior
- `crates/core`: analysis domain, parsers, metrics, graphs, and reports
- `crates/analyzers/python`: the first Tree-sitter language analyzer

The project is in its initial foundation stage. User-facing behavior and contribution documentation will expand as the first repository-inventory milestone is implemented.

## Repository inventory

Inventory the current directory:

```sh
cargo run -p codegraide -- inventory .
```

The report counts inventoried files and separates them into source, documentation, configuration, data, assets, and uncategorized categories. Use `--list-files` to print the paths in one or more categories:

```sh
cargo run -p codegraide -- inventory . \
  --list-files source \
  --list-files uncategorized
```

Use `--list-files all` to print every category.

Recognized source files also report physical source, comment, and blank lines.
The current line-count implementation covers Python, Rust, C, and C++. Strings
and docstrings count as source; comment-only lines count as comments; empty lines
inside block comments count as comments.

The categories are a partition of physical lines: `total = source + comment + blank`.
Files with an unsupported language remain in inventory but are not included in
line measurements.

Machine-readable output is versioned separately from the codegraide package:

```sh
cargo run -p codegraide -- inventory . --format json
```

The JSON report includes category paths, language counts, ignored populations,
line counts, diagnostics, and `report_schema_version: "0.1.0"`. It is sorted and
does not include timestamps or absolute repository paths, so it is suitable for
fixtures and other automated consumers. Non-UTF-8 Unix path bytes are percent-
encoded and described by `inventory.path_encoding`. Warnings are included in
`diagnostics`; `--no-warnings` removes them. `--list-files` is a terminal-only
option.

Repository `.gitignore` files are respected. To include selected ignored files, repeat `--include-ignored` with repository-relative globs:

```sh
cargo run -p codegraide -- inventory . \
  --include-ignored 'generated/**' \
  --include-ignored 'vendor/**/*.py'
```

Included files are combined with normal discovery results and counted once when patterns overlap. Built-in `.git`, `target`, and `__pycache__` exclusions cannot be overridden.

Ignored file and directory counts are reported separately. Ignored directories are not walked by default, so their contents are not included in the ignored file count. Use `--audit-ignored` when exact ignored paths and counts are needed:

```sh
cargo run -p codegraide -- inventory . --audit-ignored
```

## Category rules

Default category rules ship with codegraide. Additional JSON rulesets are loaded explicitly and may be layered:

```sh
cargo run -p codegraide -- inventory . \
  --config rules/company.json \
  --config rules/repository.json
```

A ruleset can classify files by extension, exact filename, or filename regex:

```json
{
  "config_version": "0.1.0",
  "inventory": {
    "ignore_defaults": false,
    "categories": {
      "source": {
        "include_extensions": ["go"],
        "include_filename_regexes": [".*\\.generated\\.rs"],
        "exclude_filename_regexes": ["^private/.*\\.generated\\.rs"]
      }
    }
  }
}
```

The supported categories are `source`, `documentation`, `configuration`, `data`, `assets`, and `uncategorized`.

Rules extend the defaults unless a category sets `"mode": "replace"`. Setting `"ignore_defaults": true` replaces all rules loaded before that file. Regexes containing `/` match repository-relative paths; other regexes match filenames in any directory.

Invalid configuration is an error. Nonfatal warnings explain valid but potentially surprising rule interactions. Use `--no-warnings` to suppress those warnings. Errors cannot be suppressed.

## Syntax analysis

The `analyze` command runs the analyzers available for languages found in the
inventory. It scans the current directory by default, or accepts a file or
directory path:

```sh
cargo run -p codegraide -- analyze .
cargo run -p codegraide -- analyze src/service.py
```

The initial analyzer parses Python with Tree-sitter and reports modules,
classes, functions, methods, decorators, parameters, syntactic imports, and
basic function measurements. A valid syntax tree is `successful`; a tree
containing parser recovery nodes is `partial`. Parse diagnostics are evidence
of syntax recovery only—they are not lint, type, or code-quality findings.
Languages that inventory recognizes but cannot analyze are still shown as
`inventory only`.

Use a full-match repository-relative regex to select files, and repeat it for
OR selection:

```sh
cargo run -p codegraide -- analyze . --match 'src/.*\\.py'
```

Ignored files can be selected with the same targeted approach as inventory:

```sh
cargo run -p codegraide -- analyze . --include-ignored 'generated/**/*.py'
```

Terminal output summarizes files with diagnostics without dumping every parser
message. Add `--diagnostics` to print all details, or pass one or more exact
selected file paths to print only those files. Use `--details` for complete
symbols, imports, and measurements, optionally restricted to exact files. JSON
always contains every fact and diagnostic and includes analyzer versions, grammar and query
provenance, capability claims, source spans, file statuses, measurements, and
limitations:

```sh
cargo run -p codegraide -- analyze . --diagnostics
cargo run -p codegraide -- analyze . --diagnostics src/service.py
cargo run -p codegraide -- analyze . --details src/service.py
cargo run -p codegraide -- analyze . --format json
```

The syntax report uses the independent `syntax-analysis-v1` definition and the
same `0.1.0` report schema version as inventory. Symbols, imports, project
resolution, installed-package lookup, and runtime-complexity claims are
deliberately not made yet. Module identities are repository-relative paths
until a Python interpreter and project environment are explicitly selected.
