use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::lines::RepositoryLineCounts;

pub const INVENTORY_REPORT_SCHEMA_VERSION: &str = "0.1.0";
pub const INVENTORY_REPORT_DEFINITION_VERSION: &str = "inventory-report-v1";
pub const PHYSICAL_LINE_DEFINITION_VERSION: &str = "physical-lines-v1";

#[cfg(unix)]
const JSON_PATH_ENCODING: &str = "utf8-with-percent-encoded-bytes";
#[cfg(not(unix))]
const JSON_PATH_ENCODING: &str = "normalized-utf8";

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

#[derive(Debug, Serialize)]
pub struct InventoryJsonReport {
    pub report_schema_version: &'static str,
    pub tool: JsonTool,
    pub analysis: JsonAnalysis,
    pub inventory: JsonInventory,
    pub diagnostics: Vec<JsonDiagnostic>,
}

#[derive(Debug, Serialize)]
pub struct JsonTool {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct JsonAnalysis {
    pub kind: &'static str,
    pub definition_version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct JsonInventory {
    pub path_encoding: &'static str,
    pub inventoried_files: usize,
    pub categories: BTreeMap<String, JsonCategory>,
    pub languages: BTreeMap<String, usize>,
    pub uncategorized_extensions: BTreeMap<String, usize>,
    pub ignored: JsonIgnored,
    pub included_ignored_files: usize,
    pub line_counts: JsonLineCounts,
}

#[derive(Debug, Serialize)]
pub struct JsonCategory {
    pub count: usize,
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonIgnored {
    pub exact: bool,
    pub files: Vec<String>,
    pub directories: Vec<String>,
    pub builtin_directories: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonLineCounts {
    pub definition_version: &'static str,
    pub total: JsonLineCountValues,
    pub by_language: BTreeMap<String, JsonLineCountValues>,
}

#[derive(Debug, Serialize)]
pub struct JsonLineCountValues {
    pub files: usize,
    pub total_lines: usize,
    pub source_lines: usize,
    pub comment_lines: usize,
    pub blank_lines: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonDiagnostic {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
}

impl InventoryJsonReport {
    pub fn from_inventory(inventory: &RepositoryInventory) -> Self {
        let categories = FileCategory::ALL
            .into_iter()
            .map(|category| {
                let files = inventory
                    .category_files(category)
                    .iter()
                    .map(|path| json_path(path))
                    .collect::<Vec<_>>();
                (
                    category.as_str().to_owned(),
                    JsonCategory {
                        count: files.len(),
                        files,
                    },
                )
            })
            .collect();

        let languages = inventory
            .files_by_language
            .iter()
            .map(|(language, count)| (language.as_str().to_owned(), *count))
            .collect();
        let uncategorized_extensions = inventory
            .uncategorized_files_by_extension
            .iter()
            .map(|(extension, count)| (extension.as_str().to_owned(), *count))
            .collect();

        Self {
            report_schema_version: INVENTORY_REPORT_SCHEMA_VERSION,
            tool: JsonTool {
                name: "codegraide",
                version: env!("CARGO_PKG_VERSION"),
            },
            analysis: JsonAnalysis {
                kind: "inventory",
                definition_version: INVENTORY_REPORT_DEFINITION_VERSION,
            },
            inventory: JsonInventory {
                path_encoding: JSON_PATH_ENCODING,
                inventoried_files: inventory.inventoried_files,
                categories,
                languages,
                uncategorized_extensions,
                ignored: JsonIgnored {
                    exact: inventory.ignored.exact,
                    files: inventory
                        .ignored
                        .files
                        .iter()
                        .map(|path| json_path(path))
                        .collect(),
                    directories: inventory
                        .ignored
                        .directories
                        .iter()
                        .map(|path| json_path(path))
                        .collect(),
                    builtin_directories: inventory
                        .ignored
                        .builtin_directories
                        .iter()
                        .map(|path| json_path(path))
                        .collect(),
                },
                included_ignored_files: inventory.num_included_ignored_files,
                line_counts: JsonLineCounts::from_counts(&inventory.line_counts),
            },
            diagnostics: inventory
                .diagnostics
                .iter()
                .map(|diagnostic| JsonDiagnostic {
                    severity: "warning",
                    code: diagnostic.code,
                    message: diagnostic.message.clone(),
                })
                .collect(),
        }
    }
}

impl JsonLineCounts {
    fn from_counts(counts: &RepositoryLineCounts) -> Self {
        Self {
            definition_version: PHYSICAL_LINE_DEFINITION_VERSION,
            total: JsonLineCountValues::from_counts(&counts.total),
            by_language: counts
                .by_language
                .iter()
                .map(|(language, values)| {
                    (
                        language.as_str().to_owned(),
                        JsonLineCountValues::from_counts(values),
                    )
                })
                .collect(),
        }
    }
}

impl JsonLineCountValues {
    fn from_counts(counts: &crate::lines::LineCounts) -> Self {
        Self {
            files: counts.files,
            total_lines: counts.total,
            source_lines: counts.source,
            comment_lines: counts.comment,
            blank_lines: counts.blank,
        }
    }
}

fn json_path(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        percent_encode_path(path.as_os_str().as_bytes())
    }

    #[cfg(not(unix))]
    {
        normalize_relative_path(path)
    }
}

#[cfg(unix)]
fn percent_encode_path(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'/' {
            output.push('/');
            index += 1;
            continue;
        }

        if bytes[index] == b'%' || bytes[index] == b'\\' {
            push_percent_encoded(&mut output, bytes[index]);
            index += 1;
            continue;
        }

        match std::str::from_utf8(&bytes[index..]) {
            Ok(valid) => {
                for character in valid.chars() {
                    if character == '%' || character == '\\' {
                        push_percent_encoded(&mut output, character as u8);
                    } else {
                        output.push(character);
                    }
                }
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    let valid = std::str::from_utf8(&bytes[index..index + valid_up_to])
                        .expect("UTF-8 prefix should be valid");
                    for character in valid.chars() {
                        if character == '%' || character == '\\' {
                            push_percent_encoded(&mut output, character as u8);
                        } else {
                            output.push(character);
                        }
                    }
                    index += valid_up_to;
                } else {
                    push_percent_encoded(&mut output, bytes[index]);
                    index += 1;
                }
            }
        }
    }

