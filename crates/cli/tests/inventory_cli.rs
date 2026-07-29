use std::fs;
use std::process::{Command, Output};

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
