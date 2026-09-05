//! Structural C++ flow, kept separate from symbol resolution and runtime claims.
use codegraide_core::{CallFlow, CallFlowKind as K};
use tree_sitter::Node;

pub(crate) fn extract(body: Node<'_>, source: &[u8]) -> CallFlow {
    Builder {
        source,
        remaining: 2000,
    }
    .walk(body, 0)
}

struct Builder<'a> {
    source: &'a [u8],
    remaining: usize,
}
impl Builder<'_> {
    fn text(&self, n: Node<'_>) -> String {
        let text = String::from_utf8_lossy(&self.source[n.byte_range()]);
        let mut excerpt: String = text.chars().take(300).collect();
        if text.chars().count() > 300 {
            excerpt.push('…');
        }
        excerpt
    }
    fn item(
        &self,
        n: Node<'_>,
        kind: K,
        text: impl Into<String>,
        children: Vec<CallFlow>,
    ) -> CallFlow {
        CallFlow {
            kind,
            text: text.into(),
            line: n.start_position().row + 1,
            column: n.start_position().column + 1,
            children,
        }
    }
    fn field(&mut self, n: Node<'_>, field: &str, depth: usize) -> Vec<CallFlow> {
        n.child_by_field_name(field)
            .map(|child| vec![self.walk(child, depth + 1)])
            .unwrap_or_default()
    }
    fn named(n: Node<'_>) -> Vec<Node<'_>> {
        n.named_children(&mut n.walk())
            .filter(|c| c.kind() != "comment")
            .collect()
    }
    fn group(&mut self, n: Node<'_>, kind: K, text: impl Into<String>, depth: usize) -> CallFlow {
        let mut children = Vec::new();
        for child in Self::named(n) {
            if self.remaining == 0 {
                children.push(self.item(child, K::Unsupported, "Structure limit reached", vec![]));
                break;
            }
            children.push(self.walk(child, depth + 1));
        }
        self.item(n, kind, text, children)
    }
    fn walk(&mut self, n: Node<'_>, depth: usize) -> CallFlow {
        if depth > 60 || self.remaining == 0 {
            return self.item(n, K::Unsupported, "Structure limit reached", vec![]);
        }
        self.remaining -= 1;
        if n.is_error() || n.is_missing() {
            return self.item(n, K::Unsupported, "Incomplete syntax", vec![]);
        }
        match n.kind() {
            "compound_statement" => self.group(n, K::Sequence, "Source sequence", depth),
            "if_statement" => {
                let mut children = self.field(n, "initializer", depth);
                if let Some(condition) = n.child_by_field_name("condition") {
                    let value = self.walk(condition, depth + 1);
                    children.push(self.item(
                        condition,
                        K::Condition,
                        self.text(condition),
                        vec![value],
                    ));
                }
                let mut branches = Vec::new();
                for (field, label) in [
                    ("consequence", "Then"),
                    ("alternative", "Else · alternative"),
                ] {
                    if let Some(branch) = n.child_by_field_name(field) {
                        let value = self.walk(branch, depth + 1);
                        let label = if field == "alternative"
                            && branch
                                .named_child(0)
                                .is_some_and(|child| child.kind() == "if_statement")
                        {
                            "Else if · alternative"
                        } else {
                            label
                        };
                        branches.push(self.item(branch, K::Branch, label, vec![value]));
                    }
                }
                if n.child_by_field_name("alternative").is_none() {
                    branches.push(self.item(n, K::Branch, "Otherwise · continue after if", vec![]));
                }
                children.push(self.item(n, K::Alternatives, "Choose a branch", branches));
                self.item(n, K::Sequence, "If", children)
            }
            "for_statement" | "for_range_loop" | "while_statement" | "do_statement" => {
                let mut before = Vec::new();
                if n.kind() == "for_statement" {
                    before.extend(self.field(n, "initializer", depth));
                }
                if n.kind() == "for_range_loop" {
                    before.extend(self.field(n, "right", depth));
                }
                let mut repeated = Vec::new();
                if n.kind() == "do_statement" {
                    repeated.extend(self.field(n, "body", depth));
                }
                if let Some(condition) = n.child_by_field_name("condition") {
                    let flow = self.walk(condition, depth + 1);
                    repeated.push(self.item(
                        condition,
                        K::Condition,
                        "Check before each iteration (after body for do-while)",
                        vec![flow],
                    ));
                }
                if n.kind() != "do_statement" {
                    repeated.extend(self.field(n, "body", depth));
                }
                repeated.extend(self.field(n, "update", depth));
                let label = if n.kind() == "for_range_loop" {
                    "For each item · repeats; zero iterations possible"
                } else if n.kind() == "do_statement" {
                    "Do-while · body runs before condition"
                } else {
                    "Loop · repeat while condition holds (or until an exit if no condition)"
                };
                let header = n
                    .child_by_field_name("body")
                    .map(|body| {
                        String::from_utf8_lossy(&self.source[n.start_byte()..body.start_byte()])
                            .trim()
                            .chars()
                            .take(200)
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                before.push(self.item(n, K::Loop, format!("{label} · {header}"), repeated));
                self.item(n, K::Sequence, "Loop setup", before)
            }
            "switch_statement" => {
                let mut children = self.field(n, "condition", depth);
                if let Some(body) = n.child_by_field_name("body") {
                    let cases = Self::named(body)
                        .into_iter()
                        .map(|c| self.walk(c, depth + 1))
                        .collect();
                    children.push(self.item(
                        body,
                        K::Alternatives,
                        "Switch alternatives · cases may fall through until break/return",
                        cases,
                    ));
                }
                self.item(n, K::Sequence, "Switch", children)
            }
            "case_statement" => self.group(
                n,
                K::Branch,
                n.child_by_field_name("value")
                    .map(|v| format!("Case {} · may fall through", self.text(v)))
                    .unwrap_or_else(|| "Default · alternative".into()),
                depth,
            ),
            "return_statement" | "throw_statement" | "throw_expression" | "co_return_statement" => {
                let children = Self::named(n)
                    .into_iter()
                    .map(|c| self.walk(c, depth + 1))
                    .collect();
                self.item(n, K::Exit, self.text(n), children)
            }
            "break_statement" => {
                self.item(n, K::Exit, "Break · leave enclosing loop/switch", vec![])
            }
            "continue_statement" => self.item(n, K::Exit, "Continue · next loop iteration", vec![]),
            "conditional_expression" => {
                let mut children = Vec::new();
                if let Some(condition) = n.child_by_field_name("condition") {
                    let value = self.walk(condition, depth + 1);
                    children.push(self.item(
                        condition,
                        K::Condition,
                        self.text(condition),
                        vec![value],
                    ));
                }
                let mut branches = Vec::new();
                for (field, label) in [
                    ("consequence", "If true"),
                    ("alternative", "If false · alternative"),
                ] {
                    if let Some(c) = n.child_by_field_name(field) {
                        let flow = self.walk(c, depth + 1);
                        branches.push(self.item(
                            c,
                            K::Branch,
                            format!("{label} · {}", self.text(c)),
                            vec![flow],
                        ));
                    }
                }
                children.push(self.item(n, K::Alternatives, "Conditional expression", branches));
                self.item(n, K::Sequence, "Test condition first", children)
            }
            "binary_expression" => {
                let op = n
                    .child_by_field_name("operator")
                    .map(|x| self.text(x))
                    .unwrap_or_default();
                if op == "&&" || op == "||" {
                    let mut children = self.field(n, "left", depth);
                    if let Some(right) = n.child_by_field_name("right") {
                        let flow = self.walk(right, depth + 1);
                        children.push(self.item(right,K::Branch,format!("Right operand only if left is {} for built-in {op}; overloaded operator may evaluate both",if op=="&&" {"true"} else {"false"}),vec![flow]));
                    }
                    self.item(
                        n,
                        K::Sequence,
                        "Short-circuit candidate · operator overload not resolved",
                        children,
                    )
                } else {
                    self.group(n, K::Unspecified, "Operand order not established", depth)
                }
            }
            "call_expression" => {
                let mut operands = Vec::new();
                if let Some(function) = n.child_by_field_name("function") {
                    operands.push(self.walk(function, depth + 1));
                }
                if let Some(arguments) = n.child_by_field_name("arguments") {
                    for arg in Self::named(arguments) {
                        operands.push(self.walk(arg, depth + 1));
                    }
                }
                let evaluation = self.item(
                    n,
                    K::Unspecified,
                    "Evaluate callee/arguments · relative order not established",
                    operands,
                );
                let call = self.item(n, K::Call, self.text(n), vec![]);
                self.item(n, K::Sequence, "Call", vec![evaluation, call])
            }
            "initializer_list" | "new_expression" | "compound_literal_expression" => self.group(
                n,
                K::Unsupported,
                "Construction/initialization · full sequencing not modeled",
                depth,
            ),
            "lambda_expression" => self.item(
                n,
                K::Unsupported,
                "Lambda creation · body is not executed here",
                vec![],
            ),
            "try_statement" | "goto_statement" | "co_await_expression" | "co_yield_expression" => {
                self.group(
                    n,
                    K::Unsupported,
                    "Control transfer not modeled · inspect source",
                    depth,
                )
            }
            kind if kind.starts_with("preproc") => self.item(
                n,
                K::Unsupported,
                "Preprocessor-dependent structure · inspect source",
                vec![],
            ),
            "expression_statement"
            | "declaration"
            | "init_declarator"
            | "else_clause"
            | "condition_clause"
            | "parenthesized_expression" => self.group(n, K::Sequence, self.text(n), depth),
            _ => {
                let children = Self::named(n);
                if children.is_empty() {
                    self.item(n, K::Statement, self.text(n), vec![])
                } else {
                    self.group(
                        n,
                        K::Unspecified,
                        "Expression structure · order not established",
                        depth,
                    )
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn flow(source: &str) -> CallFlow {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let function = tree.root_node().named_child(0).unwrap();
        extract(
            function.child_by_field_name("body").unwrap(),
            source.as_bytes(),
        )
    }
    fn flatten<'a>(f: &'a CallFlow, out: &mut Vec<&'a CallFlow>) {
        out.push(f);
        for c in &f.children {
            flatten(c, out)
        }
    }
    #[test]
    fn retains_branches_repeated_calls_loops_and_exits() {
        let f = flow(
            "bool f(){ for(auto x:items){if(stock_for(x)<2)return false;else use(x);} use(1);use(1);return true;}",
        );
        let mut all = Vec::new();
        flatten(&f, &mut all);
        assert_eq!(
            all.iter()
                .filter(|n| n.kind == K::Call && n.text == "use(1)")
                .count(),
            2
        );
        assert!(all.iter().any(|n| n.kind == K::Loop));
        assert!(
            all.iter()
                .any(|n| n.kind == K::Branch && n.text.starts_with("Else"))
        );
        assert!(
            all.iter()
                .any(|n| n.kind == K::Exit && n.text == "return false;")
        );
        assert_eq!(f.children.last().unwrap().kind, K::Exit);
    }
    #[test]
    fn nested_arguments_are_not_a_definite_sequence() {
        let f = flow(
            "void f(){sink(left(),right());if(test()&&next())one();auto x=test()?one():two();}",
        );
        let mut all = Vec::new();
        flatten(&f, &mut all);
        let arguments = all
            .iter()
            .find(|n| {
                n.kind == K::Unspecified
                    && n.children.iter().filter(|c| c.kind == K::Sequence).count() >= 2
            })
            .unwrap();
        assert!(arguments.text.contains("order not established"));
        assert!(all.iter().any(|n| n.text.contains("Right operand only")));
        assert!(
            all.iter()
                .any(|n| n.kind == K::Alternatives && n.text == "Conditional expression")
        );
    }
    #[test]
    fn condition_precedes_alternatives_and_exit_stays_inside_loop() {
        let f =
            flow("bool f(){for(auto item:items){if(stock_for(item)<2)return false;}return true;}");
        let mut all = Vec::new();
        flatten(&f, &mut all);
        let branch = all
            .iter()
            .find(|n| n.kind == K::Sequence && n.text == "If")
            .unwrap();
        assert_eq!(branch.children[0].kind, K::Condition);
        assert_eq!(branch.children[1].kind, K::Alternatives);
        let loop_node = all.iter().find(|n| n.kind == K::Loop).unwrap();
        let mut repeated = Vec::new();
        flatten(loop_node, &mut repeated);
        assert!(
            repeated
                .iter()
                .any(|n| n.kind == K::Call && n.text == "stock_for(item)")
        );
        assert!(
            repeated
                .iter()
                .any(|n| n.kind == K::Exit && n.text == "return false;")
        );
        assert!(!repeated.iter().any(|n| n.text == "return true;"));
        assert_eq!(f.children.last().unwrap().text, "return true;");
    }
    #[test]
    fn switches_throws_and_unsupported_transfers_are_explicit() {
        let f = flow(
            "void f(){if(a())one();else if(b())two();else three();switch(key()){case 1:one();break;default:throw fail();}goto end;end:one();}",
        );
        let mut all = Vec::new();
        flatten(&f, &mut all);
        assert!(
            all.iter()
                .any(|n| n.kind == K::Alternatives && n.text.contains("Switch alternatives"))
        );
        assert!(
            all.iter()
                .any(|n| n.kind == K::Exit && n.text.starts_with("throw"))
        );
        assert!(all.iter().any(|n| n.kind == K::Unsupported));
        assert!(
            all.iter()
                .filter(|n| n.kind == K::Alternatives && n.text == "Choose a branch")
                .count()
                >= 2
        );
    }
}
