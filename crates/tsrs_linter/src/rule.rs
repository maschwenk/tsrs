use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tsrs_ast::{Node, SourceFile};
use tsrs_compiler::{Checker, Program};
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

#[derive(Clone, Debug)]
pub struct InternalDiagnostic {
    pub range: Option<TextRange>,
    pub id: String,
    pub description: String,
    pub help: Option<String>,
    pub file_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Fixes {
    pub fix: bool,
    pub fix_suggestions: bool,
}

pub struct RuleContext<'a> {
    pub source_file: P<SourceFile>,
    pub program: &'static Program,
    pub checker: &'a mut Checker,
    pub rule_name: &'static str,
    pub fixes: Fixes,
    pub on_diagnostic: &'a Arc<dyn Fn(RuleDiagnostic) + Send + Sync>,
}

impl RuleContext<'_> {
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
        (self.on_diagnostic)(RuleDiagnostic {
            range,
            rule_name: self.rule_name,
            message,
            fixes: Vec::new(),
            suggestions: Vec::new(),
            source_file: self.source_file,
            labeled_ranges,
        });
    }

    pub fn report_with_suggestions(
        &self,
        range: TextRange,
        message: RuleMessage,
        labeled_ranges: Vec<RuleLabeledRange>,
        make_suggestions: impl FnOnce() -> Vec<RuleSuggestion>,
    ) {
        let suggestions = if self.fixes.fix_suggestions {
            make_suggestions()
        } else {
            Vec::new()
        };
        (self.on_diagnostic)(RuleDiagnostic {
            range,
            rule_name: self.rule_name,
            message,
            fixes: Vec::new(),
            suggestions,
            source_file: self.source_file,
            labeled_ranges,
        });
    }
}

pub struct RuleDefinition {
    pub name: &'static str,
    pub run: for<'a> fn(&mut RuleContext<'a>, &Value) -> Result<u64, String>,
}

#[derive(Clone)]
pub struct ConfiguredRule {
    pub definition: &'static RuleDefinition,
    pub options: Value,
}
