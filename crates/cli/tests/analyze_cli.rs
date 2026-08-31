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
    assert!(stdout.contains("Review status: Human review required."));
    assert!(stdout.contains("Review coverage: Measured callables:"));
    assert!(stdout.contains(", unavailable callables:"));
    assert!(stdout.contains(", unsupported files: 0."));
    assert!(stdout.contains("Documentation coverage: Partial."));
    assert!(stdout.contains("Languages:"));
    assert!(!stdout.contains("Inventory languages:"));
    assert!(!stdout.contains("human-review-required"));
    assert!(stdout.contains("analyzed: 2, successful: 1, partial: 1, failed: 0"));
    assert!(stdout.contains("diagnostics: bad.py (1)"));
    assert!(stdout.contains(
        "explicit exports: complete: 0, partial: 0, unavailable: 1, not declared: 1, names: 0"
    ));
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
        "__all__ = ['main']\nimport os\ndef main(value):\n    print(value)\n    return value\n",
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
    assert_eq!(report["report_schema_version"], "0.7.0");
    assert_eq!(report["analysis"]["kind"], "syntax-analysis");
    assert_eq!(
        report["analysis"]["definition_version"],
        "syntax-analysis-v5"
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
    assert_eq!(
        report["analyzers"][0]["files"][0]["explicit_exports"]["status"],
        "complete"
    );
    assert_eq!(
        report["analyzers"][0]["files"][0]["explicit_exports"]["names"][0]["name"],
        "main"
    );
    assert_eq!(
        report["analyzers"][0]["files"][0]["explicit_exports"]["names"][0]["span"]["start"]["line"],
        1
    );
    assert_eq!(report["documentation_coverage"]["status"], "complete");
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
    let dependency = &report["analyzers"][0]["files"][0]["dependencies"][0];
    assert_eq!(dependency["scope"], "module");
    assert_eq!(dependency["usage"], "runtime");
    assert_eq!(dependency["requirement"], "required");
    assert_eq!(dependency["conditional"], false);
    let call = &report["analyzers"][0]["files"][0]["calls"][0];
    assert_eq!(call["callee"], "print");
    assert_eq!(call["positional_arguments"], 1);
    assert_eq!(call["syntax_complete"], true);
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
    assert!(with_stdout.contains("analyzed: 1, successful: 1, partial: 0, failed: 0"));
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
fn json_review_reports_complexity_risk_and_gate_exit_codes() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(
        repository.path().join("simple.py"),
        "def simple(value):\n    return value\n",
    )
    .expect("simple fixture");
    fs::write(
        repository.path().join("complex.py"),
        "def complex(value):\n    if value: pass\n    if value: pass\n    if value: pass\n    if value: pass\n    if value: pass\n    if value: pass\n    if value: pass\n    if value: pass\n    if value: pass\n    if value: pass\n    return value\n",
    )
    .expect("complex fixture");
    let root = repository.path().to_str().unwrap();

    let json = run(&["analyze", root, "--format", "json"]);
    let report: Value = serde_json::from_slice(&json.stdout).expect("report should be JSON");
    assert!(json.status.success());
    assert_eq!(report["review"]["status"], "human-review-required");
    let rankings = report["review"]["rankings"]
        .as_array()
        .expect("rankings should be an array");
    assert_eq!(rankings[0]["qualified_name"], "complex");
    assert_eq!(rankings[0]["score"], 11);
    assert_eq!(rankings[0]["risk"], "high");
    assert_eq!(
        report["review"]["findings"][0]["required_action"],
        "human-review"
    );
    assert!(report["review"]["findings"][0]["severity"].is_null());

    let review = run(&[
        "analyze",
        root,
        "--format",
        "json",
        "--profile",
        "review",
        "--top",
        "1",
    ]);
    let review_report: Value =
        serde_json::from_slice(&review.stdout).expect("review report should be JSON");
    assert!(review.status.success());
    assert_eq!(review_report["report_schema_version"], "0.3.0");
    assert_eq!(review_report["review"]["status"], "human-review-required");
    assert!(review_report["analyzers"].is_null());
    assert_eq!(review_report["ranking_count"], 2);
    assert_eq!(review_report["finding_count"], 1);
    assert_eq!(
        review_report["review"]["rankings"]
            .as_array()
            .expect("review rankings should be an array")
            .len(),
        1
    );

    let gate_report_output = run(&["analyze", root, "--format", "gate", "--top", "1", "--gate"]);
    let gate_report: Value =
        serde_json::from_slice(&gate_report_output.stdout).expect("gate report should be JSON");
    assert_eq!(gate_report_output.status.code(), Some(2));
    assert_eq!(gate_report["report_schema_version"], "0.3.0");
    assert_eq!(gate_report["status"], "human-review-required");
    assert_eq!(gate_report["exit_code"], 2);
    assert_eq!(gate_report["finding_count"], 1);
    assert_eq!(gate_report["top_findings"][0]["unit"], "score");
    assert_eq!(
        gate_report["top_findings"]
            .as_array()
            .expect("gate findings should be an array")
            .len(),
        1
    );

    let gated = run(&["analyze", root, "--gate", "--format", "json"]);
    assert_eq!(gated.status.code(), Some(2));

    let blocked = run(&[
        "analyze",
        root,
        "--gate",
        "--complexity-block-at",
        "11",
        "--format",
        "json",
    ]);
    assert_eq!(blocked.status.code(), Some(3));
}

