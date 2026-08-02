use std::fs;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codegraide"))
        .args(arguments)
        .output()
        .expect("codegraide binary should run")
}

#[test]
fn default_terminal_analysis_reports_summary_without_diagnostic_details() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(repository.path().join("good.py"), "value = 1\n").expect("valid fixture");
    fs::write(repository.path().join("bad.py"), "def broken(\n").expect("malformed fixture");

    let output = run(&["analyze", repository.path().to_str().unwrap()]);
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(stdout.contains("Analyzers:"));
    assert!(stdout.contains("analyzed=2 successful=1 partial=1 failed=0"));
    assert!(stdout.contains("diagnostics: bad.py (1)"));
    assert!(!stdout.contains("parser could not interpret"));
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn diagnostics_flag_prints_details_and_exact_file_selection() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(repository.path().join("good.py"), "value = 1\n").expect("valid fixture");
    fs::write(repository.path().join("bad.py"), "def broken(\n").expect("malformed fixture");
    let root = repository.path().to_str().unwrap();

    let all = run(&["analyze", root, "--diagnostics"]);
    let all_stdout = String::from_utf8(all.stdout).expect("stdout should be UTF-8");
    assert!(all.status.success());
    assert!(all_stdout.contains("error[parse-error]"));
    assert!(all_stdout.contains("good.py:\n    (none)"));

    let selected = run(&["analyze", root, "--diagnostics", "bad.py"]);
    let selected_stdout = String::from_utf8(selected.stdout).expect("stdout should be UTF-8");
    assert!(selected.status.success());
    assert!(selected_stdout.contains("bad.py:"));
    assert!(!selected_stdout.contains("good.py:"));
}

#[test]
fn json_analysis_contains_provenance_spans_and_inventory_only_languages() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(
        repository.path().join("main.py"),
        "def main(value):\n    return value\n",
    )
    .expect("Python fixture");
    fs::write(repository.path().join("main.rs"), "fn main() {}\n").expect("Rust fixture");

    let output = run(&[
        "analyze",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(report["report_schema_version"], "0.1.0");
    assert_eq!(report["analysis"]["kind"], "syntax-analysis");
    assert_eq!(
        report["analysis"]["definition_version"],
        "syntax-analysis-v1"
    );
    assert_eq!(report["inventory"]["inventory_only_languages"]["rust"], 1);
    assert_eq!(
        report["analyzers"][0]["grammar"]["name"],
        "tree-sitter-python"
    );
    assert_eq!(
        report["analyzers"][0]["queries"][0]["version"],
        "python-symbols-v1"
    );
    assert_eq!(report["analyzers"][0]["files"][0]["status"], "successful");
    let symbols = report["analyzers"][0]["files"][0]["symbols"]
        .as_array()
        .expect("symbols should be an array");
    assert!(symbols.iter().any(|symbol| symbol["kind"] == "module"));
    assert!(symbols.iter().any(|symbol| {
        symbol["kind"] == "function"
            && symbol["measurements"]
                .as_array()
                .is_some_and(|measurements| {
                    measurements
                        .iter()
                        .any(|measurement| measurement["status"] == "measured")
                })
    }));
}

#[test]
fn include_ignored_selects_python_files_and_match_is_full_path() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(repository.path().join(".gitignore"), "generated/\n").expect("ignore fixture");
    fs::create_dir(repository.path().join("generated")).expect("ignored directory");
    fs::write(
        repository.path().join("generated/ignored.py"),
        "value = 1\n",
    )
    .expect("ignored Python fixture");

    let root = repository.path().to_str().unwrap();
    let without = run(&["analyze", root]);
    let without_stdout = String::from_utf8(without.stdout).expect("stdout should be UTF-8");
    assert!(without.status.success());
    assert!(without_stdout.contains("no selected files have a registered syntax analyzer"));

    let with = run(&[
        "analyze",
        root,
        "--include-ignored",
        "generated/**",
        "--match",
        r"generated/ignored\.py",
    ]);
    let with_stdout = String::from_utf8(with.stdout).expect("stdout should be UTF-8");
    assert!(with.status.success());
    assert!(with_stdout.contains("analyzed=1 successful=1 partial=0 failed=0"));
}

#[test]
fn diagnostics_cannot_be_combined_with_json() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(repository.path().join("main.py"), "value = 1\n").expect("Python fixture");

    let output = run(&[
        "analyze",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
        "--diagnostics",
    ]);
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(!output.status.success());
    assert!(stderr.contains("--diagnostics is only available with terminal output"));
}

#[test]
fn a_file_target_analyzes_only_that_file_and_diagnostic_paths_are_repeatable() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::create_dir(repository.path().join("src")).expect("source directory");
    fs::write(repository.path().join("src/one.py"), "value = 1\n").expect("Python fixture");
    fs::write(repository.path().join("src/two.py"), "def broken(\n").expect("Python fixture");

    let file = repository.path().join("src/two.py");
    let output = run(&["analyze", file.to_str().unwrap(), "--diagnostics", "two.py"]);
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(output.status.success());
    assert!(stdout.contains("Selected files: 1"));
    assert!(stdout.contains("two.py:"));
    assert!(!stdout.contains("one.py"));

    let root = repository.path().to_str().unwrap();
    let multiple = run(&[
        "analyze",
        root,
        "--diagnostics",
        "src/one.py",
        "--diagnostics",
        "src/two.py",
    ]);
    let multiple_stdout = String::from_utf8(multiple.stdout).expect("stdout should be UTF-8");
    assert!(multiple.status.success());
    assert!(multiple_stdout.contains("src/one.py:"));
    assert!(multiple_stdout.contains("src/two.py:"));
}

#[test]
fn details_flag_prints_facts_for_only_the_requested_file() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(
        repository.path().join("one.py"),
        "def one(value):\n    return value\n",
    )
    .expect("Python fixture");
    fs::write(repository.path().join("two.py"), "import os\n").expect("Python fixture");

    let output = run(&[
        "analyze",
        repository.path().to_str().unwrap(),
        "--details",
        "one.py",
    ]);
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(output.status.success());
    assert!(stdout.contains("Details:"));
    assert!(stdout.contains("one.py:"));
    assert!(stdout.contains("function one"));
    assert!(!stdout.contains("two.py:"));
}
