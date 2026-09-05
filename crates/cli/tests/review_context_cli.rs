use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn put(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn commit(root: &Path) -> String {
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "feat(fixture): change code"]);
    git(root, &["rev-parse", "HEAD"])
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codegraide"))
        .arg("review-context")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
fn json(root: &Path, args: &[&str]) -> Value {
    let mut args = args.to_vec();
    args.extend(["--format", "json"]);
    let out = run(root, &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn fixture() -> (TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    put(
        root,
        "stock.hpp",
        "#pragma once\nstruct Stock { int available; };\nbool reserve(Stock& stock);\nvoid remove_stock(Stock& stock);\n",
    );
    put(
        root,
        "reserve.cpp",
        "#include \"stock.hpp\"\nbool reserve(Stock& stock) {\n  remove_stock(stock);\n  return stock.available > 0;\n}\n",
    );
    put(
        root,
        "caller.cpp",
        "#include \"stock.hpp\"\nbool checkout(Stock& stock) { return reserve(stock); }\n",
    );
    put(
        root,
        "stock.cpp",
        "#include \"stock.hpp\"\nvoid remove_stock(Stock& stock) { stock.available -= 1; }\n",
    );
    let base = commit(root);
    put(
        root,
        "stock.hpp",
        "#pragma once\nstruct Stock { int available; };\nbool reserve(Stock stock);\nvoid remove_stock(Stock& stock);\n",
    );
    put(
        root,
        "reserve.cpp",
        "#include \"stock.hpp\"\nbool reserve(Stock stock) {\n  remove_stock(stock);\n  return stock.available > 0;\n}\n",
    );
    let head = commit(root);
    (dir, base, head)
}
fn find<'a>(report: &'a Value, name: &str, commit: &str) -> &'a Value {
    report["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name && s["commit"] == commit)
        .unwrap_or_else(|| panic!("missing {name}: {report}"))
}
#[test]
fn committed_before_after_callers_types_and_direct_retrieval() {
    let (dir, base, head) = fixture();
    let root = dir.path();
    // Dirty files must never affect either snapshot or include visibility.
    put(root, "reserve.cpp", "invalid WORKTREE_SENTINEL");
    put(root, "stock.hpp", "invalid WORKTREE_SENTINEL");
    let report = json(root, &["--base", &base, "--head", &head]);
    assert_eq!(report["base"], base);
    assert_eq!(report["head"], head);
    assert_eq!(report["schema_version"], "review-context-v1");
    assert_eq!(report["changes"].as_array().unwrap().len(), 1);
    assert_eq!(report["changes"][0]["status"], "modified");
    assert!(
        find(&report, "reserve", &base)["code"]["text"]
            .as_str()
            .unwrap()
            .contains("Stock& stock")
    );
    assert!(
        find(&report, "reserve", &head)["code"]["text"]
            .as_str()
            .unwrap()
            .contains("Stock stock")
    );
    assert!(
        report["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s.get("declarations").is_none())
    );
    let with_declarations = json(
        root,
        &["--base", &base, "--head", &head, "--show-declarations"],
    );
    assert_eq!(
        find(&with_declarations, "reserve", &head)["declarations"][0]["changed"],
        true
    );
    let caller = find(&report, "checkout", &head);
    assert_eq!(caller["changed"], false);
    assert_eq!(caller["code"]["state"], "included");
    let callee = find(&report, "remove_stock", &head);
    assert_eq!(callee["signature"], "void remove_stock(Stock& stock)");
    assert_eq!(callee["code"]["state"], "omitted");
    let body = json(root, &["--body", callee["reference"].as_str().unwrap()]);
    assert!(
        body["code"]["text"]
            .as_str()
            .unwrap()
            .contains("stock.available -= 1")
    );
    let expanded = json(
        root,
        &[
            "--symbol",
            find(&report, "reserve", &head)["reference"]
                .as_str()
                .unwrap(),
            "--include-callees",
        ],
    );
    assert_eq!(expanded["base"], Value::Null);
    assert_eq!(
        find(&expanded, "remove_stock", &head)["code"]["state"],
        "included"
    );
    assert_eq!(find(&expanded, "reserve", &head)["changed"], Value::Null);
    assert_eq!(find(&report, "Stock", &head)["code"]["state"], "included");
    let again = json(root, &["--base", &base, "--head", &head]);
    assert_eq!(report, again);
    assert!(!report.to_string().contains("WORKTREE_SENTINEL"));
    assert!(!report.to_string().contains(root.to_str().unwrap()));
    let text = run(root, &["--base", &base, "--head", &head]);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.starts_with("reserve [modified]"));
    assert!(text.contains("BEFORE"));
    assert!(text.contains("AFTER"));
    assert!(!text.contains("Expansion boundary"));
}
#[test]
fn additions_removals_rename_and_overloads_have_distinct_identities() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    put(
        root,
        "old name.cpp",
        "int f(int x) { return x; }\nint f(double x) { return 1; }\nint gone() { return 2; }\n",
    );
    let base = commit(root);
    git(root, &["mv", "old name.cpp", "new name.cpp"]);
    put(
        root,
        "new name.cpp",
        "int f(int x) { return x; }\nint f(double x) { return 3; }\nint added() { return 4; }\n",
    );
    let head = commit(root);
    let report = json(root, &["--base", &base, "--head", &head, "--depth", "0"]);
    let changes = report["changes"].as_array().unwrap();
    assert!(changes.iter().any(|c| c["status"] == "removed"));
    assert!(changes.iter().any(|c| c["status"] == "added"));
    let all = report["symbols"].as_array().unwrap();
    let refs = all
        .iter()
        .map(|s| s["reference"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(refs.len(), all.len());
    for s in all {
        let body = json(root, &["--body", s["reference"].as_str().unwrap()]);
        assert_eq!(body["code"], s["code"]);
    }
    // A pure Git-detected rename pairs exact signatures despite moved line positions.
    git(root, &["mv", "new name.cpp", "renamed.cpp"]);
    let renamed = commit(root);
    let report = json(root, &["--base", &head, "--head", &renamed, "--depth", "0"]);
    assert!(
        report["changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["status"] == "renamed")
    );
}
#[test]
fn bounds_unresolved_calls_and_bad_references_are_explicit() {
    let (dir, base, _) = fixture();
    let root = dir.path();
    put(
        root,
        "new.cpp",
        "int added() { return missing_target(); }\n",
    );
    let head = commit(root);
    let report = json(
        root,
        &[
            "--base",
            &base,
            "--head",
            &head,
            "--max-symbols",
            "2",
            "--max-code-bytes",
            "1",
            "--max-edges",
            "1",
        ],
    );
    assert!(report["symbols"].as_array().unwrap().len() <= 2);
    assert!(report["relations"].as_array().unwrap().len() <= 1);
    assert!(
        report["omissions"]
            .as_object()
            .unwrap()
            .contains_key("changed-symbol-limit")
    );
    let full = json(root, &["--base", &base, "--head", &head]);
    assert!(
        full["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["resolution"] == "unresolved" && e["to"].is_null())
    );
    for args in [
        vec!["--base", "does-not-exist"],
        vec!["--body", "rc1:not-valid"],
        vec!["--base", &base, "--head", &head, "--max-input-bytes", "1"],
    ] {
        assert!(!run(root, &args).status.success());
    }
    let r = find(&full, "reserve", &head)["reference"].as_str().unwrap();
    let bad = r.replacen(&head, &base, 1);
    assert!(!run(root, &["--body", &bad]).status.success());
}

#[test]
fn recursive_context_is_deduplicated_and_body_limits_apply() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    let source = "int leaf() { return 1; }\nint middle() { return leaf(); }\nint root() { return middle(); }\n";
    put(root, "chain.cpp", source);
    let base = commit(root);
    put(
        root,
        "chain.cpp",
        &source.replace("return middle();", "return middle() + middle();"),
    );
    let head = commit(root);
    let one = json(root, &["--base", &base, "--head", &head]);
    assert!(
        !one["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "leaf")
    );
    assert!(
        one["omissions"]["unexpanded-call-symbols"]
            .as_u64()
            .unwrap()
            > 0
    );
    let two = json(
        root,
        &[
            "--base",
            &base,
            "--head",
            &head,
            "--depth",
            "2",
            "--include-callees",
        ],
    );
    assert_eq!(find(&two, "leaf", &head)["code"]["state"], "included");
    let ids = two["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["reference"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), two["symbols"].as_array().unwrap().len());
    let reference = find(&two, "leaf", &head)["reference"].as_str().unwrap();
    let limited = json(root, &["--body", reference, "--max-code-bytes", "1"]);
    assert_eq!(limited["code"]["state"], "omitted");
    assert_eq!(limited["code"]["text"], Value::Null);
    assert!(
        !run(root, &["--body", reference, "--max-input-bytes", "1"])
            .status
            .success()
    );
}

#[test]
fn line_moves_type_only_changes_and_unsupported_files_are_reported_honestly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    put(root, "types.hpp", "struct Item { int count; };\n");
    put(root, "body.cpp", "int value() { return 1; }\n");
    let base = commit(root);
    put(root, "types.hpp", "struct Item { long count; };\n");
    put(
        root,
        "body.cpp",
        "// shift the unchanged function\n\nint value() { return 1; }\n",
    );
    put(root, "script.py", "def new_function():\n    return 1\n");
    let head = commit(root);
    let report = json(root, &["--base", &base, "--head", &head]);
    assert!(report["changes"].as_array().unwrap().is_empty());
    assert_eq!(report["files"].as_array().unwrap().len(), 3);
    assert!(
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["after"] == "script.py" && f["analysis"] == "unsupported-file")
    );
    assert!(
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["after"] == "types.hpp" && f["changed_functions"] == 0)
    );
    assert_eq!(report["analyzers"][0]["id"], "cpp-tree-sitter");
}

#[test]
fn changed_overloads_and_ambiguous_targets_are_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    put(
        root,
        "one.cpp",
        "int pick(int x) { return 1; }\nint pick(double x) { return 2; }\n",
    );
    let base = commit(root);
    put(
        root,
        "one.cpp",
        "int pick(long x) { return 1; }\nint pick(float x) { return 2; }\nint caller() { return pick(unknown); }\n",
    );
    let head = commit(root);
    let report = json(root, &["--base", &base, "--head", &head]);
    assert!(
        report["changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["matching"] == "unpaired")
    );
    let edges = report["relations"].as_array().unwrap();
    assert!(edges.iter().any(|e| e["resolution"] == "ambiguous"
        && e["to"].is_null()
        && e["candidates"].as_array().unwrap().len() == 2));
}

#[cfg(unix)]
#[test]
fn references_handle_literal_paths_and_skip_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    let path = "nested/ü space[1]\\part.cpp";
    put(root, path, "int value() { return 1; }\n");
    let base = commit(root);
    put(root, path, "int value() { return 2; }\n");
    std::os::unix::fs::symlink("/unreadable/outside.cpp", root.join("link.cpp")).unwrap();
    let head = commit(root);
    let report = json(root, &["--base", &base, "--head", &head]);
    let symbol = find(&report, "value", &head);
    assert_eq!(symbol["path"], path);
    let body = json(root, &["--body", symbol["reference"].as_str().unwrap()]);
    assert_eq!(body["code"], symbol["code"]);
    assert!(report["omissions"]["non-regular-file"].as_u64().unwrap() > 0);
    assert!(!run(root, &["--base=--help"]).status.success());
}

#[test]
fn declarations_are_opt_in_for_comparison_and_expansion() {
    let (dir, base, head) = fixture();
    let root = dir.path();
    let args = ["--base", base.as_str(), "--head", head.as_str()];
    let report = json(root, &args);
    assert_eq!(report["limits"]["show_declarations"], false);
    let text = String::from_utf8(run(root, &args).stdout).unwrap();
    assert!(!text.lines().any(|line| line.starts_with("declaration ")));
    let mut enabled = args.to_vec();
    enabled.push("--show-declarations");
    let with = json(root, &enabled);
    assert_eq!(with["limits"]["show_declarations"], true);
    assert_eq!(with["changes"], report["changes"]);
    let text = String::from_utf8(run(root, &enabled).stdout).unwrap();
    assert!(text.contains("declaration stock.hpp:3-3 [changed]"));
    let symbol = find(&with, "reserve", &head);
    let reference = symbol["reference"].as_str().unwrap();
    let decl_reference = symbol["declarations"][0]["reference"].as_str().unwrap();
    let plain = json(root, &["--symbol", reference]);
    let expanded = json(root, &["--symbol", reference, "--show-declarations"]);
    assert!(
        plain["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s.get("declarations").is_none())
    );
    assert_eq!(
        find(&expanded, "reserve", &head)["declarations"][0]["changed"],
        Value::Null
    );
    let text = String::from_utf8(run(root, &["--symbol", reference, "--show-declarations"]).stdout)
        .unwrap();
    assert!(text.contains("declaration stock.hpp:3-3 [context]"));
    // Explicit retrieval remains exact regardless of presentation policy.
    let direct = json(root, &["--body", decl_reference]);
    assert_eq!(direct["code"]["text"], "bool reserve(Stock stock);");
    assert_eq!(
        direct,
        json(root, &["--body", decl_reference, "--show-declarations"])
    );
    let help = String::from_utf8(run(root, &["--help"]).stdout).unwrap();
    assert!(help.contains("--show-declarations"));
}

#[test]
fn declaration_only_changes_remain_visible_without_declaration_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    put(
        root,
        "api.hpp",
        "int value(int count = 1);\nint external(int count = 1);\n",
    );
    put(
        root,
        "api.cpp",
        "#include \"api.hpp\"\nint value(int count) { return count; }\n",
    );
    let base = commit(root);
    put(
        root,
        "api.hpp",
        "int value(int count = 2);\nint external(int count = 2);\n",
    );
    let head = commit(root);
    let report = json(root, &["--base", &base, "--head", &head]);
    assert_eq!(report["changes"].as_array().unwrap().len(), 2);
    assert!(
        report["changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["reason"] == "declaration-only")
    );
    assert_eq!(
        find(&report, "value", &base)["code"]["text"],
        find(&report, "value", &head)["code"]["text"]
    );
    assert_eq!(find(&report, "value", &head)["changed"], true);
    assert!(
        find(&report, "value", &head)
            .get("other_snapshots")
            .is_none()
    );
    assert!(
        find(&report, "value", &base)
            .get("other_snapshots")
            .is_none()
    );
    assert_eq!(
        find(&report, "external", &head)["code"]["reason"],
        "declaration-policy"
    );
    assert!(find(&report, "external", &head)["signature"].is_null());
    assert!(!report.to_string().contains("count = 2"));
    let text = String::from_utf8(run(root, &["--base", &base, "--head", &head]).stdout).unwrap();
    assert!(text.contains("value [modified; declaration-only]"));
    assert!(!text.contains("count = 2"));
    let with = json(
        root,
        &["--base", &base, "--head", &head, "--show-declarations"],
    );
    assert_eq!(with["changes"], report["changes"]);
    assert_eq!(
        find(&with, "value", &head)["declarations"][0]["code"]["text"],
        "int value(int count = 2);"
    );
    assert_eq!(
        find(&with, "external", &head)["code"]["text"],
        "int external(int count = 2);"
    );
}

#[test]
fn shared_context_preserves_snapshot_references_and_declarations() {
    let (dir, base, head) = fixture();
    let root = dir.path();
    let args = [
        "--base",
        base.as_str(),
        "--head",
        head.as_str(),
        "--include-callees",
        "--show-declarations",
    ];
    let report = json(root, &args);
    let symbols = report["symbols"].as_array().unwrap();
    assert_eq!(
        symbols.iter().filter(|s| s["name"] == "checkout").count(),
        1
    );
    assert_eq!(symbols.iter().filter(|s| s["name"] == "Stock").count(), 1);
    assert_eq!(symbols.iter().filter(|s| s["name"] == "reserve").count(), 2);
    for commit in [&base, &head] {
        assert!(
            find(&report, "reserve", commit)
                .get("other_snapshots")
                .is_none()
        );
    }
    let caller = find(&report, "checkout", &head);
    assert_eq!(
        find(&report, "Stock", &head)["origin"]["from"],
        find(&report, "reserve", &head)["reference"]
    );
    assert_eq!(caller["other_snapshots"][0]["commit"], base);
    assert_eq!(caller["other_snapshots"][0]["path"], caller["path"]);
    let mut bytes = 0;
    for s in symbols {
        bytes += s["code"]["text"].as_str().map(str::len).unwrap_or(0);
        if let Some(ds) = s["declarations"].as_array() {
            bytes += ds
                .iter()
                .map(|d| d["code"]["text"].as_str().map(str::len).unwrap_or(0))
                .sum::<usize>();
        }
        for alias in s["other_snapshots"].as_array().into_iter().flatten() {
            let body = json(root, &["--body", alias["reference"].as_str().unwrap()]);
            assert_eq!(body["code"], s["code"]);
            for declaration in alias["declarations"].as_array().into_iter().flatten() {
                let body = json(
                    root,
                    &["--body", declaration["reference"].as_str().unwrap()],
                );
                assert!(
                    s["declarations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|d| d["code"] == body["code"])
                );
            }
        }
    }
    // Sharing happens before charging the source budget.
    let bytes = bytes.to_string();
    let mut limited = args.to_vec();
    limited.extend(["--max-code-bytes", &bytes]);
    assert_eq!(json(root, &limited)["symbols"], report["symbols"]);
    let alias = caller["other_snapshots"][0]["reference"].as_str().unwrap();
    let expanded = json(root, &["--symbol", alias, "--show-declarations"]);
    assert_eq!(expanded["head"], base);
    assert_eq!(find(&expanded, "checkout", &base)["code"], caller["code"]);
    assert!(
        expanded["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s.get("other_snapshots").is_none())
    );
    let text = String::from_utf8(run(root, &args).stdout).unwrap();
    assert_eq!(text.matches("checkout [unchanged;").count(), 1);
    assert!(text.contains("same source @"));
}

#[test]
fn identical_bodies_in_different_files_are_not_one_context_symbol() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    put(root, "api.hpp", "int target();\n");
    put(
        root,
        "target.cpp",
        "#include \"api.hpp\"\nint target() { return 1; }\n",
    );
    for path in ["a.cpp", "b.cpp"] {
        put(
            root,
            path,
            "#include \"api.hpp\"\nint main() { return target(); }\n",
        );
    }
    let base = commit(root);
    put(
        root,
        "target.cpp",
        "#include \"api.hpp\"\nint target() { return 2; }\n",
    );
    let head = commit(root);
    let report = json(root, &["--base", &base, "--head", &head]);
    let callers = report["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["name"] == "main")
        .collect::<Vec<_>>();
    assert_eq!(callers.len(), 2);
    assert_eq!(callers[0]["code"]["text"], callers[1]["code"]["text"]);
    assert_ne!(callers[0]["path"], callers[1]["path"]);
    assert_ne!(callers[0]["reference"], callers[1]["reference"]);
    for caller in callers {
        assert_eq!(caller["other_snapshots"].as_array().unwrap().len(), 1);
        assert_eq!(caller["other_snapshots"][0]["path"], caller["path"]);
    }
}

#[test]
fn incident_relation_default_does_not_prune_recursive_context() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["checkout", "-qb", "codex/fixture"]);
    let source = "int leaf() { return 1; }\nint middle() { return leaf(); }\nint root() { return middle(); }\n";
    put(root, "chain.cpp", source);
    let base = commit(root);
    put(
        root,
        "chain.cpp",
        &source.replace("return middle();", "return middle() + 1;"),
    );
    let head = commit(root);
    let args = [
        "--base",
        base.as_str(),
        "--head",
        head.as_str(),
        "--depth",
        "2",
    ];
    let report = json(root, &args);
    let seeds = report["changes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| [c["before"].as_str().unwrap(), c["after"].as_str().unwrap()])
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        report["relations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| seeds.contains(e["from"].as_str().unwrap())
                || e["to"].as_str().is_some_and(|id| seeds.contains(id)))
    );
    let middle = find(&report, "middle", &head);
    let leaf = find(&report, "leaf", &head);
    assert_eq!(leaf["origin"]["from"], middle["reference"]);
    assert_eq!(leaf["origin"]["resolution"], "exact");
    assert!(report["omissions"]["context-relations"].as_u64().unwrap() > 0);
    let mut all_args = args.to_vec();
    all_args.push("--all-relations");
    let all = json(root, &all_args);
    assert_eq!(all["symbols"], report["symbols"]);
    assert_eq!(all["changes"], report["changes"]);
    assert!(all["omissions"].get("context-relations").is_none());
    assert!(
        all["relations"].as_array().unwrap().len() > report["relations"].as_array().unwrap().len()
    );
    assert!(
        all["relations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["to"] == leaf["other_snapshots"][0]["reference"] && e["snapshot"] == base)
    );
    let seed = find(&report, "root", &head)["reference"].as_str().unwrap();
    let expanded = json(root, &["--symbol", seed, "--depth", "2"]);
    assert!(
        expanded["relations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["from"] == seed || e["to"] == seed)
    );
    assert_eq!(
        find(&expanded, "leaf", &head)["origin"]["from"],
        middle["reference"]
    );
    let text = String::from_utf8(run(root, &args).stdout).unwrap();
    assert!(text.contains("via middle [callee; exact]"));
    assert!(text.contains("(--all-relations)"));
    let help = String::from_utf8(run(root, &["--help"]).stdout).unwrap();
    assert!(help.contains("--all-relations"));
}
