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

fn write_documentation_fixture(root: &std::path::Path) {
    fs::write(
        root.join("service.py"),
        r#""""Module documentation."""

class Service:
    """Service documentation."""

    def documented(self):
        """Document the method."""
        return 1

    def missing(self):
        return 2

    def _private(self):
        return 3

    class Nested:
        """Excluded nested class."""

def top_level():
    return 4

def outer():
    """Document the outer function."""
    def nested():
        return 5
    return nested()

callback = lambda: None
"#,
    )
    .expect("Python fixture");
}

#[test]
fn analyze_reports_documentation_by_default_and_can_disable_it() {
    let repository = tempdir().expect("temporary repository");
    write_documentation_fixture(repository.path());
    let root = repository.path().to_str().unwrap();

    let enabled = run(&["analyze", root, "--format", "json"]);
    let report: Value = serde_json::from_slice(&enabled.stdout).expect("analysis JSON");
    assert!(enabled.status.success());
    assert_eq!(report["report_schema_version"], "0.7.0");
    assert_eq!(report["documentation_coverage"]["status"], "complete");
    assert_eq!(report["documentation_coverage"]["counts"]["eligible"], 7);
    assert_eq!(report["documentation_coverage"]["counts"]["documented"], 4);
    assert_eq!(report["documentation_coverage"]["counts"]["missing"], 3);
    let serialized = String::from_utf8(enabled.stdout).expect("UTF-8 JSON");
    assert!(!serialized.contains("Document the method."));
    assert!(
        report["analyzers"][0]["files"][0]["symbols"]
            .as_array()
            .expect("symbols")
            .iter()
            .any(|symbol| symbol["documentation"]["status"] == "documented")
    );

    let disabled = run(&[
        "analyze",
        root,
        "--format",
        "json",
        "--no-documentation-coverage",
    ]);
    let report: Value = serde_json::from_slice(&disabled.stdout).expect("disabled JSON");
    assert!(disabled.status.success());
    assert_eq!(report["documentation_coverage"]["status"], "disabled");
    assert!(
        report["analyzers"][0]["files"][0]["symbols"]
            .as_array()
            .expect("symbols")
            .iter()
            .all(|symbol| symbol["documentation"].is_null())
    );
}

#[test]
fn comments_reports_top_level_coverage_in_terminal_and_json() {
    let repository = tempdir().expect("temporary repository");
    write_documentation_fixture(repository.path());
    let root = repository.path().to_str().unwrap();

    let terminal = run(&["comments", root, "--top", "1"]);
    let stdout = String::from_utf8(terminal.stdout).expect("terminal UTF-8");
    assert!(terminal.status.success());
    assert!(stdout.contains("Coverage: documented=4/7 missing=3 unavailable=0 (57.14%)"));
    assert!(stdout.contains("... 2 more"));

    let json = run(&["comments", root, "--format", "json"]);
    let report: Value = serde_json::from_slice(&json.stdout).expect("comments JSON");
    assert!(json.status.success());
    assert_eq!(report["report_schema_version"], "0.1.0");
    assert_eq!(report["analysis"]["kind"], "documentation-coverage");
    assert_eq!(
        report["analysis"]["definition_version"],
        "python-documentation-coverage-v1"
    );
    assert_eq!(
        report["documentation_coverage"]["counts"]["coverage_basis_points"],
        5714
    );
    assert_eq!(report["documentation_coverage"]["missing_symbol_count"], 3);
    assert_eq!(
        report["documentation_coverage"]["missing_symbols"]
            .as_array()
            .expect("missing symbols")
            .len(),
        3
    );
    assert!(report["documentation_coverage"]["by_kind"]["lambda"].is_null());
}

#[test]
fn documentation_threshold_is_exact_review_only_and_cli_overrides_policy() {
    let repository = tempdir().expect("temporary repository");
    write_documentation_fixture(repository.path());
    let policy = repository.path().join("review-policy.json");
    fs::write(
        &policy,
        r#"{
            "policy_version": "0.2.0",
            "documentation_coverage": { "human_review_below": 80 }
        }"#,
    )
    .expect("policy fixture");
    let root = repository.path().to_str().unwrap();
    let policy_path = policy.to_str().unwrap();

    let review = run(&[
        "comments",
        root,
        "--format",
        "json",
        "--policy",
        policy_path,
        "--gate",
    ]);
    let report: Value = serde_json::from_slice(&review.stdout).expect("review JSON");
    assert_eq!(review.status.code(), Some(2));
    assert_eq!(report["review"]["status"], "human-review-required");
    assert_eq!(report["review"]["findings"][0]["observed_value"], 5714);
    assert_eq!(report["review"]["findings"][0]["threshold"], 8000);
    assert_eq!(report["review"]["findings"][0]["unit"], "basis-points");
    assert_eq!(report["review"]["findings"][0]["risk"], "unknown");

    let analyze_gate = run(&[
        "analyze",
        root,
        "--documentation-review-below",
        "58",
        "--gate",
        "--format",
        "gate",
    ]);
    let gate_report: Value =
        serde_json::from_slice(&analyze_gate.stdout).expect("analyze gate JSON");
    assert_eq!(analyze_gate.status.code(), Some(2));
    assert_eq!(gate_report["report_schema_version"], "0.3.0");
    assert_eq!(
        gate_report["top_findings"][0]["rule_id"],
        "python-documentation-coverage-below-threshold"
    );
    assert_eq!(gate_report["top_findings"][0]["observed_value"], 5714);
    assert_eq!(gate_report["top_findings"][0]["unit"], "basis-points");

    let override_pass = run(&[
        "comments",
        root,
        "--format",
        "json",
        "--policy",
        policy_path,
        "--documentation-review-below",
        "57",
        "--gate",
    ]);
    let report: Value = serde_json::from_slice(&override_pass.stdout).expect("override JSON");
    assert!(override_pass.status.success());
    assert_eq!(report["review"]["human_review_below"], 57);
    assert_eq!(report["review"]["status"], "pass");

    let missing_threshold = run(&["comments", root, "--gate"]);
    assert_eq!(missing_threshold.status.code(), Some(1));
    assert!(
        String::from_utf8(missing_threshold.stderr)
            .expect("stderr UTF-8")
            .contains("requires --documentation-review-below or a policy threshold")
    );
}

