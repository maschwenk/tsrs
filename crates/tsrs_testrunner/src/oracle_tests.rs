// Test-only rendering of Go-captured diagnostics; the runner always compiles with Rust.

use std::rc::Rc;

use rustc_hash::FxHashMap;
use serde_json::Value;
use tsrs_diagnostics::Category;

use crate::compiler_runner;
use crate::diagnosticwriter::{Diag, FileLike};
use crate::harnessutil::TestFile;
use crate::tsbaseline;

fn render_record(rec: &Value) -> String {
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
    tsbaseline::do_error_baseline(&files, &diags, rec["pretty"].as_bool().unwrap_or(false))
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
    tsrs_core::utf8::read_to_string(path).unwrap_or_default()
}

fn to_diag(v: &Value, texts: &Value, cache: &mut FxHashMap<String, Rc<FileLike>>) -> Diag {
    let file = v["file"].as_str().map(|name| {
        Rc::clone(cache
            .entry(name.to_string())
            .or_insert_with(|| {
                let text = match texts[name].as_str() {
                    Some(t) => t.to_string(),
                    None => bundled_lib_text(name),
                };
                FileLike::new(name.to_string(), text)
            }))
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

#[test]
fn renders_like_go() {
    // Go-captured diagnostics (message chains, related information, two files) and Go's own rendering.
    let rec: Value = serde_json::from_str(include_str!("../testdata/importTag23.oracle.json")).unwrap();
    assert_eq!(render_record(&rec), rec["baseline"].as_str().unwrap());
}
