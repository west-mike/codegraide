use std::collections::BTreeSet;

use codegraide_core::{
    ExplicitExportName, ExplicitExportStatus, ExplicitExports, SourcePosition, SourceSpan,
};
use tree_sitter::Node;

pub(crate) fn extract(root: Node<'_>, source: &[u8]) -> ExplicitExports {
    let mut state = ExportState::default();
    let mut cursor = root.walk();

    for statement in root.named_children(&mut cursor) {
        let expression = statement_expression(statement).unwrap_or(statement);
        if state.apply_direct(expression, source) {
            continue;
        }
        if let Some(span) = nested_export_operation(statement, source) {
            state.note_uncertain_operation(
                span,
                "conditional or nested module-level __all__ updates are not evaluated",
            );
        }
    }

    state.finish(root.has_error())
}

#[derive(Default)]
struct ExportState {
    seen: bool,
    names: Vec<ExplicitExportName>,
    declaration_span: Option<SourceSpan>,
    has_static_evidence: bool,
    complete: bool,
    reasons: BTreeSet<String>,
}

impl ExportState {
    fn apply_direct(&mut self, node: Node<'_>, source: &[u8]) -> bool {
        match node.kind() {
            "assignment" if assignment_sets_exports(node, source) => {
                self.apply_assignment(node, source);
                true
            }
            "assignment" if assignment_mutates_exports(node, source) => {
                self.note_uncertain_operation(
                    source_span(node),
                    "subscripted __all__ assignment is not statically evaluated",
                );
                true
            }
            "augmented_assignment" if assignment_sets_exports(node, source) => {
                self.apply_augmented_assignment(node, source);
                true
            }
            "augmented_assignment" if assignment_mutates_exports(node, source) => {
                self.note_uncertain_operation(
                    source_span(node),
                    "subscripted __all__ augmented assignment is not statically evaluated",
                );
                true
            }
            "call" if export_mutation_method(node, source).is_some() => {
                self.apply_call(node, source);
                true
            }
            "delete_statement" if delete_targets_exports(node, source) => {
                self.note_uncertain_operation(
                    source_span(node),
                    "deleting __all__ is not statically evaluated",
                );
                true
            }
            _ => false,
        }
    }

    fn apply_assignment(&mut self, node: Node<'_>, source: &[u8]) {
        self.seen = true;
        self.declaration_span = Some(source_span(node));
        self.names.clear();
        self.reasons.clear();
        self.has_static_evidence = false;
        self.complete = false;

        let Some(value) = node.child_by_field_name("right") else {
            self.reasons
                .insert("the __all__ assignment has no value".to_owned());
            return;
        };
        let evaluated = evaluate_collection(value, source);
        self.names = evaluated.names;
        self.has_static_evidence = evaluated.recognized;
        self.complete = evaluated.recognized && evaluated.complete;
        if !self.complete {
            self.reasons.insert(
                "the effective __all__ value is not a fully static list or tuple of strings"
                    .to_owned(),
            );
        }
    }

    fn apply_augmented_assignment(&mut self, node: Node<'_>, source: &[u8]) {
        self.begin_mutation(node);
        let operator = node
            .child_by_field_name("operator")
            .map(|operator| node_text(operator, source));
        if operator.as_deref() != Some("+=") {
            self.complete = false;
            self.reasons
                .insert("only += is supported for direct __all__ augmented assignments".to_owned());
            return;
        }
        let Some(value) = node.child_by_field_name("right") else {
            self.complete = false;
            self.reasons
                .insert("the __all__ += update has no value".to_owned());
            return;
        };
        self.extend_with(evaluate_collection(value, source));
    }

