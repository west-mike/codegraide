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

fn language_graph<'a>(report: &'a Value, language: &str) -> &'a Value {
    &report["languages"]
        .as_array()
        .expect("language reports")
        .iter()
        .find(|entry| entry["language"] == language)
        .unwrap_or_else(|| panic!("missing {language} dependency report"))["graph"]
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

fn mixed_dependency_project() -> TempDir {
    let repository = dependency_project();
    fs::create_dir_all(repository.path().join("native/include")).expect("C++ include directory");
    fs::write(
        repository.path().join("native/main.cpp"),
        "#include \"widget.hpp\"\nint main() { return widget(); }\n",
    )
    .expect("C++ source");
    fs::write(
        repository.path().join("native/widget.hpp"),
        "inline int widget() { return 1; }\n",
    )
    .expect("C++ header");
    fs::write(repository.path().join("tool.rs"), "fn main() {}\n").expect("Rust source");
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
    let destination = repository.path().join("dependency-report");
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "html",
        "--output",
        destination.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    let overview = fs::read_to_string(destination.join("index.html")).expect("overview");
    let python = fs::read_to_string(destination.join("python.html")).expect("Python graph");

    assert!(output.status.success(), "{stderr}");
    assert!(output.stdout.is_empty());
    assert!(overview.starts_with("<!doctype html>"));
    assert!(overview.contains("href=\"python.html\""));
    assert!(python.contains("Dependency Explorer"));
    assert!(python.contains("\"name\":\"shop.service\""));
    assert!(python.contains("\"number\":1"));
    assert!(python.contains("\"witness_nodes\""));
    assert!(python.contains("\"recommended_cuts\""));
    assert!(!python.contains("aria-label=\"Dependency languages\""));
    assert!(!python.contains("https://"));
    assert!(!python.contains("fan_in"));
    assert!(!python.contains("fan_out"));
}

