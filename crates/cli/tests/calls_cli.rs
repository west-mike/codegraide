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
    assert_eq!(report["report_schema_version"], "0.2.0");
    assert_eq!(report["definitions"]["graph"], "call-graph-v2");
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
fn call_renderers_and_focus_use_the_dedicated_explorer() {
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
    assert!(html_stdout.contains("Codegraide Call Graph Explorer"));
    assert!(html_stdout.contains("Interactive caller and callee graph"));
    assert!(html_stdout.contains("Return to the project overview"));
    assert!(html_stdout.contains("Hide navigator"));
    assert!(html_stdout.contains("Show details"));
    assert!(html_stdout.contains(".center{grid-column:2}"));
    assert!(html_stdout.contains("function wheelZoomDelta(e)"));
    assert!(html_stdout.contains("Math.abs(e.deltaY)>=Math.abs(e.deltaX)"));
    assert!(html_stdout.contains("Clear search"));
    assert!(html_stdout.contains("Search navigator"));
    assert!(html_stdout.contains("id=\"depthSlider\" type=\"range\" min=\"1\" max=\"10\""));
    assert!(html_stdout.contains("'FUNCTION CALLS'"));
    assert!(html_stdout.contains(".graph-node{cursor:pointer"));
    assert!(html_stdout.contains("ringGap=255"));
    assert!(html_stdout.contains("e.clientX,e.clientY"));
    assert!(!html_stdout.contains("useful matches per column"));
    assert!(html_stdout.contains("noticeTimer=setTimeout"));
    assert!(html_stdout.contains(".truncated{position:absolute;top:38px"));
    assert!(html_stdout.contains("Show parser-level and synthetic details"));
    assert!(html_stdout.contains("Show tests and bundled support code"));
    assert!(html_stdout.contains("shop.helpers::Client.send"));
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
    assert!(source_html_stdout.contains("Function code"));
    assert!(source_html_stdout.contains("max_expansion_depth\":3"));
    assert!(source_html_stdout.contains("Who calls this, and what it calls"));
    assert!(source_html_stdout.contains("tok-keyword"));
    assert!(source_html_stdout.contains("Showing up to"));
    assert!(source_html_stdout.contains("Exact match"));

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
fn cpp_only_projects_select_cpp_and_resolve_written_calls() {
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
    assert_eq!(report["language"], "cpp");
    assert!(report["nodes"].as_array().unwrap().len() >= 2);
    assert_eq!(report["coverage"]["total_calls"], 1);
    assert_eq!(report["coverage"]["exact_calls"], 1);
}

#[test]
fn cpp_module_imports_and_exports_reach_reports_and_the_explorer() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("facade.cppm"),
        r#"export module demo;
import std;
export import :details;
export namespace demo { int run() { return 0; } }
"#,
    )
    .expect("module fixture");

    let json = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&json.stdout).expect("call JSON");
    assert!(json.status.success());
    let module = report["cpp_modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["name"] == "demo")
        .expect("named module summary");
    assert_eq!(module["path"], "facade.cppm");
    assert_eq!(module["kind"], "interface");
    assert!(module["imports"].as_array().unwrap().iter().any(|import| {
        import["target"] == "std" && import["kind"] == "named" && import["exported"] == false
    }));
    assert!(module["imports"].as_array().unwrap().iter().any(|import| {
        import["target"] == ":details"
            && import["kind"] == "partition"
            && import["exported"] == true
    }));

    let html = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--format",
        "html",
    ]);
    let html = String::from_utf8(html.stdout).expect("UTF-8 HTML");
    assert!(html.contains("module_imports"));
    assert!(html.contains("import std (named)"));
    assert!(html.contains("export import :details (partition)"));
}

#[test]
fn cpp_links_headers_definitions_and_receiver_typed_calls() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("widget.hpp"),
        r#"namespace demo {
class Widget {
public:
  Widget(int value = 0);
  void run(int value) const;
};
int helper(int value);
}
"#,
    )
    .expect("header fixture");
    fs::write(
        repository.path().join("widget.cpp"),
        r#"#include "widget.hpp"
namespace demo {
Widget::Widget(int value) { (void)value; }
void Widget::run(int value) const { (void)value; }
int helper(int value) { return value; }
}
"#,
    )
    .expect("definition fixture");
    fs::write(
        repository.path().join("main.cpp"),
        r#"#include "widget.hpp"
using demo::helper;
using WidgetAlias = demo::Widget;
int main() {
  WidgetAlias widget(1);
  widget.run(2);
  return helper(3);
}
"#,
    )
    .expect("caller fixture");
    let output = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--format",
        "json",
    ]);
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    let report: Value = serde_json::from_slice(&output.stdout).expect("call JSON");
    assert!(output.status.success(), "{stderr}");
    assert_eq!(report["coverage"]["total_calls"], 3);
    assert_eq!(report["coverage"]["exact_calls"], 3);
    let run = report["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["selector"] == "demo::Widget::run")
        .expect("run method");
    assert_eq!(run["link_status"], "linked");
    assert_eq!(run["declarations"].as_array().unwrap().len(), 1);
    assert!(run["definition"].is_object());
}

#[test]
fn cpp_architecture_groups_validate_and_tag_symbols() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("main.cpp"),
        "namespace demo { int helper() { return 1; } }\n",
    )
    .expect("source fixture");
    let architecture = repository.path().join("architecture.json");
    fs::write(
        &architecture,
        r#"{
  "architecture_schema_version": "0.1.0",
  "groups": [
    {"id":"first","name":"First","path_globs":["*.cpp"],"symbol_regexes":[],"module_regexes":[]},
    {"id":"api","name":"API","path_globs":[],"symbol_regexes":["^demo::"],"module_regexes":[]}
  ]
}"#,
    )
    .expect("architecture fixture");
    let output = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--architecture",
        architecture.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).expect("call JSON");
    assert!(output.status.success());
    let helper = report["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["selector"] == "demo::helper")
        .expect("helper symbol");
    assert_eq!(helper["primary_architecture_group"], "first");
    assert_eq!(
        helper["architecture_groups"],
        serde_json::json!(["first", "api"])
    );

    fs::write(
        &architecture,
        r#"{"architecture_schema_version":"0.1.0","groups":[
          {"id":"same","name":"One","path_globs":[],"symbol_regexes":[],"module_regexes":[]},
          {"id":"same","name":"Two","path_globs":[],"symbol_regexes":[],"module_regexes":[]}
        ]}"#,
    )
    .expect("invalid architecture fixture");
    let invalid = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--architecture",
        architecture.to_str().unwrap(),
    ]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("duplicated"));
}

#[test]
fn mixed_projects_require_a_call_language_and_expansion_depth_requires_source_html() {
    let repository = tempfile::tempdir().expect("temporary repository");
    fs::write(
        repository.path().join("main.cpp"),
        "int main() { return 0; }\n",
    )
    .expect("C++ fixture");
    fs::write(
        repository.path().join("tool.py"),
        "def run():\n    return 0\n",
    )
    .expect("Python fixture");
    let mixed = run(&["calls", repository.path().to_str().unwrap()]);
    assert!(!mixed.status.success());
    assert!(String::from_utf8_lossy(&mixed.stderr).contains("select --language"));

    let invalid_depth = run(&[
        "calls",
        repository.path().to_str().unwrap(),
        "--language",
        "cpp",
        "--max-expansion-depth",
        "3",
    ]);
    assert!(!invalid_depth.status.success());
    assert!(String::from_utf8_lossy(&invalid_depth.stderr).contains("requires --include-source"));
}
