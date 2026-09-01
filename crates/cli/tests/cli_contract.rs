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