    fn apply_call(&mut self, node: Node<'_>, source: &[u8]) {
        self.begin_mutation(node);
        let Some(method) = export_mutation_method(node, source) else {
            return;
        };
        let arguments = node.child_by_field_name("arguments");
        let values = arguments
            .map(|arguments| {
                let mut cursor = arguments.walk();
                arguments.named_children(&mut cursor).collect::<Vec<_>>()
            })
            .unwrap_or_default();

        match method.as_str() {
            "append" if values.len() == 1 => {
                if let Some(name) = evaluate_string(values[0], source) {
                    self.names.push(name);
                    self.has_static_evidence = true;
                } else {
                    self.complete = false;
                    self.reasons
                        .insert("__all__.append requires one plain static string in v1".to_owned());
                }
            }
            "extend" if values.len() == 1 => {
                self.extend_with(evaluate_collection(values[0], source));
            }
            "append" | "extend" => {
                self.complete = false;
                self.reasons.insert(format!(
                    "__all__.{method} requires exactly one supported argument in v1"
                ));
            }
            _ => {
                self.complete = false;
                self.reasons.insert(format!(
                    "direct __all__.{method} mutation is not supported in v1"
                ));
            }
        }
    }

    fn begin_mutation(&mut self, node: Node<'_>) {
        if !self.seen {
            self.seen = true;
            self.complete = false;
            self.reasons
                .insert("__all__ is mutated before a static module-level assignment".to_owned());
        }
        self.declaration_span
            .get_or_insert_with(|| source_span(node));
    }

    fn extend_with(&mut self, evaluated: EvaluatedCollection) {
        self.names.extend(evaluated.names);
        self.has_static_evidence |= evaluated.recognized;
        if !evaluated.recognized || !evaluated.complete {
            self.complete = false;
            self.reasons.insert(
                "the __all__ update is not a fully static list or tuple of strings".to_owned(),
            );
        }
    }

    fn note_uncertain_operation(&mut self, span: SourceSpan, reason: &str) {
        self.seen = true;
        self.complete = false;
        self.declaration_span.get_or_insert(span);
        self.reasons.insert(reason.to_owned());
    }

    fn finish(mut self, parser_recovery: bool) -> ExplicitExports {
        if parser_recovery {
            self.complete = false;
            self.reasons
                .insert("parser recovery may hide or alter an __all__ declaration".to_owned());
        }

        if !self.seen && !parser_recovery {
            return ExplicitExports {
                status: ExplicitExportStatus::NotDeclared,
                names: Vec::new(),
                declaration_span: None,
                reason: None,
            };
        }

        let status = if self.complete {
            ExplicitExportStatus::Complete
        } else if self.has_static_evidence || !self.names.is_empty() {
            ExplicitExportStatus::Partial
        } else {
            ExplicitExportStatus::Unavailable
        };
        ExplicitExports {
            status,
            names: self.names,
            declaration_span: self.declaration_span,
            reason: (!self.reasons.is_empty())
                .then(|| self.reasons.into_iter().collect::<Vec<_>>().join("; ")),
        }
    }
}

struct EvaluatedCollection {
    names: Vec<ExplicitExportName>,
    recognized: bool,
    complete: bool,
}

impl EvaluatedCollection {
    fn unavailable() -> Self {
        Self {
            names: Vec::new(),
            recognized: false,
            complete: false,
        }
    }
}

fn evaluate_collection(node: Node<'_>, source: &[u8]) -> EvaluatedCollection {
    match node.kind() {
        "list" | "tuple" | "expression_list" => {
            let mut names = Vec::new();
            let mut complete = true;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(name) = evaluate_string(child, source) {
                    names.push(name);
                } else {
                    complete = false;
                }
            }
            EvaluatedCollection {
                names,
                recognized: true,
                complete,
            }
        }
        "parenthesized_expression" => single_named_child(node)
            .map(|child| evaluate_collection(child, source))
            .unwrap_or_else(EvaluatedCollection::unavailable),
        "binary_operator"
            if node
                .child_by_field_name("operator")
                .is_some_and(|operator| node_text(operator, source) == "+") =>
        {
            let left = node
                .child_by_field_name("left")
                .map(|left| evaluate_collection(left, source))
                .unwrap_or_else(EvaluatedCollection::unavailable);
            let right = node
                .child_by_field_name("right")
                .map(|right| evaluate_collection(right, source))
                .unwrap_or_else(EvaluatedCollection::unavailable);
            EvaluatedCollection {
                names: left.names.into_iter().chain(right.names).collect(),
                recognized: left.recognized || right.recognized,
                complete: left.complete && right.complete,
            }
        }
        "assignment" => node
            .child_by_field_name("right")
            .map(|right| evaluate_collection(right, source))
            .unwrap_or_else(EvaluatedCollection::unavailable),
        _ => EvaluatedCollection::unavailable(),
    }
}

