use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum FileCategory {
    Source,
    Documentation,
    Configuration,
    Data,
    Assets,
    Uncategorized,
}

impl FileCategory {
    pub const ALL: [Self; 6] = [
        Self::Source,
        Self::Documentation,
        Self::Configuration,
        Self::Data,
        Self::Assets,
        Self::Uncategorized,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Documentation => "documentation",
            Self::Configuration => "configuration",
            Self::Data => "data",
            Self::Assets => "assets",
            Self::Uncategorized => "uncategorized",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "source" => Some(Self::Source),
            "documentation" => Some(Self::Documentation),
            "configuration" => Some(Self::Configuration),
            "data" => Some(Self::Data),
            "assets" => Some(Self::Assets),
            "uncategorized" => Some(Self::Uncategorized),
            _ => None,
        }
    }
}

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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InventoryDiagnostic {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct IgnoredInventory {
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
    pub builtin_directories: Vec<PathBuf>,
    pub exact: bool,
}

impl IgnoredInventory {
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn directory_count(&self) -> usize {
        self.directories.len()
    }

    pub fn builtin_directory_count(&self) -> usize {
        self.builtin_directories.len()
    }
}

#[derive(Debug)]
pub struct RepositoryInventory {
    pub inventoried_files: usize,
    pub files_by_category: BTreeMap<FileCategory, Vec<PathBuf>>,
    pub files_by_language: BTreeMap<LanguageId, usize>,
    pub uncategorized_files_by_extension: BTreeMap<ExtensionId, usize>,
    pub ignored: IgnoredInventory,
    pub num_included_ignored_files: usize,
    pub diagnostics: Vec<InventoryDiagnostic>,
}

impl Default for RepositoryInventory {
    fn default() -> Self {
        let files_by_category = FileCategory::ALL
            .into_iter()
            .map(|category| (category, Vec::new()))
            .collect();

        Self {
            inventoried_files: 0,
            files_by_category,
            files_by_language: BTreeMap::new(),
            uncategorized_files_by_extension: BTreeMap::new(),
            ignored: IgnoredInventory::default(),
            num_included_ignored_files: 0,
            diagnostics: Vec::new(),
        }
    }
}

impl RepositoryInventory {
    pub fn category_files(&self, category: FileCategory) -> &[PathBuf] {
        self.files_by_category
            .get(&category)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn category_count(&self, category: FileCategory) -> usize {
        self.category_files(category).len()
    }

    pub fn source_files(&self) -> usize {
        self.category_count(FileCategory::Source)
    }

    pub fn uncategorized_files(&self) -> usize {
        self.category_count(FileCategory::Uncategorized)
    }

    pub(crate) fn record_category_path(&mut self, category: FileCategory, path: PathBuf) {
        self.files_by_category
            .get_mut(&category)
            .expect("all fixed categories are initialized")
            .push(path);
    }

    pub(crate) fn validate_invariants(&self) {
        debug_assert_eq!(
            self.inventoried_files,
            self.files_by_category.values().map(Vec::len).sum::<usize>()
        );
        debug_assert_eq!(
            self.uncategorized_files(),
            self.uncategorized_files_by_extension
                .values()
                .sum::<usize>()
        );
        for paths in self.files_by_category.values() {
            debug_assert!(paths.windows(2).all(|pair| pair[0] <= pair[1]));
            debug_assert!(paths.iter().all(|path| !path.is_absolute()));
        }
    }
}

pub(crate) fn normalize_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
