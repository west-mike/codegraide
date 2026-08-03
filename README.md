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
line counts, diagnostics, and `report_schema_version: "0.2.0"`. It is sorted and
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

The syntax report uses the independent `syntax-analysis-v2` definition and the
same `0.2.0` report schema version as inventory. Symbols, imports, project
resolution, installed-package lookup, and runtime-complexity claims are
deliberately not made yet. Module identities are repository-relative paths
until a Python interpreter and project environment are explicitly selected.

## Deterministic review gate

Syntax analysis also produces a review evaluation for the selected snapshot.
It is evidence for an agent or reviewer, not a universal code-quality score.
The default policy reports human review when a callable's Python cyclomatic
complexity is 11 or higher. Risk bands are low (1-5), moderate (6-10), high
(11-20), and critical (21+). A finding uses `risk` for the measured band and
`required_action` for the policy result: `none`, `human-review`, or `block`.
The overall status is `pass`, `human-review-required`, or `blocked`.

Ordinary `analyze` remains informational and exits successfully after a
completed analysis. Add `--gate` when an automation needs policy exit codes:
0 means pass, 2 means human review is required, and 3 means the configured
policy blocks. Operational, input, policy, and serialization errors use exit
code 1.

```sh
cargo run -p codegraide -- analyze . --format json --gate
cargo run -p codegraide -- analyze . --gate --complexity-block-at 21
cargo run -p codegraide -- analyze . --policy review-policy.json --gate

# Compact agent review report: policy, coverage, findings, and top rankings
cargo run -p codegraide -- analyze . --format json --profile review --top 20 --gate

# Minimal CI/orchestration report
cargo run -p codegraide -- analyze . --format gate --top 10 --gate
```

`--format json` keeps the full analysis report by default, including symbols,
decision events, dependencies, and diagnostics. Use `--profile review` to
return only the review-oriented projection. The separate `gate` format is
smaller still: it reports status, the process-equivalent exit code, total
finding count, and a bounded list of top findings. `--top` limits compact
rankings/findings; review defaults to 20 and gate defaults to 10 when omitted.
The full report remains available for targeted follow-up investigations.

The optional policy file is selected explicitly and can configure thresholds,
risk bands, and documented exceptions. CLI threshold options override matching
policy-file values. A bounded exception acknowledges a named symbol only up to
`approved_max`; an `unbounded: true` exception suppresses the action while
keeping the ranking and evidence visible. Every exception requires a reason.

```json
{
  "policy_version": "0.1.0",
  "cyclomatic_complexity": {
    "human_review_at": 11,
    "block_at": 21,
    "risk_bands": {"moderate_at": 6, "high_at": 11, "critical_at": 21},
    "exceptions": [
      {
        "symbol_id": "src/legacy.py::function:legacy#1",
        "reason": "Reviewed parser generated from an external protocol",
        "approved_max": 24
      }
    ]
  }
}
```

For Python callables, the implemented metric is `1 + decision events`. The
explicit v1 rules count `if`/`elif`, `for`/`while`, exception handlers,
refutable `match` cases, match guards, boolean short-circuit expressions,
conditional expressions, each comprehension loop/filter, and `assert`.
`else`, `with`, `try` itself, `finally`, and unconditional match cases do not
add a point. Named functions, methods, nested functions, and lambdas are
measured independently; module and class bodies are outside this v1 scope.
Every counted event is emitted with a source span in JSON so an agent can cite
the exact evidence instead of treating the score as a verdict.
