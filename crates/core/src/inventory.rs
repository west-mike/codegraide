use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ignore::overrides::OverrideBuilder;
use ignore::{Walk, WalkBuilder};

use crate::config::CategoryClassifier;
use crate::error::InventoryError;
use crate::report::{ExtensionId, FileCategory, LanguageId, RepositoryInventory};

#[derive(Debug)]
pub struct InventoryOptions {
    pub include_ignored: Vec<String>,
    pub config_path: Option<PathBuf>,
    pub audit_ignored: bool,
    pub emit_warnings: bool,
}

impl Default for InventoryOptions {
    fn default() -> Self {
        Self {
            include_ignored: Vec::new(),
            config_path: None,
            audit_ignored: false,
            emit_warnings: true,
        }
    }
}

#[derive(Debug, Default)]
struct WalkCollection {
    files: BTreeSet<PathBuf>,
    directories: BTreeSet<PathBuf>,
}

pub fn inventory_repository(root: &Path) -> Result<RepositoryInventory, InventoryError> {
    inventory_repository_with_options(root, &InventoryOptions::default())
}

pub fn inventory_repository_with_options(
    root: &Path,
    options: &InventoryOptions,
) -> Result<RepositoryInventory, InventoryError> {
    let (classifier, mut diagnostics) =
        CategoryClassifier::load(options.config_path.as_deref(), options.emit_warnings)?;

    let mut standard_walker = configured_walker(root);
    standard_walker
        .parents(false)
        .ignore(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .filter_entry(|entry| !is_builtin_ignored_directory(entry));

    let standard = collect_entries(standard_walker.build(), "repository discovery failed")?;
    let builtin_directories = Arc::new(Mutex::new(BTreeSet::new()));
    let ignored = if options.audit_ignored {
        collect_exact_ignored(root, &standard, Arc::clone(&builtin_directories))?
    } else {
        collect_ignored_boundaries(root, &standard, Arc::clone(&builtin_directories))?
    };

    let mut discovered_files = standard.files.clone();
    let mut num_included_ignored_files = 0;

    if !options.include_ignored.is_empty() {
        let overrides = build_include_overrides(root, &options.include_ignored)?;
        let mut included_walker = configured_walker(root);

        included_walker
            .standard_filters(false)
            .overrides(overrides)
            .filter_entry(|entry| !is_builtin_ignored_directory(entry));

        let included = collect_entries(
            included_walker.build(),
            "targeted ignored-file discovery failed",
        )?;

        for path in included.files {
            if discovered_files.insert(path) {
                num_included_ignored_files += 1;
            }
        }
    }

    let mut inventory = RepositoryInventory {
        ignored,
        num_included_ignored_files,
        diagnostics: Vec::new(),
        ..RepositoryInventory::default()
    };

    for path in discovered_files {
        let relative_path = path.strip_prefix(root).map_err(|error| {
            InventoryError::invalid_input(format!(
                "discovered path {} is outside repository {}: {error}",
                path.display(),
                root.display()
            ))
        })?;
        let category =
            classifier.classify(relative_path, &mut diagnostics, options.emit_warnings)?;
        record_file(relative_path, category, &mut inventory);
    }

    inventory.diagnostics = diagnostics;
    inventory.ignored.builtin_directories = take_paths_from_mutex(builtin_directories, root)?;
    inventory.validate_invariants();
    Ok(inventory)
}

fn collect_ignored_boundaries(
    root: &Path,
    standard: &WalkCollection,
    builtin_directories: Arc<Mutex<BTreeSet<PathBuf>>>,
) -> Result<crate::report::IgnoredInventory, InventoryError> {
    let standard_directories = Arc::new(standard.directories.clone());
    let ignored_directories = Arc::new(Mutex::new(BTreeSet::new()));
    let ignored_directories_for_filter = Arc::clone(&ignored_directories);
    let mut walker = configured_walker(root);

    walker.standard_filters(false).filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        if is_builtin_ignored_directory(entry) {
            insert_shared_path(&builtin_directories, entry.path());
            return false;
        }
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
            && !standard_directories.contains(entry.path())
        {
            insert_shared_path(&ignored_directories_for_filter, entry.path());
            return false;
        }
        true
    });

    let visible_boundaries = collect_entries(walker.build(), "ignored-boundary discovery failed")?;
    let ignored_files = visible_boundaries
        .files
        .difference(&standard.files)
        .cloned()
        .collect::<BTreeSet<_>>();

    Ok(crate::report::IgnoredInventory {
        files: relative_paths(ignored_files, root)?,
        directories: take_paths_from_mutex(ignored_directories, root)?,
        builtin_directories: Vec::new(),
        exact: false,
    })
}