#[test]
fn policy_file_and_cli_threshold_override_are_reported() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(
        repository.path().join("sample.py"),
        "def sample(value):\n    if value: pass\n    if value: pass\n    return value\n",
    )
    .expect("Python fixture");
    let policy = repository.path().join("review-policy.json");
    fs::write(
        &policy,
        r#"{
            "policy_version": "0.1.0",
            "cyclomatic_complexity": {
                "human_review_at": 3,
                "risk_bands": {"moderate_at": 2, "high_at": 3, "critical_at": 5}
            }
        }"#,
    )
    .expect("policy fixture");
    let root = repository.path().to_str().unwrap();
    let output = run(&[
        "analyze",
        root,
        "--policy",
        policy.to_str().unwrap(),
        "--complexity-review-at",
        "4",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("report should be JSON");
    assert!(output.status.success());
    assert_eq!(report["review"]["policy"]["complexity_review_at"], 4);
    assert_eq!(report["review"]["policy"]["risk_bands"]["high_at"], 3);
    assert_eq!(
        report["review"]["policy"]["sources"],
        serde_json::json!(["built-in", "policy-file", "cli"])
    );
    assert_eq!(report["review"]["status"], "pass");
}

#[test]
fn policy_exception_acknowledges_a_bounded_callable_without_hiding_evidence() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(
        repository.path().join("legacy.py"),
        "def legacy(value):\n    if value: pass\n    if value: pass\n    return value\n",
    )
    .expect("Python fixture");
    let root = repository.path().to_str().unwrap();
    let initial = run(&["analyze", root, "--format", "json"]);
    let initial_report: Value =
        serde_json::from_slice(&initial.stdout).expect("initial report should be JSON");
    let symbol_id = initial_report["review"]["rankings"][0]["symbol_id"]
        .as_str()
        .expect("ranking should have a symbol id");
    let policy = repository.path().join("review-policy.json");
    fs::write(
        &policy,
        serde_json::to_string_pretty(&serde_json::json!({
            "policy_version": "0.1.0",
            "cyclomatic_complexity": {
                "human_review_at": 2,
                "exceptions": [{
                    "symbol_id": symbol_id,
                    "reason": "Legacy protocol parser reviewed by the team",
                    "approved_max": 3
                }]
            }
        }))
        .expect("policy should serialize"),
    )
    .expect("policy fixture");

    let output = run(&[
        "analyze",
        root,
        "--policy",
        policy.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("report should be JSON");
    assert!(output.status.success());
    assert_eq!(report["review"]["status"], "pass");
    assert_eq!(report["review"]["findings"][0]["acknowledged"], true);
    assert_eq!(report["review"]["findings"][0]["required_action"], "none");
    assert_eq!(report["review"]["rankings"][0]["score"], 3);
    assert_eq!(
        report["review"]["policy"]["exceptions"][0]["approved_max"],
        3
    );
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
        "__all__ = ['one']\ndef one(value):\n    return value\n",
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
    assert!(stdout.contains("explicit exports [complete]"));
    assert!(stdout.contains("\"one\": 1:12-1:17"));
    assert!(!stdout.contains("two.py:"));
}

#[test]
fn mixed_python_cpp_json_and_cpp_details_preserve_language_specific_facts() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(
        repository.path().join("main.py"),
        "import os\ndef python_entry(value):\n    return value\n",
    )
    .expect("Python fixture");
    fs::write(
        repository.path().join("native.cpp"),
        r#"#include <vector>
namespace demo {
struct Worker {
    int run(int value) {
        if (value && value > 1) {
            return value;
        }
        return 0;
    }
};
auto callback = [](int value) { return value ? value : 0; };
}
"#,
    )
    .expect("C++ fixture");
    let root = repository.path().to_str().unwrap();

    let output = run(&[
        "analyze",
        root,
        "--no-documentation-coverage",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("report should be JSON");
    assert!(output.status.success());
    assert_eq!(report["report_schema_version"], "0.7.0");
    let analyzers = report["analyzers"].as_array().expect("analyzers array");
    assert_eq!(analyzers.len(), 2);
    let cpp = analyzers
        .iter()
        .find(|run| run["language"] == "cpp")
        .expect("C++ analyzer run");
    assert_eq!(cpp["id"], "cpp-tree-sitter");
    assert!(
        cpp["measurement_definitions"]
            .as_array()
            .is_some_and(|definitions| definitions.iter().any(|definition| {
                definition["concept"] == "cyclomatic-complexity"
                    && definition["id"] == "cpp-cyclomatic-complexity"
                    && definition["definition_version"] == "cpp-cyclomatic-complexity-v1"
            }))
    );
    let file = cpp["files"]
        .as_array()
        .and_then(|files| files.iter().find(|file| file["path"] == "native.cpp"))
        .expect("native.cpp analysis");
    let symbols = file["symbols"].as_array().expect("symbols array");
    for kind in ["namespace", "struct", "method", "lambda"] {
        assert!(
            symbols.iter().any(|symbol| symbol["kind"] == kind),
            "missing {kind} symbol"
        );
    }
    let include = &file["dependencies"][0];
    assert_eq!(include["kind"], "include");
    assert_eq!(include["target"], "vector");
    assert_eq!(include["delimiter"], "angle");
    assert_eq!(include["conditional"], false);
    assert_eq!(include["resolution"], "syntactic");
    assert!(report["inventory"]["inventory_only_languages"]["cpp"].is_null());
    assert!(
        report["review"]["rankings"]
            .as_array()
            .is_some_and(|rankings| {
                rankings.iter().any(|ranking| {
                    ranking["language"] == "cpp"
                        && ranking["metric_id"] == "cpp-cyclomatic-complexity"
                        && ranking["metric_definition_version"] == "cpp-cyclomatic-complexity-v1"
                })
            })
    );

    let details = run(&["analyze", root, "--details", "native.cpp"]);
    let stdout = String::from_utf8(details.stdout).expect("terminal output should be UTF-8");
    assert!(details.status.success());
    assert!(stdout.contains("namespaces: 1"));
    assert!(stdout.contains("structs: 1"));
    assert!(stdout.contains("includes: 1"));
    assert!(stdout.contains("include vector [angle]"));
}

#[test]
fn cpp_complexity_gate_reports_metric_provenance_and_exit_code() {
    let repository = tempdir().expect("temporary repository should be created");
    fs::write(
        repository.path().join("gate.cpp"),
        "int gate(int value) { if (value) { return value; } return 0; }\n",
    )
    .expect("C++ fixture");

    let output = run(&[
        "analyze",
        repository.path().to_str().unwrap(),
        "--no-documentation-coverage",
        "--complexity-review-at",
        "2",
        "--gate",
        "--format",
        "gate",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("gate report should be JSON");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["report_schema_version"], "0.3.0");
    assert_eq!(report["status"], "human-review-required");
    assert_eq!(report["top_findings"][0]["language"], "cpp");
    assert_eq!(
        report["top_findings"][0]["metric_id"],
        "cpp-cyclomatic-complexity"
    );
    assert_eq!(
        report["top_findings"][0]["metric_definition_version"],
        "cpp-cyclomatic-complexity-v1"
    );
}