fn evaluate_string(node: Node<'_>, source: &[u8]) -> Option<ExplicitExportName> {
    let name = match node.kind() {
        "string" => plain_string_value(node, source)?,
        "concatenated_string" => {
            let mut value = String::new();
            let mut found = false;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "string" {
                    return None;
                }
                found = true;
                value.push_str(&plain_string_value(child, source)?);
            }
            found.then_some(value)?
        }
        "parenthesized_expression" => {
            return single_named_child(node).and_then(|child| evaluate_string(child, source));
        }
        _ => return None,
    };
    Some(ExplicitExportName {
        name,
        span: source_span(node),
    })
}

fn plain_string_value(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let children = node.named_children(&mut cursor).collect::<Vec<_>>();
    if children.iter().any(|child| child.kind() == "interpolation") {
        return None;
    }
    let start = children
        .iter()
        .find(|child| child.kind() == "string_start")?;
    let prefix = node_text_utf8(*start, source)?
        .chars()
        .take_while(|character| *character != '\'' && *character != '"')
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if prefix
        .chars()
        .any(|character| matches!(character, 'b' | 'f' | 't'))
    {
        return None;
    }

    let mut value = String::new();
    for content in children
        .iter()
        .filter(|child| child.kind() == "string_content")
    {
        let text = node_text_utf8(*content, source)?;
        if text.contains('\\') {
            return None;
        }
        value.push_str(text);
    }
    Some(value)
}

fn statement_expression(statement: Node<'_>) -> Option<Node<'_>> {
    (statement.kind() == "expression_statement")
        .then(|| single_named_child(statement))
        .flatten()
}

fn single_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let mut children = node.named_children(&mut cursor);
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

fn assignment_sets_exports(node: Node<'_>, source: &[u8]) -> bool {
    node.child_by_field_name("left")
        .is_some_and(|target| node_text(target, source) == "__all__")
}

fn assignment_mutates_exports(node: Node<'_>, source: &[u8]) -> bool {
    node.child_by_field_name("left")
        .is_some_and(|target| export_target(target, source))
}

fn export_target(node: Node<'_>, source: &[u8]) -> bool {
    let target = node_text(node, source);
    target == "__all__" || target.starts_with("__all__[")
}

fn delete_targets_exports(node: Node<'_>, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| export_target(child, source))
}

fn export_mutation_method(node: Node<'_>, source: &[u8]) -> Option<String> {
    let function = node.child_by_field_name("function")?;
    if function.kind() != "attribute" {
        return None;
    }
    let object = function.child_by_field_name("object")?;
    if node_text(object, source) != "__all__" {
        return None;
    }
    let method = function
        .child_by_field_name("attribute")
        .map(|attribute| node_text(attribute, source))?;
    matches!(
        method.as_str(),
        "append" | "extend" | "clear" | "insert" | "pop" | "remove" | "reverse" | "sort"
    )
    .then_some(method)
}

fn nested_export_operation(node: Node<'_>, source: &[u8]) -> Option<SourceSpan> {
    if matches!(
        node.kind(),
        "function_definition" | "class_definition" | "lambda" | "decorated_definition"
    ) {
        return None;
    }
    let expression = statement_expression(node).unwrap_or(node);
    let is_operation = matches!(expression.kind(), "assignment" | "augmented_assignment")
        && assignment_mutates_exports(expression, source)
        || expression.kind() == "call" && export_mutation_method(expression, source).is_some()
        || expression.kind() == "delete_statement" && delete_targets_exports(expression, source);
    if is_operation {
        return Some(source_span(expression));
    }

    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| nested_export_operation(child, source))
}

fn source_span(node: Node<'_>) -> SourceSpan {
    let start = node.start_position();
    let end = node.end_position();
    SourceSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start: SourcePosition {
            line: start.row + 1,
            column: start.column + 1,
        },
        end: SourcePosition {
            line: end.row + 1,
            column: end.column + 1,
        },
    }
}

fn node_text(node: Node<'_>, source: &[u8]) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
        .into_owned()
}

fn node_text_utf8<'a>(node: Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    std::str::from_utf8(source.get(node.start_byte()..node.end_byte())?).ok()
}
