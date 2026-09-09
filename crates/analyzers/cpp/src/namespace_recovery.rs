//! Recover lexical namespace ownership only when every conditional branch agrees.
//! Tree-sitter can count mutually exclusive opening braces as nested blocks and
//! consequently close namespaces too early. No preprocessing branch is selected.
use std::collections::BTreeMap;
use tree_sitter::Node;

type Stack = Vec<Option<usize>>;
struct Conditional {
    entry: Stack,
    branches: Vec<Stack>,
    has_else: bool,
}

#[derive(Default)]
pub(crate) struct Recovery {
    pub owners: BTreeMap<usize, Option<usize>>,
    pub ends: BTreeMap<usize, usize>,
}

pub(crate) fn owners(root: Node<'_>, source: &[u8]) -> Option<Recovery> {
    fn namespaces(node: Node<'_>, opens: &mut BTreeMap<usize, usize>) {
        if node.kind() == "namespace_definition" {
            if let Some(body) = node.child_by_field_name("body") {
                opens.insert(body.start_byte(), node.start_byte());
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            namespaces(child, opens);
        }
    }
    let mut opens = BTreeMap::new();
    namespaces(root, &mut opens);
    let events = tokens(source)?;
    let mut stack = Stack::new();
    let mut conditionals = Vec::<Conditional>::new();
    let mut result = BTreeMap::new();
    let mut ends = BTreeMap::new();
    for (byte, token) in events {
        match token.as_str() {
            "{" => stack.push(opens.get(&byte).copied()),
            "}" => {
                if let Some(namespace) = stack.pop()? {
                    if ends.insert(namespace, byte + 1).is_some() {
                        return None;
                    }
                }
            }
            "#if" | "#ifdef" | "#ifndef" => conditionals.push(Conditional {
                entry: stack.clone(),
                branches: Vec::new(),
                has_else: false,
            }),
            "#elif" | "#else" => {
                let conditional = conditionals.last_mut()?;
                if conditional.has_else {
                    return None;
                }
                conditional.branches.push(stack.clone());
                stack.clone_from(&conditional.entry);
                conditional.has_else = token == "#else";
            }
            "#endif" => {
                let mut conditional = conditionals.pop()?;
                conditional.branches.push(stack.clone());
                if !conditional.has_else {
                    conditional.branches.push(conditional.entry);
                }
                if conditional.branches.iter().any(|branch| branch != &stack) {
                    return None;
                }
            }
            _ => unreachable!(),
        }
        result.insert(
            byte + token.len(),
            stack.iter().rev().find_map(|owner| *owner),
        );
    }
    (stack.is_empty() && conditionals.is_empty()).then_some(Recovery {
        owners: result,
        ends,
    })
}

// Scan written punctuation, including tokens swallowed by parser recovery.
// Directives are structural only; their conditions are never evaluated.
fn tokens(source: &[u8]) -> Option<Vec<(usize, String)>> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < source.len() {
        let rest = &source[i..];
        if rest.starts_with(b"//") {
            i = logical_line_end(source, i);
        } else if rest.starts_with(b"/*") {
            let end = rest.windows(2).position(|s| s == b"*/")? + 2;
            line_start |= rest[..end].contains(&b'\n');
            i += end;
        } else if rest.starts_with(b"R\"") {
            let open = rest.iter().position(|b| *b == b'(')?;
            if open > 18 {
                return None;
            }
            let mut end = vec![b')'];
            end.extend_from_slice(&rest[2..open]);
            end.push(b'"');
            i += open + 1 + rest[open + 1..].windows(end.len()).position(|s| s == end)? + end.len();
            line_start = false;
        } else if matches!(source[i], b'"' | b'\'') {
            // A digit separator is not a character literal.
            if source[i] == b'\''
                && i > 0
                && source.get(i + 1).is_some_and(u8::is_ascii_alphanumeric)
                && source[source[..i]
                    .iter()
                    .rposition(|b| !b.is_ascii_alphanumeric() && *b != b'_')
                    .map_or(0, |p| p + 1)]
                .is_ascii_digit()
            {
                i += 1;
                continue;
            }
            let quote = source[i];
            i += 1;
            loop {
                let b = *source.get(i)?;
                i += 1;
                if b == b'\\' {
                    i += 1;
                } else if b == quote {
                    break;
                }
            }
            line_start = false;
        } else if source[i] == b'#' && line_start {
            let start = i;
            i += 1;
            while source.get(i).is_some_and(|b| matches!(b, b' ' | b'\t')) {
                i += 1;
            }
            let word = i;
            while source.get(i).is_some_and(u8::is_ascii_alphabetic) {
                i += 1;
            }
            let directive = std::str::from_utf8(&source[word..i]).ok()?;
            if matches!(
                directive,
                "if" | "ifdef" | "ifndef" | "elif" | "else" | "endif"
            ) {
                result.push((start, format!("#{directive}")));
            }
            i = logical_line_end(source, i);
        } else {
            match source[i] {
                b'{' | b'}' => {
                    result.push((i, (source[i] as char).to_string()));
                    line_start = false;
                }
                b'\n' => line_start = true,
                b' ' | b'\t' | b'\r' => {}
                _ => line_start = false,
            }
            i += 1;
        }
    }
    Some(result)
}

fn logical_line_end(source: &[u8], mut i: usize) -> usize {
    while i < source.len() {
        if source[i] == b'\n' {
            let end = if i > 0 && source[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            if end == 0 || source[end - 1] != b'\\' {
                break;
            }
        }
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recover(source: &[u8]) -> Option<Recovery> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        owners(tree.root_node(), source)
    }
    #[test]
    fn divergent_conditional_scopes_and_unbalanced_input_are_not_recovered() {
        for source in [
            "#if A\nnamespace A {\n#else\nnamespace B {\n#endif\nvoid f() {}\n}\n",
            "namespace A {\n#if X\n{\n#endif\nvoid f() {}\n}\n",
            "namespace A { void f() {}",
        ] {
            assert!(recover(source.as_bytes()).is_none(), "{source}");
        }
    }
    #[test]
    fn strings_comments_and_macro_bodies_do_not_change_written_scopes() {
        let source = br##"
#define OPEN { \
  {
namespace A {
const char* text = R"tag( } #endif { )tag";
const char* quoted = "}\"{";
char brace = '}';
char32_t letter = U'a';
int separated = 0xA'B;
// continued comment \
} #endif {
/* } #else */
// }
void f() {}
}
void outside() {}
"##;
        let recovery = recover(source).unwrap();
        let f = source.windows(8).position(|s| s == b"void f()").unwrap();
        assert!(recovery.owners.range(..=f).next_back().unwrap().1.is_some());
        let outside = source
            .windows(12)
            .position(|s| s == b"void outside")
            .unwrap();
        assert!(
            recovery
                .owners
                .range(..=outside)
                .next_back()
                .unwrap()
                .1
                .is_none()
        );
    }
}
