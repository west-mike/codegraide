# codegraide

codegraide is a command-line tool for understanding codebases.
It measures function complexity and maps dependencies and function calls, with
graphs you can explore in your browser.

Current supported languages are C++ and Python, if you want any others, feel free to develop them and make a PR!

I wanted developers and coding agents to have a better way to judge the code
they're working on. If a function needs human review, test cases, documentation updates, I want to know which one,
why it was flagged, and what else depends on it.

This is a **pre-alpha** project. There are rough edges and will be numerous breaking changes to commands and
report formats.

## Install

You'll need [Rust and Cargo](https://rustup.rs/) (Rust 1.85 or newer) and Git.
Install from source:

```sh
git clone https://github.com/west-mike/codegraide.git
cd codegraide
cargo install --path crates/cli --locked
```

Check the installation with `codegraide --help`. If your shell can't find it,
make sure Cargo's bin directory (usually `~/.cargo/bin`) is on your `PATH`.

## Start with a repository

From a Python or C++ repository, run:

```sh
codegraide analyze .
```

The report shows which files were analyzed, ranks functions by complexity, and
flags functions for review. You can also pass a file or directory instead of `.`.
Repository `.gitignore` rules are respected.

To see how the code fits together, open a dependency graph:

```sh
codegraide dependencies . --format html --output codegraide-dependencies --open
```

Or explore which functions call each other, with source code alongside the graph:

```sh
codegraide calls . --format html --include-source --output calls.html --open
```

For a repository containing both Python and C++, add `--language python` or
`--language cpp` to `calls`. Click a node to inspect it; double-click to explore
its connections. It also works offline since it generates an HTML doc.
`--include-source` puts your code inside the report.

## More uses

```sh
# Count files and lines of code
codegraide inventory .

# Check Documentation coverage
codegraide comments .

# Get a compact JSON report for a coding agent or review workflow
codegraide analyze . --format json --profile review

# Compare the latest committed change with its surrounding code
codegraide review-context . --base HEAD~1 --head HEAD
```

Use `codegraide --help` to see the commands, then `codegraide <command> --help`
for all its options, examples, and config file formats. For example:

```sh
codegraide inventory --help
codegraide analyze --help
codegraide calls --help
```

Use `-h` instead of `--help` for a short option list.
[Git review context](REVIEW_CONTEXT.md) has more detail on comparing commits.

## Current limits

Rust and C currently have file inventory and line counts only. Call graphs
come from reading source code, not a compiler (intentional, might change in future); macros, dynamic calls, and parser errors can
leave missing or uncertain connections.
Try a smaller repository or directory first. Large C++ call graphs can use a
lot of memory.

## Contributing

Found a bug or a missing connection in a graph? Open an issue with a small
code example. Contributions for more languages are welcome too. See
[CONTRIBUTING.md](CONTRIBUTING.md) before starting a change.
