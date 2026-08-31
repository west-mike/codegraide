use std::fs;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codegraide"))
        .args(arguments)
        .output()
        .expect("codegraide binary should run")
}

fn dependency_project() -> TempDir {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::create_dir_all(repository.path().join("src/shop")).expect("src package");
    fs::write(
        repository.path().join("pyproject.toml"),
        "[tool.setuptools.packages.find]\nwhere = [\"src\"]\n",
    )
    .expect("pyproject fixture");
    fs::write(repository.path().join("src/shop/__init__.py"), "").expect("package fixture");
    fs::write(
        repository.path().join("src/shop/api.py"),
        "import json\nimport requests\nimport missing_package\nfrom . import service\n",
    )
    .expect("api fixture");
    fs::write(
        repository.path().join("src/shop/service.py"),
        "from . import models\n",
    )
    .expect("service fixture");
    fs::write(
        repository.path().join("src/shop/models.py"),
        "from . import service\n",
    )
    .expect("models fixture");
    repository
}

fn contextual_dependency_project() -> TempDir {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::create_dir_all(repository.path().join("src/pkg")).expect("src package");
    fs::write(
        repository.path().join("pyproject.toml"),
        "[tool.setuptools.packages.find]\nwhere = [\"src\"]\n",
    )
    .expect("pyproject fixture");
    fs::write(repository.path().join("src/pkg/__init__.py"), "").expect("package fixture");
    fs::write(
        repository.path().join("src/pkg/a.py"),
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from . import b\ntry:\n    from . import c\nexcept ImportError:\n    pass\ndef load():\n    from . import d\nif enabled:\n    from . import e\n",
    )
    .expect("context source");
    for name in ["b", "c", "d", "e"] {
        fs::write(
            repository.path().join(format!("src/pkg/{name}.py")),
            "from . import a\n",
        )
        .expect("reverse dependency");
    }
    repository
}

#[test]
fn mermaid_output_contains_local_cycle_and_unresolved_boundaries() {
    let repository = dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "mermaid",
    ]);
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");

    assert!(output.status.success(), "{stderr}");
    assert!(stdout.starts_with("flowchart LR\n"));
    assert!(stdout.contains("Cycle 1"));
    assert!(stdout.contains("shop.service"));
    assert!(stdout.contains("shop.models"));
    assert!(stdout.contains("requests"));
    assert!(stdout.contains("environment-unavailable"));
}

#[test]
fn html_output_is_a_self_contained_interactive_graph() {
    let repository = dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "html",
    ]);
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");

    assert!(output.status.success(), "{stderr}");
    assert!(stdout.starts_with("<!doctype html>"));
    assert!(stdout.contains("Codegraide Dependency Explorer"));
    assert!(stdout.contains("\"name\":\"shop.service\""));
    assert!(stdout.contains("\"number\":1"));
    assert!(stdout.contains("\"witness_nodes\""));
    assert!(stdout.contains("\"recommended_cuts\""));
    assert!(!stdout.contains("https://"));
    assert!(!stdout.contains("fan_in"));
    assert!(!stdout.contains("fan_out"));
}

#[test]
fn html_output_can_be_written_to_an_explicit_file() {
    let repository = dependency_project();
    let destination = repository.path().join("dependency-view.html");
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "html",
        "--output",
        destination.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");

    assert!(output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty());
    assert!(stderr.contains("wrote interactive dependency graph"));
    assert!(
        fs::read_to_string(destination)
            .expect("written graph")
            .starts_with("<!doctype html>")
    );
}

#[test]
fn browser_options_require_html_output() {
    let repository = dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--open",
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");

    assert!(!output.status.success());
    assert!(stderr.contains("--open requires --format html"));
}

#[test]
fn dependency_json_has_an_independent_schema_and_no_absolute_root() {
    let repository = dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
        "--local-only",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");

    assert!(output.status.success());
    assert_eq!(report["report_schema_version"], "0.4.0");
    assert_eq!(report["definitions"]["graph"], "dependency-graph-v1");
    assert_eq!(report["view"]["local_only"], true);
    assert_eq!(report["coverage"]["total_references"], 6);
    assert_eq!(
        report["definitions"]["cycle_explanations"],
        "dependency-cycle-explanation-v1"
    );
    assert_eq!(report["cycle_explanations"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["cycle_explanations"][0]["recommended_cuts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let serialized = String::from_utf8(output.stdout).expect("UTF-8 JSON");
    assert!(!serialized.contains(repository.path().to_str().unwrap()));
}

#[test]
fn path_query_is_ordered_and_limits_every_output_view() {
    let repository = dependency_project();
    let terminal = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--path-from",
        "shop.api",
        "--path-to",
        "shop.models",
    ]);
    let stdout = String::from_utf8(terminal.stdout).expect("UTF-8 stdout");
    assert!(terminal.status.success());
    assert!(stdout.contains("shop.api -> shop.service -> shop.models"));

    let json = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--path-from",
        "shop.api",
        "--path-to",
        "shop.models",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&json.stdout).expect("dependency JSON");
    assert!(json.status.success());
    assert_eq!(report["query"]["kind"], "shortest-path");
    assert_eq!(report["query"]["found"], true);
    assert_eq!(report["view"]["node_count"], 3);
    assert_eq!(report["view"]["relation_count"], 2);

    let html = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--path-from",
        "shop.api",
        "--path-to",
        "shop.models",
        "--format",
        "html",
    ]);
    let html_stdout = String::from_utf8(html.stdout).expect("UTF-8 HTML");
    assert!(html.status.success());
    assert!(html_stdout.contains("Shortest path: shop.api"));
    assert!(html_stdout.contains("\"kind\":\"shortest-path\""));
}

