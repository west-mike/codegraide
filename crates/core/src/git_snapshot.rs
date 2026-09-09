//! Read committed source without checking out revisions or consulting the worktree.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub enum SnapshotError {
    Invalid(String),
    Limit {
        resource: &'static str,
        limit: usize,
    },
    Io {
        operation: String,
        source: std::io::Error,
    },
    Git {
        operation: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
    Analysis(Box<dyn std::error::Error>),
}
impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Limit { resource, limit } => write!(f, "snapshot exceeds {resource} {limit}"),
            Self::Io { operation, source } => write!(f, "{operation}: {source}"),
            Self::Git {
                operation,
                status,
                stderr,
            } => write!(f, "git {operation} failed ({status}): {stderr}"),
            Self::Analysis(source) => source.fmt(f),
        }
    }
}
impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Analysis(source) => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// Metadata and elapsed-time bounds are independent of the source-byte budget.
#[derive(Debug, Clone, Copy)]
pub struct GitLimits {
    pub metadata_bytes: usize,
    pub entries: usize,
    pub timeout: std::time::Duration,
}
impl Default for GitLimits {
    fn default() -> Self {
        Self {
            metadata_bytes: 16 * 1024 * 1024,
            entries: 100_000,
            timeout: std::time::Duration::from_secs(120),
        }
    }
}
type Result<T> = std::result::Result<T, SnapshotError>;

