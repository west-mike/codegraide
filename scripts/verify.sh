#!/bin/sh
# One verification entry point for contributors and CI.
set -eu
cd "$(dirname "$0")/.."
msrv=$(sed -n 's/^rust-version = "\([^"]*\)"/\1/p' Cargo.toml)
case "$msrv" in *.*.*) ;; *) msrv="$msrv.0" ;; esac
case "${1:-}" in
  --print-msrv) printf '%s\n' "$msrv" ;;
  --msrv) rustup run "$msrv" cargo check --workspace --all-targets --locked ;;
  '')
    cargo fmt --all --check
    cargo check --workspace --locked
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo test --workspace --locked
    node --test crates/core/tests/*.test.cjs
    ;;
  *) echo 'usage: scripts/verify.sh [--msrv|--print-msrv]' >&2; exit 2 ;;
esac
