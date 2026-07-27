use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;

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
    pub num_ignored_directories: usize,
}

impl RepositoryInventory {
    pub fn recognized_source_files(&self) -> usize {
        self.files_by_language.values().sum()
    }

    pub fn unclassified_files(&self) -> usize {
        self.unclassified_files_by_extension.values().sum()
    }
}

pub fn inventory_repository(root: &Path) -> io::Result<RepositoryInventory> {
    let mut inventory = RepositoryInventory::default();

    visit_directory(root, &mut inventory)?;

    Ok(inventory)
}

fn visit_directory(directory: &Path, inventory: &mut RepositoryInventory) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();

        if file_type.is_dir() {
            if should_ignore_directory(entry.file_name().as_os_str()) {
                inventory.num_ignored_directories += 1;
                continue;
            }

            visit_directory(&path, inventory)?;
        } else if file_type.is_file() {
            inventory.total_files += 1;

            if let Some(language) = detect_language(&path) {
                *inventory.files_by_language.entry(language).or_insert(0) += 1;
            } else {
                let extension = extension_id(&path);

                *inventory
                    .unclassified_files_by_extension
                    .entry(extension)
                    .or_insert(0) += 1;
            }
        }
    }

    Ok(())
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