#[test]
fn comments_honors_selection_and_ignored_inclusion() {
    let repository = tempdir().expect("temporary repository");
    fs::write(repository.path().join(".gitignore"), "generated/\n").expect("ignore fixture");
    fs::create_dir(repository.path().join("generated")).expect("generated directory");
    fs::write(
        repository.path().join("generated/documented.py"),
        "\"\"\"Generated module.\"\"\"\n",
    )
    .expect("ignored Python fixture");

    let output = run(&[
        "comments",
        repository.path().to_str().unwrap(),
        "--include-ignored",
        "generated/**",
        "--match",
        r"generated/documented\.py",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("comments JSON");
    assert!(output.status.success());
    assert_eq!(report["documentation_coverage"]["applicable_files"], 1);
    assert_eq!(report["documentation_coverage"]["counts"]["documented"], 1);
}

#[test]
fn documentation_skips_tests_by_default_and_can_include_them() {
    let repository = tempdir().expect("temporary repository");
    fs::create_dir(repository.path().join("src")).expect("source directory");
    fs::create_dir(repository.path().join("tests")).expect("test directory");
    fs::write(
        repository.path().join("src/service.py"),
        "\"\"\"Service module.\"\"\"\ndef run():\n    pass\n",
    )
    .expect("source fixture");
    fs::write(
        repository.path().join("tests/test_service.py"),
        "def test_run():\n    pass\n",
    )
    .expect("test fixture");
    let root = repository.path().to_str().unwrap();

    let default = run(&["analyze", root, "--format", "json"]);
    let report: Value = serde_json::from_slice(&default.stdout).expect("analysis JSON");
    assert!(default.status.success());
    assert_eq!(report["analyzers"][0]["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["documentation_coverage"]["applicable_files"], 1);
    assert_eq!(report["documentation_coverage"]["skipped_test_files"], 1);
    assert_eq!(report["documentation_coverage"]["counts"]["eligible"], 2);
    assert_eq!(report["documentation_coverage"]["counts"]["documented"], 1);

    let included = run(&["comments", root, "--include-tests", "--format", "json"]);
    let report: Value = serde_json::from_slice(&included.stdout).expect("comments JSON");
    assert!(included.status.success());
    assert_eq!(report["documentation_coverage"]["applicable_files"], 2);
    assert_eq!(report["documentation_coverage"]["skipped_test_files"], 0);
    assert_eq!(report["documentation_coverage"]["counts"]["eligible"], 4);
    assert_eq!(report["documentation_coverage"]["counts"]["documented"], 1);
}

#[test]
fn analyze_rejects_disabling_coverage_required_by_policy() {
    let repository = tempdir().expect("temporary repository");
    write_documentation_fixture(repository.path());
    let policy = repository.path().join("review-policy.json");
    fs::write(
        &policy,
        r#"{
            "policy_version": "0.2.0",
            "documentation_coverage": { "human_review_below": 80 }
        }"#,
    )
    .expect("policy fixture");

    let output = run(&[
        "analyze",
        repository.path().to_str().unwrap(),
        "--policy",
        policy.to_str().unwrap(),
        "--no-documentation-coverage",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr UTF-8")
            .contains("documentation coverage cannot be disabled")
    );
}

#[test]
fn partial_documentation_evidence_cannot_pass_a_threshold() {
    let repository = tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("broken.py"),
        "\"\"\"Module documentation.\"\"\"\ndef broken():\n    if:\n        return 1\n",
    )
    .expect("malformed Python fixture");

    let output = run(&[
        "comments",
        repository.path().to_str().unwrap(),
        "--documentation-review-below",
        "1",
        "--gate",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("comments JSON");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(report["documentation_coverage"]["status"], "partial");
    assert_eq!(
        report["review"]["findings"][0]["rule_id"],
        "python-documentation-coverage-unavailable"
    );

    let invalid = run(&[
        "comments",
        repository.path().to_str().unwrap(),
        "--documentation-review-below",
        "0",
    ]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8(invalid.stderr)
            .expect("stderr UTF-8")
            .contains("between 1 and 100")
    );
}

#[test]
fn cpp_documentation_coverage_is_explicitly_not_applicable() {
    let repository = tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("native.cpp"),
        "int native_entry() { return 1; }\n",
    )
    .expect("C++ fixture");

    let output = run(&[
        "comments",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("comments JSON");
    assert!(output.status.success());
    assert_eq!(report["documentation_coverage"]["status"], "not-applicable");
    assert_eq!(report["documentation_coverage"]["applicable_files"], 0);
    assert_eq!(
        report["documentation_coverage"]["unsupported_selected_files"],
        1
    );
}