#[test]
fn html_output_can_be_written_to_an_explicit_directory() {
    let repository = dependency_project();
    let destination = repository.path().join("dependency-view");
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
    assert!(stderr.contains("wrote dependency report bundle"));
    assert!(
        fs::read_to_string(destination.join("python.html"))
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
    assert_eq!(report["report_schema_version"], "0.5.0");
    let report = language_graph(&report, "python");
    assert_eq!(report["definitions"]["graph"], "dependency-graph-v2");
    assert_eq!(report["view"]["local_only"], true);
    assert_eq!(report["coverage"]["total_references"], 6);
    assert_eq!(
        report["definitions"]["cycle_explanations"],
        "dependency-cycle-explanation-v2"
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
    let report = language_graph(&report, "python");
    assert_eq!(report["query"]["kind"], "shortest-path");
    assert_eq!(report["query"]["found"], true);
    assert_eq!(report["view"]["node_count"], 3);
    assert_eq!(report["view"]["relation_count"], 2);

    let html_directory = repository.path().join("path-report");
    let html = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--path-from",
        "shop.api",
        "--path-to",
        "shop.models",
        "--format",
        "html",
        "--output",
        html_directory.to_str().unwrap(),
    ]);
    assert!(html.status.success());
    let html_page = fs::read_to_string(html_directory.join("python.html")).expect("HTML graph");
    assert!(html_page.contains("Shortest path: shop.api"));
    assert!(html_page.contains("\"kind\":\"shortest-path\""));
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
    let report = language_graph(&report, "python");
    assert_eq!(report["query"]["direction"], "dependencies");
    assert_eq!(
        report["query"]["ordered_units"].as_array().unwrap().len(),
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
    let report = language_graph(&report, "python");
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
    let baseline_report = language_graph(&baseline_report, "python");
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
        let report = language_graph(&report, "python");
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
    let excluded_report = language_graph(&excluded_report, "python");
    assert_eq!(excluded_report["input_exclusions"]["conditional"], true);
    assert_eq!(excluded_report["coverage"]["total_references"], 6);
    assert_eq!(
        excluded_report["query"]["ordered_units"]
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
    let report = language_graph(&report, "python");
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
fn dependency_graph_builds_cpp_file_graph_when_cpp_includes_are_present() {
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
    let report = language_graph(&report, "cpp");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["languages"][0]["resolver"]["definition_version"],
        "cpp-header-resolution-v1"
    );
    assert_eq!(report["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(report["relations"].as_array().unwrap().len(), 1);
    assert_eq!(report["coverage"]["total_references"], 1);
    assert!(
        report["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["kind"] == "local-file")
    );
}

#[test]
fn mixed_repository_reports_independent_languages_and_qualified_focus() {
    let repository = mixed_dependency_project();
    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");
    assert!(output.status.success());
    let languages = report["languages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["language"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(languages, ["cpp", "python"]);
    assert!(
        report["unavailable_languages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["language"] == "rust")
    );

    let ambiguous = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--focus",
        "shop.api",
    ]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("language:identity"));

    let qualified = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--focus",
        "python:shop.api",
        "--format",
        "json",
    ]);
    assert!(qualified.status.success());
}

#[test]
fn cpp_compilation_database_preserves_context_dependent_targets_without_execution() {
    let repository = tempfile::tempdir().expect("temporary repository");
    for directory in ["src", "debug", "release"] {
        fs::create_dir_all(repository.path().join(directory)).expect("fixture directory");
    }
    fs::write(
        repository.path().join("src/main.cpp"),
        "#include \"config.hpp\"\n#include \"shared.hpp\"\n",
    )
    .expect("source fixture");
    fs::write(repository.path().join("src/shared.hpp"), "#pragma once\n").expect("shared header");
    fs::write(
        repository.path().join("debug/config.hpp"),
        "#define MODE 1\n",
    )
    .expect("debug header");
    fs::write(
        repository.path().join("release/config.hpp"),
        "#define MODE 2\n",
    )
    .expect("release header");
    let sentinel = repository.path().join("compiler-was-executed");
    let fake_compiler = repository.path().join("fake-clang++");
    fs::write(
        &fake_compiler,
        format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    )
    .expect("fake compiler");
    let database = serde_json::json!([
        {
            "directory": repository.path(),
            "arguments": [fake_compiler.clone(), "-I", repository.path().join("debug"), "-c", repository.path().join("src/main.cpp")],
            "file": repository.path().join("src/main.cpp")
        },
        {
            "directory": repository.path(),
            "command": format!("{} -I{} -c {}", fake_compiler.display(), repository.path().join("release").display(), repository.path().join("src/main.cpp").display()),
            "file": repository.path().join("src/main.cpp")
        }
    ]);
    let database_path = repository.path().join("compile_commands.json");
    fs::write(
        &database_path,
        serde_json::to_string_pretty(&database).unwrap(),
    )
    .expect("compilation database");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--compile-commands",
        database_path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !sentinel.exists(),
        "recorded compiler command must not execute"
    );
    let serialized = String::from_utf8(output.stdout).expect("UTF-8 JSON");
    assert!(!serialized.contains(repository.path().to_str().unwrap()));
    let report: Value = serde_json::from_str(&serialized).expect("dependency JSON");
    let graph = language_graph(&report, "cpp");
    assert_eq!(graph["coverage"]["exact_references"], 1);
    assert_eq!(graph["coverage"]["context_dependent_references"], 1);
    assert!(
        graph["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|relation| relation["kind"] == "context-dependent")
    );
}

#[test]
fn html_bundle_separates_language_payloads_and_regenerates_known_files() {
    let repository = mixed_dependency_project();
    let destination = repository.path().join("report");
    let first = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "html",
        "--output",
        destination.to_str().unwrap(),
    ]);
    assert!(first.status.success());
    let index = fs::read_to_string(destination.join("index.html")).expect("overview");
    let python = fs::read_to_string(destination.join("python.html")).expect("Python page");
    let cpp = fs::read_to_string(destination.join("cpp.html")).expect("C++ page");
    assert!(index.contains("href=\"python.html\"") && index.contains("href=\"cpp.html\""));
    assert!(python.contains("\"name\":\"shop.api\""));
    assert!(!python.contains("\"name\":\"native/main.cpp\""));
    assert!(cpp.contains("\"name\":\"native/main.cpp\""));
    assert!(!cpp.contains("\"name\":\"shop.api\""));

    let second = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--format",
        "html",
        "--output",
        destination.to_str().unwrap(),
    ]);
    assert!(second.status.success());
    assert!(!destination.join("python.html").exists());
    assert!(destination.join("cpp.html").is_file());
}

#[test]
fn static_multi_language_outputs_use_disconnected_language_clusters() {
    let repository = mixed_dependency_project();
    for format in ["mermaid", "dot"] {
        let output = run(&[
            "dependencies",
            repository.path().to_str().unwrap(),
            "--format",
            format,
        ]);
        let stdout = String::from_utf8(output.stdout).expect("UTF-8 graph");
        assert!(output.status.success());
        assert!(stdout.contains("language_cpp"));
        assert!(stdout.contains("language_python"));
    }
}

