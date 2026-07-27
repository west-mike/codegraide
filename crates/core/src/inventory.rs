use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ignore::overrides::OverrideBuilder;
use ignore::{Walk, WalkBuilder};

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct LanguageId(String);

impl LanguageId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExtensionId(String);

impl ExtensionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Default)]
pub struct RepositoryInventory {
    pub total_files: usize,
    pub files_by_language: BTreeMap<LanguageId, usize>,
    pub unclassified_files_by_extension: BTreeMap<ExtensionId, usize>,
    pub num_builtin_ignored_directories: usize,
    pub num_included_ignored_files: usize,
}

impl RepositoryInventory {
    pub fn recognized_source_files(&self) -> usize {
        self.files_by_language.values().sum()
    }

    pub fn unclassified_files(&self) -> usize {
        self.unclassified_files_by_extension.values().sum()
    }
}

#[derive(Debug, Default)]
pub struct InventoryOptions {
    pub include_ignored: Vec<String>,
}

pub fn inventory_repository(root: &Path) -> io::Result<RepositoryInventory> {
    inventory_repository_with_options(root, &InventoryOptions::default())
}

pub fn inventory_repository_with_options(
    root: &Path,
    options: &InventoryOptions,
) -> io::Result<RepositoryInventory> {
    let ignored_directories = Arc::new(AtomicUsize::new(0));
    let ignored_directories_for_filter = Arc::clone(&ignored_directories);
    let mut standard_walker = configured_walker(root);

    standard_walker
        .parents(false)
        .ignore(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .filter_entry(move |entry| {
            let is_ignored = is_builtin_ignored_directory(entry);

            if is_ignored {
                ignored_directories_for_filter.fetch_add(1, Ordering::Relaxed);
            }

            !is_ignored
        });

    let mut discovered_files = BTreeSet::new();
    collect_regular_files(standard_walker.build(), &mut discovered_files)?;

    let mut inventory = RepositoryInventory::default();

    if !options.include_ignored.is_empty() {
        let overrides = build_include_overrides(root, &options.include_ignored)?;
        let mut included_walker = configured_walker(root);

        included_walker
            .standard_filters(false)
            .overrides(overrides)
            .filter_entry(|entry| !is_builtin_ignored_directory(entry));

        let mut included_files = BTreeSet::new();
        collect_regular_files(included_walker.build(), &mut included_files)?;

        for path in included_files {
            if discovered_files.insert(path) {
                inventory.num_included_ignored_files += 1;
            }
        }
    }

    for path in discovered_files {
        record_file(&path, &mut inventory);
    }

    inventory.num_builtin_ignored_directories = ignored_directories.load(Ordering::Relaxed);
    Ok(inventory)
}

fn configured_walker(root: &Path) -> WalkBuilder {
    let mut walker = WalkBuilder::new(root);

    walker
        .hidden(false)
        .follow_links(false)
        .sort_by_file_path(|left, right| left.cmp(right));

    walker
}

fn build_include_overrides(
    root: &Path,
    patterns: &[String],
) -> io::Result<ignore::overrides::Override> {
    let mut builder = OverrideBuilder::new(root);

    for pattern in patterns {
        if pattern.is_empty() || pattern.starts_with('!') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid include-ignored pattern {pattern:?}"),
            ));
        }

        builder.add(pattern).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid include-ignored pattern {pattern:?}: {error}"),
            )
        })?;
    }

    builder.build().map_err(io::Error::other)
}

fn collect_regular_files(walker: Walk, files: &mut BTreeSet<PathBuf>) -> io::Result<()> {
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;

        if let Some(error) = entry.error() {
            return Err(io::Error::other(error.to_string()));
        }

        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            files.insert(entry.path().to_path_buf());
        }
    }

    Ok(())
}

fn is_builtin_ignored_directory(entry: &ignore::DirEntry) -> bool {
    entry.depth() > 0
        && entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
        && should_ignore_directory(entry.file_name())
}

fn record_file(path: &Path, inventory: &mut RepositoryInventory) {
    inventory.total_files += 1;

    if let Some(language) = detect_language(path) {
        *inventory.files_by_language.entry(language).or_insert(0) += 1;
    } else {
        let extension = extension_id(path);

        *inventory
            .unclassified_files_by_extension
            .entry(extension)
            .or_insert(0) += 1;
    }
}

fn should_ignore_directory(name: &OsStr) -> bool {
    matches!(name.to_str(), Some(".git" | "target" | "__pycache__"))
}

fn detect_language(path: &Path) -> Option<LanguageId> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();

    let language = match extension.as_str() {
        "py" => "python",
        "rs" => "rust",
        "c" => "c",
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => "cpp",
        _ => return None,
    };

    Some(LanguageId::new(language))
}

fn extension_id(path: &Path) -> ExtensionId {
    let extension = match path.extension() {
        Some(extension) => match extension.to_str() {
            Some(extension) => extension.to_ascii_lowercase(),
            None => "[non-utf8]".to_owned(),
        },
        None => "[none]".to_owned(),
    };

    ExtensionId::new(extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_languages_case_insensitively() {
        let cases = [
            ("script.py", "python"),
            ("library.RS", "rust"),
            ("native.c", "c"),
            ("header.HPP", "cpp"),
        ];

        for (path, expected) in cases {
            let language = detect_language(Path::new(path)).expect("language should be recognized");

            assert_eq!(language.as_str(), expected);
        }
    }

    #[test]
    fn leaves_unknown_extensions_unclassified() {
        assert_eq!(detect_language(Path::new("README.md")), None);
    }

    #[test]
    fn extracts_normalized_extensions() {
        assert_eq!(extension_id(Path::new("README.MD")).as_str(), "md");
        assert_eq!(extension_id(Path::new("LICENSE")).as_str(), "[none]");
    }

    #[test]
    fn identifies_ignored_directories() {
        for name in [".git", "target", "__pycache__"] {
            assert!(should_ignore_directory(OsStr::new(name)));
        }

        assert!(!should_ignore_directory(OsStr::new("src")));
    }
}
