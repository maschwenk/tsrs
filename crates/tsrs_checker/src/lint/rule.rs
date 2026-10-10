use crate::{Checker, Program};
use serde::{Deserialize, Serialize};
use tsrs_ast::{Node, SourceFile};
use tsrs_core::{P, TextRange};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleMessage {
    pub id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl RuleMessage {
    pub fn new(id: &str, description: &str, help: Option<&str>) -> RuleMessage {
        RuleMessage {
            id: id.to_string(),
            description: description.to_string(),
            help: help.map(str::to_string),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleFix {
    pub text: String,
    pub range: TextRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleSuggestion {
    pub message: RuleMessage,
    pub fixes: Vec<RuleFix>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleLabeledRange {
    pub label: String,
    pub range: TextRange,
}

#[derive(Clone, Debug)]
pub struct RuleDiagnostic {
    pub range: TextRange,
    pub rule_name: &'static str,
    pub message: RuleMessage,
    pub fixes: Vec<RuleFix>,
    pub suggestions: Vec<RuleSuggestion>,
    pub source_file: P<SourceFile>,
    pub labeled_ranges: Vec<RuleLabeledRange>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Fixes {
    pub fix: bool,
    pub fix_suggestions: bool,
}

pub(super) struct RuleContext<'a> {
    pub source_file: P<SourceFile>,
    pub program: &'static dyn Program,
    pub checker: &'a mut Checker,
    pub rule_name: &'static str,
    pub fixes: Fixes,
}

impl RuleContext<'_> {
    pub fn emit(&self, diagnostic: RuleDiagnostic) {
        self.checker
            .lint_output
            .borrow_mut()
            .entry(self.source_file)
            .or_default()
            .diagnostics
            .push(diagnostic);
    }

    pub fn trim_node_range(&self, node: P<Node>) -> TextRange {
        tsrs_scanner::get_range_of_token_at_position(self.source_file, node.pos())
            .with_end(node.end())
    }

    pub fn report(
        &self,
        range: TextRange,
        message: RuleMessage,
        labeled_ranges: Vec<RuleLabeledRange>,
    ) {
        self.emit(RuleDiagnostic {
            range,
            rule_name: self.rule_name,
            message,
            fixes: Vec::new(),
            suggestions: Vec::new(),
            source_file: self.source_file,
            labeled_ranges,
        });
    }
}
