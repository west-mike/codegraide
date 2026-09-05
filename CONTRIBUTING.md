# Contributing to codegraide

The project currently is focused on clarity, learning, and deterministic analysis rather than breadth of functionality.

Before proposing a feature, open an issue describing the user problem, intended behavior, explicit non-goals, error cases, and representative test inputs. Proposals should include their exact formula, unit, scope, aggregation method, provenance, and known limitations.

Keep changes small enough to explain. A useful contribution includes the user-visible behavior, domain logic, representative fixtures, error cases, and documentation in one reviewable slice.

Rust changes should pass:

```text
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Avoid introducing abstractions solely for hypothetical future languages or output formats.

Anything language-specific should go in it's respective analyzer. Core is meant to be as generic as we can make it.

Graph UI integration and per-language offline files are documented in
[EXPLORER_ADAPTERS.md](EXPLORER_ADAPTERS.md).
