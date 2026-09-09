use codegraide_core::review_context::{ContextOrigin, ContextReport, ContextSymbol, SourceRecord};
use std::collections::{BTreeMap, BTreeSet};

pub fn render_context(report: &ContextReport) -> String {
    let mut out = String::new();
    let symbols: BTreeMap<_, _> = report
        .symbols
        .iter()
        .flat_map(|s| {
            std::iter::once((s.source.reference.as_str(), s)).chain(
                s.other_snapshots
                    .iter()
                    .map(move |alias| (alias.source.reference.as_str(), s)),
            )
        })
        .collect();
    let mut rendered = BTreeSet::new();
    for change in &report.changes {
        let name = change
            .after
            .as_deref()
            .or(change.before.as_deref())
            .and_then(|id| symbols.get(id))
            .map(|s| s.name.as_str())
            .unwrap_or("function");
        let reason = change
            .reason
            .map(|reason| format!("; {reason}"))
            .unwrap_or_default();
        out.push_str(&format!("{name} [{}{reason}]\n", change.status));
        for (label, id) in [("BEFORE", &change.before), ("AFTER", &change.after)] {
            if let Some(id) = id {
                if let Some(s) = symbols.get(id.as_str()) {
                    out.push_str(&format!("{label} "));
                    write_source(&mut out, &s.source);
                    for d in &s.declarations {
                        out.push_str("declaration ");
                        write_source(&mut out, d);
                    }
                    rendered.insert(id.as_str());
                }
            } else {
                out.push_str(&format!("{label} [absent]\n"));
            }
        }
        out.push('\n');
    }
    for s in &report.symbols {
        if rendered.contains(s.source.reference.as_str()) {
            continue;
        }
        out.push_str(&format!(
            "{} [{}; {}]\n",
            s.name,
            change_label(s.source.changed),
            s.roles.iter().cloned().collect::<Vec<_>>().join(",")
        ));
        write_source(&mut out, &s.source);
        write_origin(
            &mut out,
            &s.source.reference,
            s.origin.as_ref(),
            report,
            &symbols,
        );
        for other in &s.other_snapshots {
            out.push_str(&format!(
                "same source @{} {}:{}-{}\n",
                &other.source.commit[..12],
                other.source.path,
                other.source.start_line,
                other.source.end_line
            ));
            write_origin(
                &mut out,
                &other.source.reference,
                other.origin.as_ref(),
                report,
                &symbols,
            );
            for declaration in &other.declarations {
                out.push_str(&format!(
                    "same declaration @{} {}:{}-{}\n",
                    &declaration.commit[..12],
                    declaration.path,
                    declaration.start_line,
                    declaration.end_line
                ));
            }
        }
        if s.source.code.state != "included" {
            if let Some(sig) = &s.signature {
                out.push_str(&format!("  {sig}\n"));
            }
        }
        for d in &s.declarations {
            out.push_str("declaration ");
            write_source(&mut out, d);
        }
        out.push('\n');
    }
    if !report.relations.is_empty() {
        out.push_str("Relations\n");
    }
    for e in &report.relations {
        out.push_str(&format!(
            "  {} -> {} [{}; {}] {}:{} @{}\n",
            e.from_name,
            e.to_name.as_deref().unwrap_or(&e.expression),
            e.relation,
            e.resolution,
            e.path,
            e.line,
            &e.snapshot[..12]
        ));
    }
    for file in report.files.iter().filter(|f| f.changed_functions == 0) {
        out.push_str(&format!(
            "file {} [{}; {}; no changed function body]\n",
            file.after
                .as_deref()
                .or(file.before.as_deref())
                .unwrap_or(""),
            file.status,
            file.analysis
        ));
    }
    for (kind, count) in &report.omissions {
        if *count > 0 {
            let option = if kind == "context-relations" {
                " (--all-relations)"
            } else {
                ""
            };
            out.push_str(&format!("omitted {kind}={count}{option}\n"));
        }
    }
    for diagnostic in &report.diagnostics {
        out.push_str(&format!("! {diagnostic}\n"));
    }
    out
}
fn write_origin(
    out: &mut String,
    reference: &str,
    origin: Option<&ContextOrigin>,
    report: &ContextReport,
    symbols: &BTreeMap<&str, &ContextSymbol>,
) {
    let Some(origin) = origin else {
        return;
    };
    let visible = report
        .relations
        .iter()
        .any(|edge| match origin.role.as_str() {
            "caller" => edge.from == reference && edge.to.as_deref() == Some(origin.from.as_str()),
            _ => {
                edge.from == origin.from
                    && (edge.to.as_deref() == Some(reference)
                        || edge.candidates.iter().any(|id| id == reference))
            }
        });
    if !visible {
        if let Some(parent) = symbols.get(origin.from.as_str()) {
            // The reference itself identifies the original snapshot, including aliases.
            let commit = origin.from.split(':').nth(1).unwrap_or("");
            out.push_str(&format!(
                "via {} [{}; {}] @{}\n",
                parent.name,
                origin.role,
                origin.resolution,
                &commit[..commit.len().min(12)]
            ));
        }
    }
}

fn write_source(out: &mut String, s: &SourceRecord) {
    out.push_str(&format!(
        "{}:{}-{} [{}] @{}\n",
        s.path,
        s.start_line,
        s.end_line,
        change_label(s.changed),
        &s.commit[..12]
    ));
    if let Some(text) = &s.code.text {
        for (i, line) in text.lines().enumerate() {
            out.push_str(&format!("{:>4}  {line}\n", s.start_line + i));
        }
    } else {
        out.push_str(&format!(
            "code [{}: {}]\n",
            s.code.state,
            s.code.reason.unwrap_or("unspecified")
        ));
    }
}

fn change_label(changed: Option<bool>) -> &'static str {
    match changed {
        Some(true) => "changed",
        Some(false) => "unchanged",
        None => "context",
    }
}
