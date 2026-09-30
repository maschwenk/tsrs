// Development backend: renders diagnostics captured from the Go implementation by
// tools/oracle/testrunner (`tsrs-oracle-testrunner diags > diags.jsonl`) through this crate's baseline
// renderer. It validates enumeration, variant naming, rendering, classification and the worker pool
// independently of the Rust compiler. Also the source of `options.json` until tsrs_tsoptions builds.

use std::rc::Rc;

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use tsrs_diagnostics::Category;

use crate::compiler_runner::{self, Outcome, TestItem};
use crate::diagnosticwriter::{Diag, FileLike};
use crate::harnessutil::{OptKind, OptionDecl, OptionTable, TestFile};
use crate::tsbaseline;

pub struct Oracle {
    records: FxHashMap<String, Value>,
}

impl Oracle {
    pub fn load(path: &str) -> Oracle {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read oracle file {path}: {e}"));
        let mut records = FxHashMap::default();
        for line in text.lines() {
            let v: Value = serde_json::from_str(line).expect("bad oracle json");
            let name = v["name"].as_str().unwrap();
            let stem = compiler_runner::baseline_stem(name);
            records.insert(format!("{}/{}", v["suite"].as_str().unwrap(), stem), v);
        }
        Oracle { records }
    }

    pub fn run(&self, item: &TestItem) -> Outcome {
        let Some(rec) = self.records.get(&item.id()) else {
            return Outcome::Error(format!("no oracle record for {}", item.id()));
        };
        if let Some(e) = rec["error"].as_str() {
            return Outcome::Error(format!("oracle: {e}"));
        }
        if let Some(s) = rec["skip"].as_str() {
            return Outcome::Skip(s.to_string());
        }
        let files: Vec<TestFile> = rec["files"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| TestFile { unit_name: f["name"].as_str().unwrap().to_string(), content: f["content"].as_str().unwrap().to_string() })
                    .collect()
            })
            .unwrap_or_default();
        let mut file_cache: FxHashMap<String, Rc<FileLike>> = FxHashMap::default();
        let texts = &rec["texts"];
        let diags: Vec<Diag> =
            rec["diags"].as_array().map(|a| a.iter().map(|d| to_diag(d, texts, &mut file_cache)).collect()).unwrap_or_default();
        Outcome::Baseline(tsbaseline::do_error_baseline(&files, &diags, rec["pretty"].as_bool().unwrap_or(false)))
    }
}

fn category(n: i64) -> Category {
    match n {
        0 => Category::Warning,
        1 => Category::Error,
        2 => Category::Suggestion,
        _ => Category::Message,
    }
}

fn bundled_lib_text(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap();
    let path = compiler_runner::repo_root().join("crates/tsrs_vfs/libs").join(base);
    std::fs::read_to_string(path).unwrap_or_default()
}

fn to_diag(v: &Value, texts: &Value, cache: &mut FxHashMap<String, Rc<FileLike>>) -> Diag {
    let file = v["file"].as_str().map(|name| {
        cache
            .entry(name.to_string())
            .or_insert_with(|| {
                let text = match texts[name].as_str() {
                    Some(t) => t.to_string(),
                    None => bundled_lib_text(name),
                };
                FileLike::new(name.to_string(), text)
            })
            .clone()
    });
    let message = v["message"].as_str().unwrap_or("").to_string();
    let key = v["key"].as_str().unwrap_or("");
    let strs = |k: &str| -> Vec<String> { v[k].as_array().map(|a| a.iter().map(|s| s.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default() };
    let list = |k: &str, cache: &mut FxHashMap<String, Rc<FileLike>>| -> Vec<Diag> {
        v[k].as_array().map(|a| a.iter().map(|d| to_diag(d, texts, cache)).collect()).unwrap_or_default()
    };
    let chain = list("chain", cache);
    let related = list("related", cache);
    Diag {
        file,
        pos: v["pos"].as_i64().unwrap_or(0) as i32,
        end: v["end"].as_i64().unwrap_or(0) as i32,
        code: v["code"].as_i64().unwrap_or(0) as i32,
        category: category(v["category"].as_i64().unwrap_or(1)),
        source: v["source"].as_str().unwrap_or("").to_string(),
        identity: if key.is_empty() { message.clone() } else { key.to_string() },
        message,
        args: strs("args"),
        chain,
        related,
    }
}

// Option table dumped by `tsrs-oracle-testrunner options`.
pub fn load_option_table(path: &str) -> Result<OptionTable, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut decls = Vec::new();
    let mut vary_by = FxHashSet::default();
    for o in v.as_array().ok_or("options.json: expected array")? {
        let name = o["name"].as_str().unwrap().to_string();
        let kind = match o["kind"].as_str().unwrap() {
            "boolean" => OptKind::Boolean,
            "enum" => OptKind::Enum(
                o["enum"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p[0].as_str().unwrap().to_string(), p[1].as_str().unwrap().to_string()))
                    .collect(),
            ),
            _ => OptKind::Other,
        };
        if o["vary"].as_bool().unwrap_or(false) {
            vary_by.insert(name.to_lowercase());
        }
        decls.push(OptionDecl { name, kind });
    }
    Ok(OptionTable { decls, vary_by })
}
