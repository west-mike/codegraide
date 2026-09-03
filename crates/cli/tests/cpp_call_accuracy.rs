use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: String,
    reviewed: bool,
    repositories: Vec<RepositoryLabels>,
}

#[derive(Debug, Deserialize)]
struct RepositoryLabels {
    name: String,
    revision: String,
    relative_path: String,
    labels: Vec<CallLabel>,
}

#[derive(Debug, Deserialize)]
struct CallLabel {
    path: String,
    line: usize,
    column: usize,
    form: String,
    expected_owner: String,
    expected_status: String,
    correct_targets: Vec<String>,
    runtime_dispatch: bool,
}

#[test]
#[ignore = "requires CODEGRAIDE_CPP_CORPUS_ROOT with pinned argparse, Catch2, and OpenCV checkouts"]
fn reviewed_cpp_call_accuracy_corpus() {
    let Some(corpus_root) = std::env::var_os("CODEGRAIDE_CPP_CORPUS_ROOT").map(PathBuf::from)
    else {
        eprintln!("CODEGRAIDE_CPP_CORPUS_ROOT is not set; corpus accuracy run skipped");
        return;
    };
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cpp-call-accuracy.json");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest_path).expect("reviewed C++ accuracy manifest should be readable"),
    )
    .expect("reviewed C++ accuracy manifest should be valid JSON");
    assert_eq!(manifest.schema_version, "0.1.0");
    assert!(
        manifest.reviewed,
        "the stratified baseline must be independently reviewed before it can enforce precision gates"
    );
    assert_eq!(
        manifest
            .repositories
            .iter()
            .map(|repository| repository.labels.len())
            .collect::<Vec<_>>(),
        [80, 60, 60]
    );

    let output_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/cpp-call-accuracy");
    fs::create_dir_all(&output_root).expect("accuracy output directory");
    let mut totals = AccuracyTotals::default();
    for repository in &manifest.repositories {
        let checkout = corpus_root.join(&repository.relative_path);
        assert_eq!(git_revision(&checkout), repository.revision);
        let started = Instant::now();
        let first = run_report(&checkout);
        let elapsed = started.elapsed();
        let second = run_report(&checkout);
        assert_eq!(
            first, second,
            "{} JSON changed between runs",
            repository.name
        );
        fs::write(
            output_root.join(format!("{}.json", repository.name)),
            &first,
        )
        .expect("accuracy report should be written");
        let report: Value = serde_json::from_slice(&first).expect("call report JSON");
        evaluate(repository, &report, &mut totals);
        let coverage = &report["coverage"];
        eprintln!(
            "{}: elapsed={:.3}s resolved={}/{} peak-memory=record-with-/usr/bin/time",
            repository.name,
            elapsed.as_secs_f64(),
            coverage["exact_calls"].as_u64().unwrap_or(0)
                + coverage["inferred_calls"].as_u64().unwrap_or(0),
            coverage["total_calls"].as_u64().unwrap_or(0),
        );
    }
    assert_eq!(
        totals.exact_correct, totals.exact_total,
        "exact precision must be 100%"
    );
    assert!(ratio(totals.unique_correct, totals.unique_total) >= 0.95);
    assert!(ratio(totals.ambiguous_recalled, totals.ambiguous_total) >= 0.90);
    assert_eq!(totals.ownership_correct, 200);
}

#[derive(Default)]
struct AccuracyTotals {
    exact_correct: usize,
    exact_total: usize,
    unique_correct: usize,
    unique_total: usize,
    ambiguous_recalled: usize,
    ambiguous_total: usize,
    ownership_correct: usize,
}

fn evaluate(repository: &RepositoryLabels, report: &Value, totals: &mut AccuracyTotals) {
    let nodes = report["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|node| (node["id"].as_str().unwrap(), node))
        .collect::<BTreeMap<_, _>>();
    let relations = report["relations"].as_array().expect("relations");
    for label in &repository.labels {
        let relation = relations
            .iter()
            .find(|relation| {
                relation["evidence"].as_array().is_some_and(|evidence| {
                    evidence.iter().any(|item| {
                        item["source_path"] == label.path
                            && item["line"] == label.line
                            && item["column"] == label.column
                            && item["form"] == label.form
                    })
                })
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing label {}:{}:{}",
                    label.path, label.line, label.column
                )
            });
        assert_eq!(relation["kind"], label.expected_status);
        let owner = nodes[relation["source"].as_str().unwrap()]["selector"]
            .as_str()
            .unwrap();
        assert_eq!(owner, label.expected_owner);
        totals.ownership_correct += 1;

        let predicted = candidate_targets(relation, &nodes);
        let correct = label
            .correct_targets
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        match label.expected_status.as_str() {
            "exact" => {
                totals.exact_total += 1;
                totals.unique_total += 1;
                if predicted == correct {
                    totals.exact_correct += 1;
                    totals.unique_correct += 1;
                }
            }
            "inferred" => {
                totals.unique_total += 1;
                if predicted == correct {
                    totals.unique_correct += 1;
                }
            }
            "ambiguous" if !label.runtime_dispatch && !correct.is_empty() => {
                totals.ambiguous_total += 1;
                if correct.is_subset(&predicted) {
                    totals.ambiguous_recalled += 1;
                }
            }
            _ => {}
        }
    }
}

fn candidate_targets(relation: &Value, nodes: &BTreeMap<&str, &Value>) -> BTreeSet<String> {
    if relation["kind"] == "ambiguous" {
        return nodes[relation["target"].as_str().unwrap()]["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|candidate| candidate["selector"].as_str().map(str::to_owned))
            .collect();
    }
    nodes[relation["target"].as_str().unwrap()]["selector"]
        .as_str()
        .map(|target| BTreeSet::from([target.to_owned()]))
        .unwrap_or_default()
}

fn run_report(checkout: &Path) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_codegraide"))
        .args([
            "calls",
            checkout.to_str().unwrap(),
            "--language",
            "cpp",
            "--format",
            "json",
        ])
        .output()
        .expect("codegraide should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git_revision(checkout: &Path) -> String {
    let output = Command::new("git")
        .args(["-C", checkout.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .expect("git should read corpus revision");
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}
