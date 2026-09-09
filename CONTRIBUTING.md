# Contributing to codegraide

The project currently is focused on clarity, learning, and deterministic analysis rather than breadth of functionality.

Before proposing a feature, open an issue describing the user problem, intended behavior, explicit non-goals, error cases, and representative test inputs. Proposals should include their exact formula, unit, scope, aggregation method, provenance, and known limitations.

Keep changes small enough to explain. A useful contribution includes the user-visible behavior, domain logic, representative fixtures, error cases, and documentation in one reviewable slice.

Run the shared checks before submitting changes:

```sh
scripts/verify.sh
scripts/verify.sh --msrv
```

The first command checks Rust formatting, workspace compilation, strict Clippy,
Rust tests, and the offline explorer's JavaScript tests. It requires Rust with
rustfmt/Clippy and Node.js 22 or newer. The second checks all Rust targets using
the minimum version declared in `Cargo.toml`; install that toolchain with
`rustup toolchain install "$(scripts/verify.sh --print-msrv)" --profile minimal`.
GitHub runs these same commands for pushes and pull requests. Keep the compiler
minimum and locked dependencies compatible; passing on a newer compiler alone
is insufficient.

The optional C++ accuracy corpus requires independently reviewed labels and
pinned repositories. The normal suite deliberately leaves that gate ignored
until those inputs are available.

Avoid introducing abstractions solely for hypothetical future languages or output formats.

Language-specific rules belong in their respective analyzer. Core is meant to be as generic as we can make it.

Graph UI integration and per-language offline files are documented in
[EXPLORER_ADAPTERS.md](EXPLORER_ADAPTERS.md).
