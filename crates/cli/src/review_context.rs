use crate::bootstrap::{BuiltinAnalyzerFeatures, build_builtin_analyzer_registry};
use clap::{Args, ValueEnum};
use codegraide_core::git_snapshot::{GitRepository, SnapshotError};
use codegraide_core::review_context::{
    ContextLimits, ContextReport, ContextSnapshot, assemble_context, render_context,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum Format {
    #[default]
    Terminal,
    Json,
}
#[derive(Debug, Args)]
pub struct ReviewContextArgs {
    /// Git repository; only committed files are read
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Compare this commit directly with --head (not a merge-base comparison)
    #[arg(long,required_unless_present_any=["body","symbol"],conflicts_with_all=["body","symbol"])]
    base: Option<String>,
    /// Head revision for comparison; defaults to HEAD
    #[arg(long,conflicts_with_all=["body","symbol"])]
    head: Option<String>,
    /// Retrieve source directly using a snapshot-qualified reference from JSON
    #[arg(long, value_name = "REF", conflicts_with = "symbol")]
    body: Option<String>,
    /// Expand callers/callees for this snapshot-qualified function reference
    #[arg(long, value_name = "REF")]
    symbol: Option<String>,
    /// Number of caller/callee hops (0-10)
    #[arg(long,default_value="1",value_parser=clap::value_parser!(u8).range(0..=10))]
    depth: u8,
    /// Include complete unchanged callee bodies
    #[arg(long)]
    include_callees: bool,
    /// Include declaration source in terminal and JSON output (off by default)
    #[arg(long)]
    show_declarations: bool,
    /// Include relationships from all expanded context
    #[arg(long)]
    all_relations: bool,
    /// Maximum emitted symbol records, including both revisions
    #[arg(long,default_value="200",value_parser=clap::value_parser!(u32).range(1..=10000))]
    max_symbols: u32,
    /// Maximum emitted relationships
    #[arg(long,default_value="1000",value_parser=clap::value_parser!(u32).range(1..=100000))]
    max_edges: u32,
    /// Source-code byte budget; bodies are omitted whole when it is exhausted
    #[arg(long,default_value="1048576",value_parser=clap::value_parser!(u32).range(1..))]
    max_code_bytes: u32,
    /// Maximum total C++ source bytes read per snapshot; exceeding it is an error
    #[arg(long,default_value="67108864",value_parser=clap::value_parser!(u32).range(1..))]
    max_input_bytes: u32,
    #[arg(long, value_enum, default_value = "terminal")]
    format: Format,
}
fn snapshot(
    repo: &GitRepository,
    commit: &str,
    max_bytes: usize,
) -> Result<ContextSnapshot, SnapshotError> {
    let git = repo.snapshot(commit, max_bytes)?;
    let mut registry = build_builtin_analyzer_registry(BuiltinAnalyzerFeatures {
        documentation: false,
    })
    .map_err(|e| SnapshotError(e.to_string()))?;
    let analysis = codegraide_core::analyze_source_files(
        git.files
            .iter()
            .map(|(p, f)| (p.clone(), f.source.as_bytes().to_vec()))
            .collect(),
        &mut registry,
    );
    let dependencies = codegraide_analyzer_cpp::resolve_cpp_snapshot_dependencies(&analysis)
        .map_err(|e| SnapshotError(e.to_string()))?;
    let mut resolution = codegraide_analyzer_cpp::resolve_cpp_calls(&analysis, &dependencies);
    // Resolution may use a declaration's default arguments. Display the actual
    // definition signature so hidden declarations cannot leak through metadata.
    let definition_signatures = analysis
        .analyzers
        .iter()
        .flat_map(|run| &run.files)
        .flat_map(|file| {
            file.facts.symbols.iter().filter_map(|symbol| {
                symbol.callable_signature.as_ref().map(|signature| {
                    (
                        (file.path.clone(), symbol.span.start_byte),
                        signature.display.clone(),
                    )
                })
            })
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for symbol in &mut resolution.symbols {
        if let (Some(definition), Some(signature)) = (&symbol.definition, &mut symbol.signature) {
            if let Some(display) =
                definition_signatures.get(&(definition.path.clone(), definition.span.start_byte))
            {
                signature.display.clone_from(display);
            }
        }
    }
    let mut diagnostics = resolution.diagnostics;
    for run in &analysis.analyzers {
        for file in &run.files {
            if file.status != codegraide_core::FileAnalysisStatus::Successful {
                diagnostics.push(format!(
                    "{}: parse-{}",
                    file.path.display(),
                    file.status.as_str()
                ));
            }
        }
    }
    Ok(ContextSnapshot::new(
        git,
        resolution.symbols,
        resolution.resolutions,
        diagnostics,
        analysis
            .analyzers
            .iter()
            .filter(|run| run.descriptor.language.as_str() == "cpp")
            .map(|run| run.descriptor.clone())
            .collect(),
    ))
}
fn execute(args: &ReviewContextArgs) -> Result<String, SnapshotError> {
    let repo = GitRepository::open(&args.path)?;
    if let Some(reference) = &args.body {
        let (r, source) = repo.retrieve(reference, args.max_input_bytes as usize)?;
        let text = source
            .get(r.start..r.end)
            .ok_or_else(|| SnapshotError("invalid source range".into()))?;
        let start_line = source[..r.start].bytes().filter(|b| *b == b'\n').count() + 1;
        let end_line = start_line + text.bytes().filter(|b| *b == b'\n').count();
        let code = codegraide_core::review_context::Code::from_source(
            Some(text),
            true,
            &mut (args.max_code_bytes as usize),
        );
        return match args.format {
            Format::Json=>serde_json::to_string_pretty(&serde_json::json!({"schema_version":codegraide_core::review_context::SCHEMA_VERSION,"commit":r.commit,"reference":reference,"path":r.path,"start_line":start_line,"end_line":end_line,"code":code})).map_err(|e|SnapshotError(e.to_string())),
            Format::Terminal=>Ok(format!("{}:{}-{} @{}\n{}",r.path,start_line,end_line,r.commit,
                code.text.as_deref().unwrap_or("code [omitted: code-byte-limit]"))),
        };
    }
    let limits = ContextLimits {
        depth: args.depth.into(),
        max_symbols: args.max_symbols as usize,
        max_edges: args.max_edges as usize,
        max_code_bytes: args.max_code_bytes as usize,
        include_callees: args.include_callees,
        show_declarations: args.show_declarations,
        all_relations: args.all_relations,
    };
    let report: ContextReport = if let Some(reference) = &args.symbol {
        let (r, _) = repo.retrieve(reference, args.max_input_bytes as usize)?;
        let head = snapshot(&repo, &r.commit, args.max_input_bytes as usize)?;
        if !head.symbols.contains_key(reference) {
            return Err(SnapshotError("reference is not a symbol boundary in this snapshot; use --body for source retrieval".into()));
        }
        assemble_context(None, &head, &Default::default(), Some(reference), limits)
    } else {
        let base_commit = repo.resolve(
            args.base
                .as_deref()
                .ok_or_else(|| SnapshotError("--base is required".into()))?,
        )?;
        let head_commit = repo.resolve(args.head.as_deref().unwrap_or("HEAD"))?;
        let base = snapshot(&repo, &base_commit, args.max_input_bytes as usize)?;
        let head = snapshot(&repo, &head_commit, args.max_input_bytes as usize)?;
        let renames = repo.renames(&base_commit, &head_commit)?;
        assemble_context(Some(&base), &head, &renames, None, limits)
    };
    match args.format {
        Format::Json => {
            serde_json::to_string_pretty(&report).map_err(|e| SnapshotError(e.to_string()))
        }
        Format::Terminal => Ok(render_context(&report)),
    }
}
pub fn run(args: &ReviewContextArgs) -> ExitCode {
    match execute(args) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