#[test]
fn cpp_fallback_marks_unique_repository_suffixes_as_inferred() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::create_dir_all(repository.path().join("src")).expect("source directory");
    fs::write(
        repository.path().join("src/main.cpp"),
        "#include \"local.hpp\"\n#include \"generated.inc\"\n#include \"elsewhere.hpp\"\n",
    )
    .expect("source fixture");
    fs::write(repository.path().join("src/local.hpp"), "#pragma once\n").expect("local header");
    fs::write(
        repository.path().join("src/generated.inc"),
        "GENERATED_VALUE\n",
    )
    .expect("unrecognized local include");
    fs::write(repository.path().join("elsewhere.hpp"), "#pragma once\n").expect("nonlocal header");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--format",
        "json",
    ]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");
    let graph = language_graph(&report, "cpp");
    assert_eq!(graph["coverage"]["exact_references"], 2);
    assert_eq!(graph["coverage"]["inferred_references"], 1);
    assert_eq!(graph["coverage"]["unresolved_references"], 0);
    assert!(
        graph["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|relation| {
                relation["kind"] == "inferred"
                    && relation["inference_basis"] == "unique-repository-suffix"
            })
    );
    assert!(graph["nodes"].as_array().unwrap().iter().any(|node| {
        node["name"] == "src/generated.inc" && node["outgoing_dependencies_analyzed"] == false
    }));
}

#[test]
fn cpp_fallback_does_not_guess_when_repository_suffixes_are_ambiguous() {
    let repository = tempfile::tempdir().expect("temporary repository");
    for directory in ["src", "first", "second"] {
        fs::create_dir_all(repository.path().join(directory)).expect("fixture directory");
    }
    fs::write(
        repository.path().join("src/main.cpp"),
        "#include <shared.hpp>\n#include <vector>\n",
    )
    .expect("source fixture");
    for directory in ["first", "second"] {
        fs::write(
            repository.path().join(directory).join("shared.hpp"),
            "#pragma once\n",
        )
        .expect("duplicate header");
    }

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--format",
        "json",
    ]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");
    let graph = language_graph(&report, "cpp");
    assert_eq!(graph["coverage"]["exact_references"], 1);
    assert_eq!(graph["coverage"]["inferred_references"], 0);
    assert_eq!(graph["coverage"]["unresolved_references"], 1);
    assert!(
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| { node["kind"] == "system-header" && node["name"] == "vector" })
    );
}

#[test]
fn cpp_header_only_layout_produces_useful_inferred_local_relations() {
    let repository = tempfile::tempdir().expect("temporary repository");
    for directory in ["include/argparse", "samples", "test"] {
        fs::create_dir_all(repository.path().join(directory)).expect("fixture directory");
    }
    fs::write(
        repository.path().join("include/argparse/argparse.hpp"),
        "#include <vector>\n",
    )
    .expect("library header");
    fs::write(repository.path().join("test/doctest.hpp"), "#pragma once\n").expect("test header");
    fs::write(
        repository.path().join("samples/example.cpp"),
        "#include <argparse/argparse.hpp>\n",
    )
    .expect("sample source");
    fs::write(
        repository.path().join("test/example.cpp"),
        "#include <argparse/argparse.hpp>\n#include <doctest.hpp>\n",
    )
    .expect("test source");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--local-only",
        "--format",
        "json",
    ]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("dependency JSON");
    let graph = language_graph(&report, "cpp");
    assert_eq!(graph["coverage"]["exact_references"], 1);
    assert_eq!(graph["coverage"]["inferred_references"], 3);
    assert_eq!(graph["view"]["node_count"], 4);
    assert_eq!(graph["view"]["relation_count"], 3);
    assert!(
        graph["relations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|relation| { relation["kind"] == "inferred" })
    );
}

#[test]
fn malformed_selected_compilation_database_is_an_operational_error() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(repository.path().join("main.cpp"), "#include <vector>\n").expect("source fixture");
    let database = repository.path().join("compile_commands.json");
    fs::write(&database, "{ definitely not JSON }").expect("malformed database");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--compile-commands",
        database.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(!output.status.success());
    assert!(stderr.contains("cannot parse compilation database"));
}

#[test]
fn html_bundle_refuses_an_unowned_nonempty_directory() {
    let repository = dependency_project();
    let destination = repository.path().join("existing-output");
    fs::create_dir(&destination).expect("output directory");
    fs::write(destination.join("keep.txt"), "belongs to the user\n").expect("user file");

    let output = run(&[
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "html",
        "--output",
        destination.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(!output.status.success());
    assert!(stderr.contains("nonempty and has no Codegraide dependency manifest"));
    assert_eq!(
        fs::read_to_string(destination.join("keep.txt")).expect("preserved user file"),
        "belongs to the user\n"
    );
}

#[test]
fn repeated_multi_language_json_is_byte_identical() {
    let repository = mixed_dependency_project();
    let arguments = [
        "dependencies",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ];
    let first = run(&arguments);
    let second = run(&arguments);
    assert!(first.status.success() && second.status.success());
    assert_eq!(first.stdout, second.stdout);
}
