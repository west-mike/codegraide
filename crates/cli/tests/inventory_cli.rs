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
fn prints_human_readable_summary_and_selected_category_paths() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    fs::create_dir(root.join("src")).expect("source directory should be created");
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n")
        .expect("source fixture should be written");
    fs::write(root.join("README.md"), "# Example\n")
        .expect("documentation fixture should be written");

    let output = run(&[
        "inventory",
        root.to_str().expect("temporary path should be UTF-8"),
        "--list-files",
        "source",
    ]);
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(stdout.contains("Inventoried files: 2"));
    assert!(stdout.contains("Source files: 1"));
    assert!(stdout.contains("Source lines: 1"));
    assert!(stdout.contains("documentation    1"));
    assert!(stdout.contains("Selected files:"));
    assert!(stdout.contains("src/main.rs"));
    assert!(stdout.contains("contents of ignored directories were not enumerated"));
    assert!(stderr.is_empty());
}

#[test]
fn prints_exact_ignored_audit_paths() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    fs::write(root.join(".gitignore"), "generated/\n")
        .expect("Git ignore fixture should be written");
    fs::create_dir(root.join("generated")).expect("ignored directory should be created");
    fs::write(root.join("generated").join("output.py"), "ignored\n")
        .expect("ignored fixture should be written");

    let output = run(&[
        "inventory",
        root.to_str().expect("temporary path should be UTF-8"),
        "--audit-ignored",
    ]);
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");

    assert!(output.status.success());
    assert!(stdout.contains("Audit: exact except for built-in safety directories"));
    assert!(stdout.contains("generated/output.py"));
    assert!(stdout.contains("generated"));
}

#[test]
fn warnings_can_be_suppressed_but_errors_cannot() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = root.join("rules.json");
    fs::write(
        &config,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "categories": {
                    "configuration": {
                        "include_filenames": ["special.md"]
                    }
                }
            }
        }"#,
    )
    .expect("configuration fixture should be written");
    fs::write(root.join("special.md"), "special\n").expect("overlap fixture should be written");

    let path = root.to_str().expect("temporary path should be UTF-8");
    let config_path = config.to_str().expect("config path should be UTF-8");
    let warning_output = run(&["inventory", path, "--config", config_path]);
    let warning_stderr = String::from_utf8(warning_output.stderr).expect("stderr should be UTF-8");
    assert!(warning_output.status.success());
    assert!(warning_stderr.contains("warning[category-more-specific-rule-wins]"));

    let quiet_output = run(&["inventory", path, "--config", config_path, "--no-warnings"]);
    assert!(quiet_output.status.success());
    assert!(quiet_output.stderr.is_empty());

    fs::write(&config, "{not JSON").expect("invalid configuration should be written");
    let error_output = run(&["inventory", path, "--config", config_path, "--no-warnings"]);
    let error_stderr = String::from_utf8(error_output.stderr).expect("stderr should be UTF-8");
    assert!(!error_output.status.success());
    assert!(error_stderr.contains("error:"));
    assert!(error_stderr.contains("JSON could not be parsed"));
}

#[test]
fn rejects_unknown_list_category_during_argument_parsing() {
    let output = run(&["inventory", ".", "--list-files", "infrastructure"]);
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success());
    assert!(stderr.contains("invalid value 'infrastructure'"));
}

#[test]
fn prints_deterministic_versioned_json_with_line_counts() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    fs::write(
        root.join("main.py"),
        "# comment\n\nvalue = 1 # trailing comment\n",
    )
    .expect("Python fixture should be written");
    fs::write(root.join("README.md"), "# Example\n")
        .expect("documentation fixture should be written");

    let path = root.to_str().expect("temporary path should be UTF-8");
    let first = run(&["inventory", path, "--format", "json"]);
    let second = run(&["inventory", path, "--format", "json"]);
    let first_stdout = String::from_utf8(first.stdout).expect("stdout should be UTF-8");
    let second_stdout = String::from_utf8(second.stdout).expect("stdout should be UTF-8");
    let report: Value = serde_json::from_str(&first_stdout).expect("stdout should be JSON");

    assert!(first.status.success());
    assert!(second.status.success());
    assert!(first.stderr.is_empty());
    assert_eq!(first_stdout, second_stdout);
    assert_eq!(report["report_schema_version"], "0.2.0");
    assert_eq!(report["analysis"]["kind"], "inventory");
    assert_eq!(
        report["analysis"]["definition_version"],
        "inventory-report-v1"
    );
    assert_eq!(report["inventory"]["inventoried_files"], 2);
    assert_eq!(report["inventory"]["categories"]["source"]["count"], 1);
    assert_eq!(
        report["inventory"]["categories"]["source"]["files"][0],
        "main.py"
    );
    assert_eq!(report["inventory"]["line_counts"]["total"]["files"], 1);
    assert_eq!(
        report["inventory"]["line_counts"]["total"]["total_lines"],
        3
    );
    assert_eq!(
        report["inventory"]["line_counts"]["total"]["source_lines"],
        1
    );
    assert_eq!(
        report["inventory"]["line_counts"]["total"]["comment_lines"],
        1
    );
    assert_eq!(
        report["inventory"]["line_counts"]["total"]["blank_lines"],
        1
    );
    assert_eq!(report["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn puts_warnings_in_json_and_can_omit_them() {
    let repository = tempdir().expect("temporary repository should be created");
    let root = repository.path();
    let config = root.join("rules.json");
    fs::write(
        &config,
        r#"{
            "config_version": "0.1.0",
            "inventory": {
                "categories": {
                    "configuration": {
                        "include_filenames": ["special.md"]
                    }
                }
            }
        }"#,
    )
    .expect("configuration fixture should be written");
    fs::write(root.join("special.md"), "special\n").expect("fixture should be written");

    let path = root.to_str().expect("temporary path should be UTF-8");
    let config_path = config.to_str().expect("configuration path should be UTF-8");
    let output = run(&[
        "inventory",
        path,
        "--config",
        config_path,
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(report["diagnostics"][0]["severity"], "warning");
    assert_eq!(
        report["diagnostics"][0]["code"],
        "category-more-specific-rule-wins"
    );

    let quiet = run(&[
        "inventory",
        path,
        "--config",
        config_path,
        "--format",
        "json",
        "--no-warnings",
    ]);
    let quiet_report: Value = serde_json::from_slice(&quiet.stdout).expect("stdout should be JSON");
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());
    assert_eq!(quiet_report["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn rejects_terminal_file_listing_in_json_mode() {
    let output = run(&[
        "inventory",
        ".",
        "--format",
        "json",
        "--list-files",
        "source",
    ]);
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success());
    assert!(stderr.contains("--list-files cannot be combined with --format json"));
}

#[test]
fn reports_missing_and_non_directory_paths() {
    let repository = tempdir().expect("temporary repository should be created");
    let file = repository.path().join("file.txt");
    fs::write(&file, "not a directory\n").expect("fixture should be written");

    let missing = run(&[
        "inventory",
        repository.path().join("missing").to_str().unwrap(),
    ]);
    let missing_stderr = String::from_utf8(missing.stderr).expect("stderr should be UTF-8");
    assert!(!missing.status.success());
    assert!(missing_stderr.contains("cannot access"));

    let file_output = run(&["inventory", file.to_str().unwrap()]);
    let file_stderr = String::from_utf8(file_output.stderr).expect("stderr should be UTF-8");
    assert!(!file_output.status.success());
    assert!(file_stderr.contains("is not a directory"));
}