    output
}

#[cfg(unix)]
fn push_percent_encoded(output: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    output.push('%');
    output.push(HEX[(byte >> 4) as usize] as char);
    output.push(HEX[(byte & 0x0F) as usize] as char);
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    use super::json_path;

    #[test]
    fn json_paths_encode_non_utf8_and_percent_bytes_losslessly() {
        let path = PathBuf::from(OsString::from_vec(b"dir/mod%\\\xFF.rs".to_vec()));

        assert_eq!(json_path(&path), "dir/mod%25%5C%FF.rs");
    }
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
    pub line_counts: RepositoryLineCounts,
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
            line_counts: RepositoryLineCounts::default(),
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
        debug_assert_eq!(
            self.line_counts.total.source
                + self.line_counts.total.comment
                + self.line_counts.total.blank,
            self.line_counts.total.total
        );
        debug_assert_eq!(
            self.line_counts.total.files,
            self.line_counts
                .by_language
                .values()
                .map(|counts| counts.files)
                .sum::<usize>()
        );
        for counts in self.line_counts.by_language.values() {
            debug_assert_eq!(counts.source + counts.comment + counts.blank, counts.total);
        }
        for paths in self.files_by_category.values() {
            debug_assert!(paths.windows(2).all(|pair| pair[0] <= pair[1]));
            debug_assert!(paths.iter().all(|path| !path.is_absolute()));
        }
    }
}

pub(crate) fn normalize_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