pub struct GitRepository {
    root: PathBuf,
    limits: GitLimits,
    deadline: std::time::Instant,
}
#[derive(Debug, Clone)]
pub struct SnapshotFile {
    pub object: String,
    pub source: String,
}
#[derive(Debug)]
pub struct GitSnapshot {
    pub commit: String,
    pub files: BTreeMap<PathBuf, SnapshotFile>,
    pub excluded: Vec<(String, String)>,
    /// All tracked entries, including files the selected analyzers cannot read.
    pub entries: BTreeMap<String, (String, String)>,
}
impl GitRepository {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_limits(path, GitLimits::default())
    }
    pub fn open_with_limits(path: &Path, limits: GitLimits) -> Result<Self> {
        let result = Self {
            root: path.to_path_buf(),
            limits,
            deadline: std::time::Instant::now() + limits.timeout,
        };
        result.git(&["rev-parse", "--git-dir"])?;
        Ok(result)
    }
    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        self.command(args, None, self.limits.metadata_bytes)
    }
    fn command(
        &self,
        args: &[&str],
        input: Option<std::fs::File>,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let mut command = Command::new("git");
        command
            .arg("--no-replace-objects")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(
                input
                    .map(std::process::Stdio::from)
                    .unwrap_or_else(std::process::Stdio::null),
            );
        let output = crate::process::run(&mut command, self.deadline, limit).map_err(|source| {
            SnapshotError::Io {
                operation: format!("git {}", args[0]),
                source,
            }
        })?;
        if !output.status.success() {
            return Err(SnapshotError::Git {
                operation: args[0].into(),
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }
        Ok(output.stdout)
    }
    pub fn resolve(&self, revision: &str) -> Result<String> {
        if revision.starts_with('-') {
            return Err(SnapshotError::Invalid(
                "revision must not start with '-'".into(),
            ));
        }
        let bytes = self.git(&["rev-parse", "--verify", &format!("{revision}^{{commit}}")])?;
        let commit = String::from_utf8_lossy(&bytes).trim().to_owned();
        if !valid_oid(&commit) {
            return Err(SnapshotError::Invalid(
                "Git returned an invalid commit ID".into(),
            ));
        }
        Ok(commit)
    }
    pub fn snapshot(
        &self,
        commit: &str,
        max_bytes: usize,
        include: impl Fn(&Path) -> bool,
    ) -> Result<GitSnapshot> {
        let commit = self.resolve(commit)?;
        let tree = self.git(&["ls-tree", "-rzl", "--full-tree", &commit])?;
        let mut result = GitSnapshot {
            commit,
            files: BTreeMap::new(),
            excluded: Vec::new(),
            entries: BTreeMap::new(),
        };
        let mut total = 0usize;
        let mut selected = Vec::new();
        for (index, entry) in tree
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .enumerate()
        {
            if index >= self.limits.entries {
                return Err(SnapshotError::Limit {
                    resource: "tree entries",
                    limit: self.limits.entries,
                });
            }
            let (mode, object, path) = tree_entry(entry)?;
            result
                .entries
                .insert(path.into(), (mode.into(), object.into()));
            if !matches!(mode, "100644" | "100755") {
                result
                    .excluded
                    .push((path.into(), "non-regular-file".into()));
                continue;
            }
            if !include(Path::new(path)) {
                result
                    .excluded
                    .push((path.into(), "unsupported-file".into()));
                continue;
            }
            let size = utf8(entry)?
                .split_once('\t')
                .and_then(|(meta, _)| meta.split_whitespace().nth(3))
                .and_then(|size| size.parse::<usize>().ok())
                .ok_or_else(|| SnapshotError::Invalid("invalid Git tree size".into()))?;
            total = total.checked_add(size).ok_or(SnapshotError::Limit {
                resource: "--max-input-bytes",
                limit: max_bytes,
            })?;
            if total > max_bytes {
                return Err(SnapshotError::Limit {
                    resource: "--max-input-bytes",
                    limit: max_bytes,
                });
            }
            selected.push((path, object, size));
        }
        if !selected.is_empty() {
            use std::io::{Seek, SeekFrom, Write};
            let input_error = |source| SnapshotError::Io {
                operation: "prepare Git batch input".into(),
                source,
            };
            let mut input = tempfile::tempfile().map_err(input_error)?;
            for (_, object, _) in &selected {
                writeln!(input, "{object}").map_err(input_error)?;
            }
            input.seek(SeekFrom::Start(0)).map_err(input_error)?;
            let bound = total
                .checked_add(selected.len().saturating_mul(128))
                .ok_or(SnapshotError::Limit {
                    resource: "--max-input-bytes",
                    limit: max_bytes,
                })?;
            let bytes = self.command(&["cat-file", "--batch"], Some(input), bound)?;
            let mut remaining = bytes.as_slice();
            for (path, object, size) in selected {
                let newline = remaining
                    .iter()
                    .position(|b| *b == b'\n')
                    .ok_or_else(|| SnapshotError::Invalid("missing Git batch header".into()))?;
                if utf8(&remaining[..newline])? != format!("{object} blob {size}") {
                    return Err(SnapshotError::Invalid("Git batch object mismatch".into()));
                }
                remaining = &remaining[newline + 1..];
                let body = remaining
                    .get(..size)
                    .ok_or_else(|| SnapshotError::Invalid("truncated Git batch blob".into()))?;
                match std::str::from_utf8(body) {
                    Ok(source) if !source.contains('\0') => {
                        result.files.insert(
                            path.into(),
                            SnapshotFile {
                                object: object.into(),
                                source: source.into(),
                            },
                        );
                    }
                    _ => result
                        .excluded
                        .push((path.into(), "non-utf8-or-binary".into())),
                }
                if remaining.get(size) != Some(&b'\n') {
                    return Err(SnapshotError::Invalid("invalid Git batch delimiter".into()));
                }
                remaining = &remaining[size + 1..];
            }
            if !remaining.is_empty() {
                return Err(SnapshotError::Invalid("unexpected Git batch output".into()));
            }
        }
        Ok(result)
    }
    pub fn renames(&self, base: &str, head: &str) -> Result<BTreeMap<PathBuf, PathBuf>> {
        let bytes = self.git(&[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-status",
            "-z",
            "-M",
            base,
            head,
            "--",
        ])?;
        let mut fields = bytes.split(|b| *b == 0).filter(|s| !s.is_empty());
        let mut result = BTreeMap::new();
        while let Some(status) = fields.next() {
            let first = fields
                .next()
                .ok_or_else(|| SnapshotError::Invalid("invalid Git diff path".into()))?;
            if status.starts_with(b"R") || status.starts_with(b"C") {
                let second = fields
                    .next()
                    .ok_or_else(|| SnapshotError::Invalid("invalid Git rename".into()))?;
                if status.starts_with(b"R") {
                    result.insert(utf8(first)?.into(), utf8(second)?.into());
                }
            }
        }
        Ok(result)
    }
    pub fn retrieve(&self, reference: &str, max_bytes: usize) -> Result<(SourceReference, String)> {
        let r = SourceReference::parse(reference)?;
        if self.resolve(&r.commit)? != r.commit {
            return Err(SnapshotError::Invalid(
                "reference commit must be a full ID".into(),
            ));
        }
        let spec = format!(":(literal){}", r.path);
        let tree = self.git(&["ls-tree", "-rz", "--full-tree", &r.commit, "--", &spec])?;
        let entry = tree
            .split(|b| *b == 0)
            .find(|s| !s.is_empty())
            .ok_or_else(|| {
                SnapshotError::Invalid("reference path is absent from snapshot".into())
            })?;
        let (mode, object, path) = tree_entry(entry)?;
        if !matches!(mode, "100644" | "100755") || path != r.path || object != r.object {
            return Err(SnapshotError::Invalid(
                "reference does not match the snapshot blob".into(),
            ));
        }
        let size = self.git(&["cat-file", "-s", object])?;
        let size = String::from_utf8_lossy(&size)
            .trim()
            .parse::<usize>()
            .map_err(|e| SnapshotError::Invalid(e.to_string()))?;
        if size > max_bytes {
            return Err(SnapshotError::Limit {
                resource: "--max-input-bytes",
                limit: max_bytes,
            });
        }
        let source = self.command(&["cat-file", "blob", object], None, max_bytes)?;
        let source = String::from_utf8(source)
            .map_err(|_| SnapshotError::Invalid("reference source is not UTF-8".into()))?;
        if source.get(r.start..r.end).is_none() {
            return Err(SnapshotError::Invalid(
                "invalid reference byte range".into(),
            ));
        }
        Ok((r, source))
    }
}
fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| SnapshotError::Invalid("Git paths must be UTF-8".into()))
}
fn tree_entry(entry: &[u8]) -> Result<(&str, &str, &str)> {
    let entry = utf8(entry)?;
    let (meta, path) = entry
        .split_once('\t')
        .ok_or_else(|| SnapshotError::Invalid("invalid Git tree entry".into()))?;
    let mut parts = meta.split_whitespace();
    let mode = parts.next().unwrap_or_default();
    parts.next();
    let object = parts
        .next()
        .ok_or_else(|| SnapshotError::Invalid("invalid Git tree object".into()))?;
    Ok((mode, object, path))
}
fn valid_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}
#[derive(Debug, Clone)]
pub struct SourceReference {
    pub commit: String,
    pub object: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
}
impl SourceReference {
    pub fn encode(&self) -> String {
        let path = self
            .path
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        format!(
            "rc1:{}:{}:{}:{}:{path}",
            self.commit, self.object, self.start, self.end
        )
    }
    pub fn parse(text: &str) -> Result<Self> {
        let parts: Vec<_> = text.split(':').collect();
        if parts.len() != 6
            || parts[0] != "rc1"
            || !valid_oid(parts[1])
            || !valid_oid(parts[2])
            || parts[5].len() % 2 != 0
            || !parts[5].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(SnapshotError::Invalid(
                "invalid rc1 symbol reference; copy the complete reference from JSON".into(),
            ));
        }
        let path = (0..parts[5].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&parts[5][i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        let path = utf8(&path)?.to_owned();
        if path.is_empty()
            || Path::new(&path)
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(SnapshotError::Invalid(
                "reference path must be repository-relative".into(),
            ));
        }
        let number = |s: &str| {
            s.parse::<usize>()
                .map_err(|_| SnapshotError::Invalid("invalid reference offset".into()))
        };
        let r = Self {
            commit: parts[1].to_owned(),
            object: parts[2].to_owned(),
            path,
            start: number(parts[3])?,
            end: number(parts[4])?,
        };
        if r.start > r.end {
            return Err(SnapshotError::Invalid("reference range is reversed".into()));
        }
        Ok(r)
    }
}
