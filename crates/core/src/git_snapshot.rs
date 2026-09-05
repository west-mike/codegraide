//! Read committed source without checking out revisions or consulting the worktree.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct SnapshotError(pub String);
impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SnapshotError {}
type Result<T> = std::result::Result<T, SnapshotError>;

pub struct GitRepository {
    root: PathBuf,
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
    /// All tracked entries, including files the C++ analyzer cannot read.
    pub entries: BTreeMap<String, (String, String)>,
}
impl GitRepository {
    pub fn open(path: &Path) -> Result<Self> {
        let result = Self {
            root: path.to_path_buf(),
        };
        result.git(&["rev-parse", "--git-dir"])?;
        Ok(result)
    }
    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        let output = Command::new("git")
            .arg("--no-replace-objects")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(|e| SnapshotError(format!("cannot run Git: {e}")))?;
        if !output.status.success() {
            return Err(SnapshotError(format!(
                "git {} failed: {}",
                args[0],
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output.stdout)
    }
    pub fn resolve(&self, revision: &str) -> Result<String> {
        if revision.starts_with('-') {
            return Err(SnapshotError("revision must not start with '-'".into()));
        }
        let bytes = self.git(&["rev-parse", "--verify", &format!("{revision}^{{commit}}")])?;
        let commit = String::from_utf8_lossy(&bytes).trim().to_owned();
        if !valid_oid(&commit) {
            return Err(SnapshotError("Git returned an invalid commit ID".into()));
        }
        Ok(commit)
    }
    pub fn snapshot(&self, commit: &str, max_bytes: usize) -> Result<GitSnapshot> {
        let commit = self.resolve(commit)?;
        let tree = self.git(&["ls-tree", "-rz", "--full-tree", &commit])?;
        let mut result = GitSnapshot {
            commit,
            files: BTreeMap::new(),
            excluded: Vec::new(),
            entries: BTreeMap::new(),
        };
        let mut total = 0usize;
        for entry in tree.split(|b| *b == 0).filter(|s| !s.is_empty()) {
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
            if crate::inventory::detect_language(Path::new(path))
                .is_none_or(|l| l.as_str() != "cpp")
            {
                result
                    .excluded
                    .push((path.into(), "unsupported-file".into()));
                continue;
            }
            let size = self.git(&["cat-file", "-s", object])?;
            let size = String::from_utf8_lossy(&size)
                .trim()
                .parse::<usize>()
                .map_err(|e| SnapshotError(e.to_string()))?;
            total = total
                .checked_add(size)
                .ok_or_else(|| SnapshotError("snapshot size overflow".into()))?;
            if total > max_bytes {
                return Err(SnapshotError(format!(
                    "snapshot exceeds --max-input-bytes {max_bytes}; increase the limit explicitly"
                )));
            }
            let bytes = self.git(&["cat-file", "blob", object])?;
            match String::from_utf8(bytes) {
                Ok(source) if !source.contains('\0') => {
                    result.files.insert(
                        path.into(),
                        SnapshotFile {
                            object: object.into(),
                            source,
                        },
                    );
                }
                _ => result
                    .excluded
                    .push((path.into(), "non-utf8-or-binary".into())),
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
                .ok_or_else(|| SnapshotError("invalid Git diff path".into()))?;
            if status.starts_with(b"R") || status.starts_with(b"C") {
                let second = fields
                    .next()
                    .ok_or_else(|| SnapshotError("invalid Git rename".into()))?;
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
            return Err(SnapshotError("reference commit must be a full ID".into()));
        }
        let spec = format!(":(literal){}", r.path);
        let tree = self.git(&["ls-tree", "-rz", "--full-tree", &r.commit, "--", &spec])?;
        let entry = tree
            .split(|b| *b == 0)
            .find(|s| !s.is_empty())
            .ok_or_else(|| SnapshotError("reference path is absent from snapshot".into()))?;
        let (mode, object, path) = tree_entry(entry)?;
        if !matches!(mode, "100644" | "100755") || path != r.path || object != r.object {
            return Err(SnapshotError(
                "reference does not match the snapshot blob".into(),
            ));
        }
        let size = self.git(&["cat-file", "-s", object])?;
        let size = String::from_utf8_lossy(&size)
            .trim()
            .parse::<usize>()
            .map_err(|e| SnapshotError(e.to_string()))?;
        if size > max_bytes {
            return Err(SnapshotError(format!(
                "source blob exceeds --max-input-bytes {max_bytes}"
            )));
        }
        let source = self.git(&["cat-file", "blob", object])?;
        let source = String::from_utf8(source)
            .map_err(|_| SnapshotError("reference source is not UTF-8".into()))?;
        if source.get(r.start..r.end).is_none() {
            return Err(SnapshotError("invalid reference byte range".into()));
        }
        Ok((r, source))
    }
}
fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| SnapshotError("Git paths must be UTF-8".into()))
}
fn tree_entry(entry: &[u8]) -> Result<(&str, &str, &str)> {
    let entry = utf8(entry)?;
    let (meta, path) = entry
        .split_once('\t')
        .ok_or_else(|| SnapshotError("invalid Git tree entry".into()))?;
    let mut parts = meta.split_whitespace();
    let mode = parts.next().unwrap_or_default();
    parts.next();
    let object = parts
        .next()
        .ok_or_else(|| SnapshotError("invalid Git tree object".into()))?;
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
            return Err(SnapshotError(
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
            return Err(SnapshotError(
                "reference path must be repository-relative".into(),
            ));
        }
        let number = |s: &str| {
            s.parse::<usize>()
                .map_err(|_| SnapshotError("invalid reference offset".into()))
        };
        let r = Self {
            commit: parts[1].to_owned(),
            object: parts[2].to_owned(),
            path,
            start: number(parts[3])?,
            end: number(parts[4])?,
        };
        if r.start > r.end {
            return Err(SnapshotError("reference range is reversed".into()));
        }
        Ok(r)
    }
}
