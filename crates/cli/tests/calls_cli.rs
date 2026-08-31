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

fn call_project() -> TempDir {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::create_dir_all(repository.path().join("src/shop")).expect("src package");
    fs::write(
        repository.path().join("pyproject.toml"),
        "[tool.setuptools.packages.find]\nwhere = [\"src\"]\n",
    )
    .expect("pyproject fixture");
    fs::write(repository.path().join("src/shop/__init__.py"), "").expect("package");
    fs::write(
        repository.path().join("src/shop/helpers.py"),
        r#"def helper():
    return 1

def recurse(value):
    if value:
        return recurse(value - 1)

class Client:
    def send(self):
        return self.retry()

    def retry(self):
        return self.send()

def outer():
    def inner():
        return 1
    return inner()
"#,
    )
    .expect("helper calls");
    fs::write(
        repository.path().join("src/shop/service.py"),
        r#"import requests as req
from .helpers import helper as h, Client
from . import helpers as hm

def run():
    h()
    hm.helper()
    Client()
    req.get("https://example.invalid")
    unknown.value()
"#,
    )
    .expect("service calls");
    repository
}

fn duplicate_call_project() -> TempDir {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("module.py"),
        "def duplicate():\n    return 1\ndef duplicate():\n    return 2\ndef caller():\n    return duplicate()\n",
    )
    .expect("duplicate definitions");
    repository
}

#[test]
fn call_json_resolves_conservative_local_patterns_and_boundaries() {
    let repository = call_project();
    let output = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    let report: Value = serde_json::from_slice(&output.stdout).expect("call JSON");

    assert!(output.status.success(), "{stderr}");
    assert_eq!(report["report_schema_version"], "0.1.0");
    assert_eq!(report["definitions"]["graph"], "call-graph-v1");
    assert_eq!(report["coverage"]["total_calls"], 9);
    assert_eq!(report["coverage"]["exact_calls"], 7);
    assert_eq!(report["coverage"]["external_calls"], 1);
    assert_eq!(report["coverage"]["unresolved_calls"], 1);
    let selectors = report["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["selector"].as_str())
        .collect::<Vec<_>>();
    assert!(selectors.contains(&"shop.helpers::Client.send"));
    assert!(selectors.contains(&"shop.helpers::outer.inner"));
    assert_eq!(
        report["strongly_connected_components"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|component| component["cyclic"] == true)
            .count(),
        2
    );
    assert!(
        report["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|relation| {
                relation["evidence"][0]["callee"] == "h"
                    && relation["evidence"][0]["source_path"] == "src/shop/service.py"
            })
    );
}

#[test]
fn call_renderers_and_focus_use_the_shared_explorer() {
    let repository = call_project();
    let mermaid = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--focus",
        "shop.helpers::Client.send",
        "--depth",
        "1",
        "--direction",
        "both",
        "--format",
        "mermaid",
    ]);
    let mermaid_stdout = String::from_utf8(mermaid.stdout).expect("UTF-8 Mermaid");
    assert!(mermaid.status.success());
    assert!(mermaid_stdout.starts_with("flowchart LR\n"));
    assert!(mermaid_stdout.contains("Client.send"));
    assert!(mermaid_stdout.contains("Client.retry"));

    let html = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--local-only",
        "--format",
        "html",
    ]);
    let html_stdout = String::from_utf8(html.stdout).expect("UTF-8 HTML");
    assert!(html.status.success());
    assert!(html_stdout.starts_with("<!doctype html>"));
    assert!(html_stdout.contains("\"graph_kind\":\"calls\""));
    assert!(html_stdout.contains("shop.helpers::Client.send"));
    assert!(html_stdout.contains("\"qualified_name\":\"shop.helpers.Client\""));
    assert!(!html_stdout.contains("\"source\":{"));
    assert!(!html_stdout.contains("https://"));

    let source_html = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--local-only",
        "--format",
        "html",
        "--include-source",
    ]);
    let source_html_stdout = String::from_utf8(source_html.stdout).expect("UTF-8 source HTML");
    assert!(source_html.status.success());
    assert!(source_html_stdout.contains("\"source\":{"));
    assert!(source_html_stdout.contains("\"lines\":[\"def helper():\",\"    return 1\"]"));
    assert!(source_html_stdout.contains("sourceAutoExpandLines = 15"));
    assert!(source_html_stdout.contains("callerRelationshipsMarkup"));
    assert!(source_html_stdout.contains("data-relation-source"));

    let dot = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--cycles-only",
        "--format",
        "dot",
    ]);
    let dot_stdout = String::from_utf8(dot.stdout).expect("UTF-8 DOT");
    assert!(dot.status.success());
    assert!(dot_stdout.starts_with("digraph call_graph"));
}

#[test]
fn including_source_is_limited_to_html_output() {
    let repository = call_project();
    let output = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
        "--include-source",
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");

    assert!(!output.status.success());
    assert!(stderr.contains("--include-source requires --format html"));
}

#[test]
fn duplicate_targets_remain_ambiguous_and_selectors_require_ordinals() {
    let repository = duplicate_call_project();
    let report_output = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&report_output.stdout).expect("call JSON");
    assert!(report_output.status.success());
    assert_eq!(report["coverage"]["ambiguous_calls"], 1);

    let ambiguous_focus = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--focus",
        "module::duplicate",
    ]);
    let stderr = String::from_utf8(ambiguous_focus.stderr).expect("UTF-8 stderr");
    assert!(!ambiguous_focus.status.success());
    assert!(stderr.contains("#N"));

    let ordinal_focus = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--focus",
        "module::duplicate#1",
    ]);
    assert!(ordinal_focus.status.success());
}

#[test]
fn call_graph_remains_python_only_when_cpp_call_syntax_is_present() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("native.cpp"),
        "int helper() { return 1; }\nint native_entry() { return helper(); }\n",
    )
    .expect("C++ fixture");

    let output = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("call JSON");
    assert!(output.status.success());
    assert_eq!(report["nodes"].as_array().unwrap().len(), 0);
    assert_eq!(report["relations"].as_array().unwrap().len(), 0);
    assert_eq!(report["coverage"]["total_calls"], 0);
}
