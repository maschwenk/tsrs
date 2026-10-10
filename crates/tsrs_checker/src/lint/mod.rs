//! Native lint rules, dispatched by the checker after checking each relevant node.
mod no_floating_promises;
mod rule;
mod utils;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::Value;
use tsrs_ast::{self as ast, Kind, Node, SourceFile};
use tsrs_core::P;
use tsrs_core::tspath::{self, Path};

use crate::{Checker, NodeCheckFlags};
use rule::RuleContext;
pub use rule::{Fixes, RuleDiagnostic, RuleFix, RuleLabeledRange, RuleMessage, RuleSuggestion};

pub const NO_FLOATING_PROMISES: &str = "no-floating-promises";

/// Oxlint's version 2 headless input, also accepted by `tsrs --lint`.
#[derive(Deserialize)]
pub struct HeadlessConfig {
    pub version: i32,
    #[serde(default)]
    pub configs: Vec<FileConfig>,
    #[serde(default)]
    pub source_overrides: Option<FxHashMap<String, String>>,
    #[serde(default)]
    pub report_syntactic: bool,
    #[serde(default)]
    pub report_semantic: bool,
}

#[derive(Deserialize)]
pub struct FileConfig {
    #[serde(default)]
    pub file_paths: Vec<String>,
    #[serde(default)]
    pub rules: Vec<RequestedRule>,
}

#[derive(Deserialize)]
pub struct RequestedRule {
    pub name: String,
    #[serde(default)]
    pub options: Value,
}

#[derive(Debug)]
enum ConfiguredRule {
    NoFloatingPromises(no_floating_promises::Options),
}

impl ConfiguredRule {
    fn name(&self) -> &'static str {
        match self {
            Self::NoFloatingPromises(_) => NO_FLOATING_PROMISES,
        }
    }

    fn check(&self, context: &mut RuleContext<'_>, node: P<Node>) {
        match self {
            Self::NoFloatingPromises(options) => {
                no_floating_promises::check_statement(context, options, node)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuleTimingRecord {
    pub rule_name: String,
    pub duration: Duration,
    pub calls: u64,
}

#[derive(Debug, Default)]
pub struct LintOutput {
    pub diagnostics: Vec<RuleDiagnostic>,
    pub timings: Vec<RuleTimingRecord>,
    pub no_emit: bool,
}

#[derive(Clone, Default)]
pub(crate) struct FileLintOutput {
    diagnostics: Vec<RuleDiagnostic>,
    timings: BTreeMap<&'static str, (Duration, u64)>,
}

/// Prepared once before program creation. Checkers keep their own results and publish only files
/// they own; type queries may visit a different file's bodies before its own checker gets to it.
/// An empty rule list still selects the file for mandatory semantic checking.
#[derive(Debug)]
pub struct LintConfig {
    files: FxHashMap<Path, Arc<[ConfiguredRule]>>,
    fixes: Fixes,
    timings: bool,
    output: Mutex<LintOutput>,
}

impl LintConfig {
    pub fn new(
        configs: &[FileConfig],
        cwd: &str,
        case_sensitive: bool,
        fixes: Fixes,
        timings: bool,
    ) -> Result<Self, String> {
        let mut files = FxHashMap::default();
        for config in configs {
            let mut rules = Vec::new();
            for rule in &config.rules {
                match rule.name.as_str() {
                    NO_FLOATING_PROMISES => rules.push(ConfiguredRule::NoFloatingPromises(
                        no_floating_promises::parse_options(&rule.options)?,
                    )),
                    // Oxlint may send rules this backend has not ported yet.
                    _ => {}
                }
            }
            let rules: Arc<[ConfiguredRule]> = rules.into();
            for file in &config.file_paths {
                files.insert(
                    tspath::to_path(file, cwd, case_sensitive),
                    Arc::clone(&rules),
                );
            }
        }
        Ok(Self {
            files,
            fixes,
            timings,
            output: Mutex::new(LintOutput::default()),
        })
    }

    /// Match compiler paths resolved through symlinks as well as the paths in the payload.
    pub fn add_path_alias(&mut self, path: &Path, alias: Path) {
        if let Some(rules) = self.files.get(path) {
            self.files.insert(alias, Arc::clone(rules));
        }
    }

    pub fn includes(&self, file: P<SourceFile>) -> bool {
        self.files.contains_key(file.path())
    }

    /// Called on the owning checker after semantic checking, before its AST can be reclaimed.
    pub fn collect(&self, checker: &mut Checker, file: P<SourceFile>) {
        let Some(file_output) = checker.lint_output.get_mut().remove(&file) else {
            return;
        };
        let mut output = self.output.lock().unwrap();
        output.no_emit = checker.compiler_options.no_emit.is_true();
        output.diagnostics.extend(file_output.diagnostics);
        for (name, (duration, calls)) in file_output.timings {
            if let Some(record) = output
                .timings
                .iter_mut()
                .find(|record| record.rule_name == name)
            {
                record.duration += duration;
                record.calls += calls;
            } else {
                output.timings.push(RuleTimingRecord {
                    rule_name: name.to_string(),
                    duration,
                    calls,
                });
            }
        }
    }

    pub fn take_output(&self) -> LintOutput {
        let mut output = std::mem::take(&mut *self.output.lock().unwrap());
        // Deferred function bodies are checked out of source order. Keep the protocol and rule
        // tester deterministic without retaining a second list of AST nodes.
        output.diagnostics.sort_by(|a, b| {
            a.source_file
                .file_name()
                .cmp(b.source_file.file_name())
                .then(a.range.pos().cmp(&b.range.pos()))
                .then(b.range.end().cmp(&a.range.end()))
        });
        output.timings.sort_by(|a, b| a.rule_name.cmp(&b.rule_name));
        output
    }
}

impl Checker {
    pub(crate) fn lint_node(&mut self, node: P<Node>) {
        let Some(config) = self.lint_config else {
            return;
        };
        let Some(file) = ast::get_source_file_of_node(node) else {
            return;
        };
        let Some(rules) = config
            .files
            .get(file.path())
            .filter(|rules| !rules.is_empty())
        else {
            return;
        };
        let links = self.node_links.get(node);
        if links.flags.get().intersects(NodeCheckFlags::LintChecked) {
            return;
        }
        links
            .flags
            .set(links.flags.get() | NodeCheckFlags::LintChecked);
        for rule in rules.iter() {
            let start = config.timings.then(Instant::now);
            let mut context = RuleContext {
                source_file: file,
                program: self.program,
                checker: self,
                rule_name: rule.name(),
                fixes: config.fixes,
            };
            rule.check(&mut context, node);
            if let Some(start) = start {
                let stat = self
                    .lint_output
                    .get_mut()
                    .entry(file)
                    .or_default()
                    .timings
                    .entry(rule.name())
                    .or_default();
                stat.0 += start.elapsed();
                stat.1 += 1;
            }
        }
    }

    /// `with` bodies are deliberately skipped by TypeScript's checker. Visit only that skipped
    /// subtree for lint coverage; the ordinary checker traversal handles everything else.
    pub(crate) fn lint_unchecked_node(&mut self, node: P<Node>) {
        if self.lint_config.is_none() {
            return;
        }
        if node.kind() == Kind::ExpressionStatement {
            self.lint_node(node);
        }
        node.for_each_child(&mut |child| {
            self.lint_unchecked_node(child);
            false
        });
    }
}
