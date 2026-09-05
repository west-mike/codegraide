//! Parser-independent, structural possible-flow evidence (not a runtime trace).
use serde::Serialize;

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CallFlowKind {
    Sequence,
    Call,
    Condition,
    Alternatives,
    Branch,
    Loop,
    Exit,
    Unspecified,
    Unsupported,
    Statement,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CallFlow {
    pub kind: CallFlowKind,
    pub text: String,
    pub line: usize,
    pub column: usize,
    pub children: Vec<CallFlow>,
}
