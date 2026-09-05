use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codegraide"))
        .args(arguments)
        .output()
        .expect("codegraide binary should run")
}

fn assert_error(arguments: &[&str], expected_code: i32, expected_message: &str) {
    let output = run(arguments);
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert_eq!(
        output.status.code(),
        Some(expected_code),
        "unexpected exit code for {arguments:?}: {stderr}"
    );
    assert!(
        stderr.contains(expected_message),
        "stderr for {arguments:?} did not contain {expected_message:?}: {stderr}"
    );
}

#[test]
fn global_help_and_version_expose_the_supported_command_surface() {
    let help = run(&["--help"]);
    let stdout = String::from_utf8(help.stdout).expect("help should be UTF-8");
    assert!(help.status.success());
    for command in ["inventory", "analyze", "comments", "dependencies", "calls"] {
        assert!(stdout.contains(command), "help should list {command}");
    }

    let version = run(&["--version"]);
    let stdout = String::from_utf8(version.stdout).expect("version should be UTF-8");
    assert!(version.status.success());
    assert_eq!(
        stdout.trim(),
        concat!("codegraide ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn bounded_numeric_options_reject_values_outside_their_contracts() {
    for arguments in [
        ["analyze", "--top", "0"],
        ["comments", "--top", "0"],
        ["dependencies", "--top", "0"],
    ] {
        assert_error(&arguments, 2, "COUNT must be a positive integer");
    }

    for arguments in [
        ["analyze", "--documentation-review-below", "0"],
        ["comments", "--documentation-review-below", "101"],
    ] {
        assert_error(
            &arguments,
            2,
            "PERCENT must be an integer between 1 and 100",
        );
    }
}

#[test]
fn analyze_rejects_incompatible_output_and_documentation_options() {
    for (arguments, expected_code, expected_message) in [
        (
            vec!["analyze", "--profile", "review"],
            1,
            "--profile review requires --format json",
        ),
        (
            vec!["analyze", "--format", "json", "--details"],
            1,
            "--details is only available with terminal output",
        ),
        (
            vec!["analyze", "--no-documentation-coverage", "--include-tests"],
            2,
            "cannot be used with '--include-tests'",
        ),
        (
            vec![
                "analyze",
                "--no-documentation-coverage",
                "--documentation-review-below",
                "80",
            ],
            2,
            "cannot be used with '--documentation-review-below",
        ),
        (
            vec![
                "analyze",
                "--no-complexity-block",
                "--complexity-block-at",
                "20",
            ],
            2,
            "cannot be used with '--complexity-block-at",
        ),
    ] {
        assert_error(&arguments, expected_code, expected_message);
    }
}

#[test]
fn dependency_query_options_enforce_complete_and_unambiguous_requests() {
    for (arguments, expected_code, expected_message) in [
        (
            vec!["dependencies", "--direction", "both"],
            1,
            "--direction requires --focus or --closure",
        ),
        (vec!["dependencies", "--depth", "2"], 2, "--focus <UNIT>"),
        (
            vec!["dependencies", "--path-from", "one"],
            2,
            "--path-to <UNIT>",
        ),
        (
            vec!["dependencies", "--closure", "one", "--focus", "two"],
            2,
            "cannot be used with '--focus",
        ),
    ] {
        assert_error(&arguments, expected_code, expected_message);
    }
}

#[test]
fn graph_commands_enforce_environment_and_html_option_contracts() {
    for command in ["dependencies", "calls"] {
        assert_error(
            &[command, "--python", "python", "--venv", "venv"],
            2,
            "cannot be used with '--venv",
        );
        assert_error(
            &[command, "--output", "graph.html"],
            1,
            "--output requires --format html",
        );
        assert_error(&[command, "--open"], 1, "--open requires --format html");
    }

    assert_error(
        &["calls", "--include-source"],
        1,
        "--include-source requires --format html",
    );
    assert_error(&["calls", "--direction", "both"], 2, "--focus <SYMBOL>");
    assert_error(&["calls", "--depth", "2"], 2, "--focus <SYMBOL>");
}

fn help_text(arguments: &[&str]) -> String {
    let output = run(arguments);
    assert!(output.status.success(), "help failed for {arguments:?}");
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).expect("help is UTF-8")
}

#[test]
fn detailed_help_is_available_without_inputs_and_short_help_stays_short() {
    assert!(help_text(&["--help"]).contains("configuration formats"));
    for command in [
        "inventory",
        "analyze",
        "comments",
        "dependencies",
        "calls",
        "review-context",
    ] {
        let long = help_text(&[command, "--help"]);
        assert!(long.contains("Examples:"), "{command} needs examples");
        assert_eq!(long, help_text(&["help", command]));
        let short = help_text(&[command, "-h"]);
        assert!(!short.contains("Examples:"));
        assert!(short.contains("Use --help"));
    }
    let dependencies = help_text(&["dependencies", "--help"]);
    assert!(dependencies.contains("--output <DIRECTORY>"));
    assert!(dependencies.contains("language:identity"));
    let calls = help_text(&["calls", "--help"]);
    assert!(calls.contains("inferred local calls remain visible"));
}

// Extract the JSON users can copy from the installed binary's help, then run it
// through real commands. This catches examples drifting from accepted formats.
fn help_json(command: &str) -> String {
    let text = help_text(&[command, "--help"]);
    let start = text.find("  {\n").expect("help includes a JSON example");
    let mut json = String::new();
    for line in text[start..].lines() {
        json.push_str(line.strip_prefix("  ").unwrap_or(line));
        json.push('\n');
        if line == "  }" {
            return json;
        }
    }
    panic!("unterminated JSON example in {command} help")
}

#[test]
fn configuration_examples_from_help_are_accepted_by_the_commands() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("sample.go"), "package sample\n").unwrap();
    std::fs::write(root.join("sample.py"), "\"\"\"Documented module.\"\"\"\n").unwrap();
    std::fs::write(
        root.join("sample.cpp"),
        "namespace app { int run() { return 1; } }\n",
    )
    .unwrap();
    for (command, flag, filename) in [
        ("inventory", "--config", "rules.json"),
        ("analyze", "--policy", "policy.json"),
        ("comments", "--policy", "comments-policy.json"),
        ("calls", "--architecture", "architecture.json"),
    ] {
        std::fs::write(root.join(filename), help_json(command)).unwrap();
        let mut invocation = Command::new(env!("CARGO_BIN_EXE_codegraide"));
        invocation
            .current_dir(root)
            .args([command, ".", flag, filename, "--format", "json"]);
        if command == "calls" {
            invocation.args(["--language", "cpp"]);
        }
        let output = invocation.output().unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report.is_object());
    }
}
