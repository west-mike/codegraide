# codegraide

codegraide is an early-stage Rust CLI for understanding source repositories through code-quality metrics, dependency graphs, change hotspots, and database ERDs.

The project is intentionally being developed in small vertical slices so that its implementation remains understandable to a developer learning Rust.

## Workspace

- `crates/cli`: command-line interface, presentation, and exit behavior
- `crates/core`: analysis domain, parsers, metrics, graphs, and reports

The project is in its initial foundation stage. User-facing behavior and contribution documentation will expand as the first repository-inventory milestone is implemented.

## Repository inventory

Inventory the current directory:

```sh
cargo run -p codegraide -- inventory .
```

Repository `.gitignore` files are respected. To include selected ignored files, repeat `--include-ignored` with repository-relative globs:

```sh
cargo run -p codegraide -- inventory . \
  --include-ignored 'generated/**' \
  --include-ignored 'vendor/**/*.py'
```

Included files are combined with normal discovery results and counted once when patterns overlap. Built-in `.git`, `target`, and `__pycache__` exclusions cannot be overridden.