#[test]
fn closure_defaults_to_dependencies_and_rejects_both() {
    let repository = dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--closure",
        "shop.api",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");
    assert!(output.status.success());
    assert_eq!(report["query"]["direction"], "dependencies");
    assert_eq!(
        report["query"]["ordered_modules"].as_array().unwrap().len(),
        3
    );

    let invalid = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--closure",
        "shop.api",
        "--direction",
        "both",
    ]);
    let stderr = String::from_utf8(invalid.stderr).expect("UTF-8 stderr");
    assert!(!invalid.status.success());
    assert!(stderr.contains("not both"));
}

#[test]
fn unreachable_path_is_successful_and_unknown_query_modules_are_errors() {
    let repository = dependency_project();
    let unreachable = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--path-from",
        "shop.models",
        "--path-to",
        "shop.api",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&unreachable.stdout).expect("dependency JSON");
    assert!(unreachable.status.success());
    assert_eq!(report["query"]["found"], false);

    let unknown = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--closure",
        "shop.modls",
    ]);
    let stderr = String::from_utf8(unknown.stderr).expect("UTF-8 stderr");
    assert!(!unknown.status.success());
    assert!(stderr.contains("shop.models"));
}

#[test]
fn import_context_exclusions_recalculate_graphs_and_queries() {
    let repository = contextual_dependency_project();
    let baseline = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--local-only",
        "--format",
        "json",
    ]);
    let baseline_report: Value =
        serde_json::from_slice(&baseline.stdout).expect("baseline dependency JSON");
    assert!(baseline.status.success());
    assert_eq!(baseline_report["coverage"]["total_references"], 9);
    let a = baseline_report["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "pkg.a")
        .expect("pkg.a node");
    assert_eq!(a["fan_out"], 4);
    assert_eq!(
        baseline_report["relations"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|relation| relation["evidence"].as_array().unwrap())
            .filter(|evidence| evidence["usage"] == "type-checking-only")
            .count(),
        1
    );

    for (flag, field) in [
        ("--exclude-type-only", "type_only"),
        ("--exclude-optional", "optional"),
        ("--exclude-callable-local", "callable_local"),
    ] {
        let output = run(&[
            "dependencies",
            repository.path().to_str().unwrap(),
            flag,
            "--format",
            "json",
        ]);
        let report: Value =
            serde_json::from_slice(&output.stdout).expect("excluded dependency JSON");
        assert!(output.status.success(), "{flag} should succeed");
        assert_eq!(report["input_exclusions"][field], true);
        assert_eq!(report["coverage"]["total_references"], 8);
    }

    let excluded = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--closure",
        "pkg.a",
        "--exclude-conditional",
        "--format",
        "json",
    ]);
    let excluded_report: Value =
        serde_json::from_slice(&excluded.stdout).expect("excluded dependency JSON");
    assert!(excluded.status.success());
    assert_eq!(excluded_report["input_exclusions"]["conditional"], true);
    assert_eq!(excluded_report["coverage"]["total_references"], 6);
    assert_eq!(
        excluded_report["query"]["ordered_modules"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn focus_validation_suggests_a_known_module() {
    let repository = dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--focus",
        "shop.servic",
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");

    assert!(!output.status.success());
    assert!(stderr.contains("shop.service"));
}

#[test]
fn invalid_virtual_environment_is_an_operational_error() {
    let repository = dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--venv",
        repository.path().to_str().unwrap(),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");

    assert!(!output.status.success());
    assert!(stderr.contains("does not contain pyvenv.cfg"));
}

#[cfg(unix)]
#[test]
fn malformed_interpreter_probe_output_is_an_operational_error() {
    use std::os::unix::fs::PermissionsExt;

    let repository = dependency_project();
    let interpreter = repository.path().join("malformed-python");
    fs::write(&interpreter, "#!/bin/sh\nprintf 'not-json'\n").expect("fake interpreter");
    let mut permissions = fs::metadata(&interpreter).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&interpreter, permissions).expect("executable fixture");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--python",
        interpreter.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(!output.status.success());
    assert!(stderr.contains("malformed JSON"));
}

#[cfg(unix)]
#[test]
fn explicit_interpreter_enriches_stdlib_and_installed_packages() {
    use std::os::unix::fs::PermissionsExt;

    let repository = dependency_project();
    let interpreter = repository.path().join("fake-python");
    fs::write(
        &interpreter,
        "#!/bin/sh\nprintf '%s' '{\"schema_version\":\"codegraide-python-environment-v1\",\"implementation\":\"cpython\",\"version\":[3,12,1],\"is_virtual_environment\":false,\"stdlib_names\":[\"json\"],\"distributions\":[{\"normalized_name\":\"requests\",\"display_name\":\"Requests\",\"version\":\"2.32.0\",\"import_names\":[\"requests\"]}]}'\n",
    )
    .expect("fake interpreter");
    let mut permissions = fs::metadata(&interpreter).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&interpreter, permissions).expect("executable fixture");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--python",
        interpreter.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");
    assert!(output.status.success());
    assert_eq!(report["environment"]["python_version"], "3.12.1");
    let kinds = report["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|node| node["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"standard-library"));
    assert!(kinds.contains(&"installed-distribution"));
}

#[test]
fn dependency_graph_remains_python_only_when_cpp_includes_are_present() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("native.cpp"),
        "#include <vector>\nint native_entry() { return 1; }\n",
    )
    .expect("C++ fixture");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");
    assert!(output.status.success());
    assert_eq!(report["nodes"].as_array().unwrap().len(), 0);
    assert_eq!(report["relations"].as_array().unwrap().len(), 0);
    assert_eq!(report["coverage"]["total_references"], 0);
}