fn collect_exact_ignored(
    root: &Path,
    standard: &WalkCollection,
    builtin_directories: Arc<Mutex<BTreeSet<PathBuf>>>,
) -> Result<crate::report::IgnoredInventory, InventoryError> {
    let mut walker = configured_walker(root);

    walker.standard_filters(false).filter_entry(move |entry| {
        if is_builtin_ignored_directory(entry) {
            insert_shared_path(&builtin_directories, entry.path());
            return false;
        }
        true
    });

    let all_entries = collect_entries(walker.build(), "ignored-file audit failed")?;
    let ignored_files = all_entries
        .files
        .difference(&standard.files)
        .cloned()
        .collect::<BTreeSet<_>>();
    let ignored_directories = all_entries
        .directories
        .difference(&standard.directories)
        .filter(|path| path.as_path() != root)
        .cloned()
        .collect::<BTreeSet<_>>();

    Ok(crate::report::IgnoredInventory {
        files: relative_paths(ignored_files, root)?,
        directories: relative_paths(ignored_directories, root)?,
        builtin_directories: Vec::new(),
        exact: true,
    })
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
) -> Result<ignore::overrides::Override, InventoryError> {
    let mut builder = OverrideBuilder::new(root);

    for pattern in patterns {
        if pattern.is_empty() || pattern.starts_with('!') {
            return Err(InventoryError::invalid_input(format!(
                "invalid include-ignored pattern {pattern:?}"
            )));
        }

        builder.add(pattern).map_err(|error| {
            InventoryError::invalid_input(format!(
                "invalid include-ignored pattern {pattern:?}: {error}"
            ))
        })?;
    }

    builder.build().map_err(|error| {
        InventoryError::invalid_input(format!("could not build include-ignored patterns: {error}"))
    })
}

fn collect_entries(walker: Walk, context: &'static str) -> Result<WalkCollection, InventoryError> {
    let mut collection = WalkCollection::default();

    for entry in walker {
        let entry =
            entry.map_err(|error| InventoryError::io(context, std::io::Error::other(error)))?;

        if let Some(error) = entry.error() {
            return Err(InventoryError::io(
                context,
                std::io::Error::other(error.to_string()),
            ));
        }

        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            collection.files.insert(entry.path().to_path_buf());
        } else if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
        {
            collection.directories.insert(entry.path().to_path_buf());
        }
    }

    Ok(collection)
}

fn is_builtin_ignored_directory(entry: &ignore::DirEntry) -> bool {
    entry.depth() > 0
        && entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
        && should_ignore_directory(entry.file_name())
}

fn insert_shared_path(paths: &Mutex<BTreeSet<PathBuf>>, path: &Path) {
    paths
        .lock()
        .expect("ignored-path collector mutex should not be poisoned")
        .insert(path.to_path_buf());
}

fn take_paths_from_mutex(
    paths: Arc<Mutex<BTreeSet<PathBuf>>>,
    root: &Path,
) -> Result<Vec<PathBuf>, InventoryError> {
    let paths = paths
        .lock()
        .expect("ignored-path collector mutex should not be poisoned");
    relative_paths(paths.clone(), root)
}

fn relative_paths(paths: BTreeSet<PathBuf>, root: &Path) -> Result<Vec<PathBuf>, InventoryError> {
    paths
        .into_iter()
        .map(|path| {
            path.strip_prefix(root)
                .map(Path::to_path_buf)
                .map_err(|error| {
                    InventoryError::invalid_input(format!(
                        "path {} is outside repository {}: {error}",
                        path.display(),
                        root.display()
                    ))
                })
        })
        .collect()
}

fn record_file(path: &Path, category: FileCategory, inventory: &mut RepositoryInventory) {
    inventory.inventoried_files += 1;
    inventory.record_category_path(category, path.to_path_buf());

    if let Some(language) = detect_language(path) {
        *inventory.files_by_language.entry(language).or_insert(0) += 1;
    }

    if category == FileCategory::Uncategorized {
        let extension = extension_id(path);
        *inventory
            .uncategorized_files_by_extension
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
    fn leaves_unknown_extensions_without_a_language() {
        assert_eq!(detect_language(Path::new("README.md")), None);
    }

    #[test]
    fn extracts_normalized_extensions() {
        assert_eq!(extension_id(Path::new("README.MD")).as_str(), "md");
        assert_eq!(extension_id(Path::new("LICENSE")).as_str(), "[none]");
    }

    #[cfg(unix)]
    #[test]
    fn handles_non_utf8_paths_without_panicking() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(vec![
            b'm', b'o', b'd', 0xFF, b'.', b'r', b's',
        ]));

        assert_eq!(
            detect_language(&path).map(|language| language.as_str().to_owned()),
            Some("rust".to_owned())
        );
        assert_eq!(extension_id(&path).as_str(), "rs");
    }

    #[test]
    fn identifies_builtin_ignored_directories() {
        for name in [".git", "target", "__pycache__"] {
            assert!(should_ignore_directory(OsStr::new(name)));
        }

        assert!(!should_ignore_directory(OsStr::new("src")));
    }
}
