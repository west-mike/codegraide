use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::error::InventoryError;
use crate::inventory::detect_language;
use crate::report::{FileCategory, LanguageId, RepositoryInventory};

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct LineCounts {
    pub files: usize,
    pub total: usize,
    pub source: usize,
    pub comment: usize,
    pub blank: usize,
}

impl LineCounts {
    fn add(&mut self, other: Self) {
        self.files += other.files;
        self.total += other.total;
        self.source += other.source;
        self.comment += other.comment;
        self.blank += other.blank;
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct RepositoryLineCounts {
    pub total: LineCounts,
    pub by_language: BTreeMap<LanguageId, LineCounts>,
}

impl RepositoryLineCounts {
    pub fn analyze(root: &Path, inventory: &RepositoryInventory) -> Result<Self, InventoryError> {
        let mut counts = Self::default();

        for category in FileCategory::ALL {
            for path in inventory.category_files(category) {
                let Some(language) = detect_language(path) else {
                    continue;
                };

                let bytes = fs::read(root.join(path)).map_err(|error| {
                    InventoryError::io(
                        format!("could not read source file {}", path.display()),
                        error,
                    )
                })?;
                let file_counts = count_file(&bytes, language.as_str());

                counts.total.add(file_counts);
                counts
                    .by_language
                    .entry(language)
                    .or_default()
                    .add(file_counts);
            }
        }

        Ok(counts)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LineKind {
    Source,
    Comment,
    Blank,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PythonString {
    Single(u8),
    Triple(u8),
}

#[derive(Debug, Default)]
struct PythonState {
    string: Option<PythonString>,
    escaped: bool,
}

#[derive(Debug, Clone)]
enum CLikeString {
    Quoted(u8),
    Raw(Vec<u8>),
}

#[derive(Debug, Default)]
struct CLikeState {
    block_comment_depth: usize,
    string: Option<CLikeString>,
    escaped: bool,
}

fn count_file(bytes: &[u8], language: &str) -> LineCounts {
    let mut counts = LineCounts {
        files: 1,
        ..LineCounts::default()
    };
    let mut state = match language {
        "python" => LineState::Python(PythonState::default()),
        _ => LineState::CLike(CLikeState::default()),
    };

    for line in split_physical_lines(bytes) {
        let kind = state.classify(line);
        counts.total += 1;
        match kind {
            LineKind::Source => counts.source += 1,
            LineKind::Comment => counts.comment += 1,
            LineKind::Blank => counts.blank += 1,
        }
    }

    counts
}

fn split_physical_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split_inclusive(|byte| *byte == b'\n')
}

enum LineState {
    Python(PythonState),
    CLike(CLikeState),
}

impl LineState {
    fn classify(&mut self, line: &[u8]) -> LineKind {
        match self {
            Self::Python(state) => classify_python_line(line, state),
            Self::CLike(state) => classify_c_like_line(line, state),
        }
    }
}

fn classify_python_line(line: &[u8], state: &mut PythonState) -> LineKind {
    let mut source = state.string.is_some();
    let mut comment = false;
    let mut index = 0;

    while index < line.len() {
        if let Some(string) = state.string {
            source = true;
            match string {
                PythonString::Triple(quote) => {
                    if line[index..].starts_with(&[quote, quote, quote]) && !state.escaped {
                        state.string = None;
                        state.escaped = false;
                        index += 3;
                    } else {
                        state.escaped = line[index] == b'\\' && !state.escaped;
                        if line[index] != b'\\' {
                            state.escaped = false;
                        }
                        index += 1;
                    }
                }
                PythonString::Single(quote) => {
                    if line[index] == quote && !state.escaped {
                        state.string = None;
                    }
                    state.escaped = line[index] == b'\\' && !state.escaped;
                    if line[index] != b'\\' {
                        state.escaped = false;
                    }
                    index += 1;
                }
            }
            continue;
        }

        if line[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if line[index] == b'#' {
            comment = true;
            break;
        }

        if line[index] == b'\'' || line[index] == b'"' {
            let quote = line[index];
            if line[index..].starts_with(&[quote, quote, quote]) {
                state.string = Some(PythonString::Triple(quote));
                source = true;
                index += 3;
            } else {
                state.string = Some(PythonString::Single(quote));
                source = true;
                index += 1;
            }
            continue;
        }

        source = true;
        index += 1;
    }

    classify_line(source, comment)
}

fn classify_c_like_line(line: &[u8], state: &mut CLikeState) -> LineKind {
    let mut source = state.string.is_some();
    let mut comment = state.block_comment_depth > 0;
    let mut index = 0;

    while index < line.len() {
        if let Some(string) = state.string.clone() {
            source = true;
            match string {
                CLikeString::Quoted(quote) => {
                    if line[index] == quote && !state.escaped {
                        state.string = None;
                    }
                    state.escaped = line[index] == b'\\' && !state.escaped;
                    if line[index] != b'\\' {
                        state.escaped = false;
                    }
                    index += 1;
                }
                CLikeString::Raw(terminator) => {
                    if line[index..].starts_with(&terminator) {
                        state.string = None;
                        index += terminator.len();
                    } else {
                        index += 1;
                    }
                }
            }
            continue;
        }

        if state.block_comment_depth > 0 {
            comment = true;
            if line[index..].starts_with(b"/*") {
                state.block_comment_depth += 1;
                index += 2;
            } else if line[index..].starts_with(b"*/") {
                state.block_comment_depth -= 1;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if line[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if line[index..].starts_with(b"//") {
            comment = true;
            break;
        }

        if line[index..].starts_with(b"/*") {
            comment = true;
            state.block_comment_depth = 1;
            index += 2;
            continue;
        }

        if let Some((raw, consumed)) = raw_string_start(&line[index..]) {
            source = true;
            state.string = Some(CLikeString::Raw(raw));
            index += consumed;
            continue;
        }

        if line[index] == b'"' || line[index] == b'\'' {
            source = true;
            state.string = Some(CLikeString::Quoted(line[index]));
            state.escaped = false;
            index += 1;
            continue;
        }

        source = true;
        index += 1;
    }

    classify_line(source, comment)
}

fn raw_string_start(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    if bytes.first() == Some(&b'r') {
        let quote_index = bytes.iter().position(|byte| *byte == b'"')?;
        let prefix = &bytes[..quote_index];
        if prefix.iter().all(|byte| *byte == b'r' || *byte == b'#') {
            let hashes = &prefix[1..];
            let mut terminator = Vec::with_capacity(hashes.len() + 1);
            terminator.push(b'"');
            terminator.extend_from_slice(hashes);
            return Some((terminator, quote_index + 1));
        }
    }

    if bytes.starts_with(b"R\"") {
        let open_paren = bytes[2..].iter().position(|byte| *byte == b'(')? + 2;
        let delimiter = &bytes[2..open_paren];
        if delimiter.len() > 16
            || !delimiter
                .iter()
                .all(|byte| !byte.is_ascii_whitespace() && *byte != b'(' && *byte != b')')
        {
            return None;
        }

        let mut terminator = Vec::with_capacity(delimiter.len() + 2);
        terminator.push(b')');
        terminator.extend_from_slice(delimiter);
        terminator.push(b'"');
        return Some((terminator, open_paren + 1));
    }

    None
}

fn classify_line(source: bool, comment: bool) -> LineKind {
    match (source, comment) {
        (true, _) => LineKind::Source,
        (false, true) => LineKind::Comment,
        (false, false) => LineKind::Blank,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(source: &str, language: &str) -> LineCounts {
        count_file(source.as_bytes(), language)
    }

    #[test]
    fn counts_python_code_comments_and_blank_lines() {
        let counts = count("# comment\n\nvalue = 1 # trailing\n", "python");

        assert_eq!(counts.total, 3);
        assert_eq!(counts.source, 1);
        assert_eq!(counts.comment, 1);
        assert_eq!(counts.blank, 1);
    }

    #[test]
    fn treats_python_strings_and_docstrings_as_source() {
        let counts = count("\"\"\"doc\n# text\n\"\"\"\n", "python");

        assert_eq!(counts.total, 3);
        assert_eq!(counts.source, 3);
        assert_eq!(counts.comment, 0);
        assert_eq!(counts.blank, 0);
    }

    #[test]
    fn counts_c_like_block_comments_and_mixed_lines() {
        let counts = count("/* start\n\nend */\nint x = 1; // note\n", "cpp");

        assert_eq!(counts.total, 4);
        assert_eq!(counts.source, 1);
        assert_eq!(counts.comment, 3);
        assert_eq!(counts.blank, 0);
    }

    #[test]
    fn ignores_comment_markers_inside_strings() {
        let counts = count("const char* x = \"// not a comment\";\n", "rust");

        assert_eq!(counts.total, 1);
        assert_eq!(counts.source, 1);
        assert_eq!(counts.comment, 0);
        assert_eq!(counts.blank, 0);
    }

    #[test]
    fn handles_rust_raw_strings() {
        let counts = count("let x = r#\"// text\n/* text */\"#;\n", "rust");

        assert_eq!(counts.total, 2);
        assert_eq!(counts.source, 2);
        assert_eq!(counts.comment, 0);
    }

    #[test]
    fn handles_cpp_raw_strings_with_delimiters() {
        let counts = count("auto x = R\"tag(// text\n/* text */)tag\";\n", "cpp");

        assert_eq!(counts.total, 2);
        assert_eq!(counts.source, 2);
        assert_eq!(counts.comment, 0);
    }

    #[test]
    fn an_empty_file_has_no_physical_lines() {
        assert_eq!(count("", "python").total, 0);
    }
}
