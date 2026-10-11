use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Row {
    layer: &'static str,
    site: &'static str,
    logical_operation: &'static str,
    operation: &'static str,
    path: String,
    result: &'static str,
}

#[derive(Default)]
struct Collector {
    destination: Option<PathBuf>,
    rows: Mutex<BTreeMap<Row, u64>>,
    // Kept separate from aggregates and absent unless explicitly requested. Instrumented runs
    // are for call analysis only; recording paths and taking these locks perturbs timing.
    physical_events: Option<Mutex<Vec<serde_json::Value>>>,
}

impl Collector {
    fn record(&self, row: Row) {
        *self.rows.lock().unwrap().entry(row).or_default() += 1;
    }

    fn json(&self) -> serde_json::Value {
        let rows = self.rows.lock().unwrap();
        let rows: Vec<_> = rows
            .iter()
            .map(|(row, count)| {
                serde_json::json!({
                    "layer": row.layer,
                    "site": row.site,
                    "logical_operation": row.logical_operation,
                    "operation": row.operation,
                    "path": row.path,
                    "result": row.result,
                    "count": count,
                })
            })
            .collect();
        let mut report = serde_json::json!({
            "version": 1,
            "physical_unit": "Rust API or rustix wrapper calls; not a complete syscall trace",
            "rows": rows,
        });
        if let Some(events) = &self.physical_events {
            report["physical_events"] = serde_json::Value::Array(events.lock().unwrap().clone());
        }
        report
    }
}

static COLLECTOR: LazyLock<Option<Arc<Collector>>> = LazyLock::new(|| {
    let destination = PathBuf::from(std::env::var_os("TSRS_RESOLVER_FS_STATS")?);
    if !destination.is_absolute() {
        eprintln!("TSRS_RESOLVER_FS_STATS requires an absolute report path");
        return None;
    }
    let physical_events = (std::env::var("TSRS_RESOLVER_FS_TRACE").as_deref() == Ok("1")).then(|| Mutex::new(Vec::new()));
    Some(Arc::new(Collector { destination: Some(destination), physical_events, ..Collector::default() }))
});

#[derive(Clone)]
struct Context {
    collector: Arc<Collector>,
    site: &'static str,
    operation: &'static str,
    path: Option<Arc<str>>,
}

impl Context {
    fn root(collector: Arc<Collector>) -> Self {
        Self { collector, site: "nonresolver", operation: "none", path: None }
    }

    fn record(&self, layer: &'static str, operation: &'static str, path: &str, result: &'static str) {
        self.collector.record(Row { layer, site: self.site, logical_operation: self.operation, operation, path: path.to_string(), result });
    }

    fn physical_event(&self, operation: &'static str, path: &str, completed: Option<(usize, &'static str)>) -> Option<usize> {
        let events = self.collector.physical_events.as_ref()?;
        let mut events = events.lock().unwrap();
        let sequence = events.len() + 1;
        events.push(serde_json::json!({
            "sequence": sequence,
            "call": completed.map_or(sequence, |(call, _)| call),
            "phase": if completed.is_some() { "end" } else { "start" },
            "thread": format!("{:?}", std::thread::current().id()),
            "site": self.site,
            "logical_operation": self.operation,
            "logical_path": self.path.as_deref(),
            "operation": operation,
            "path": path,
            "result": completed.map(|(_, result)| result),
        }));
        Some(sequence)
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

struct Scope {
    previous: Option<Context>,
}

impl Scope {
    fn enter(context: Context) -> Self {
        Self { previous: CURRENT.with(|current| current.replace(Some(context))) }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
    }
}

fn context() -> Option<Context> {
    CURRENT.with(|current| current.borrow().clone()).or_else(|| COLLECTOR.as_ref().map(|collector| Context::root(Arc::clone(collector))))
}

pub fn enabled() -> bool {
    context().is_some()
}

pub fn logical<T>(
    site: &'static str,
    operation: &'static str,
    path: &str,
    f: impl FnOnce() -> T,
    result: impl FnOnce(&T) -> &'static str,
) -> T {
    let Some(context) = context() else {
        return f();
    };
    let context = Context { collector: context.collector, site, operation, path: Some(Arc::from(path)) };
    let _scope = Scope::enter(context.clone());
    let value = f();
    context.record("logical", operation, path, result(&value));
    value
}

pub fn physical<T>(operation: &'static str, path: &str, f: impl FnOnce() -> T, result: impl FnOnce(&T) -> &'static str) -> T {
    let Some(context) = context() else {
        return f();
    };
    let call = context.physical_event(operation, path, None);
    let value = f();
    let result = result(&value);
    context.record("physical", operation, path, result);
    if let Some(call) = call {
        context.physical_event(operation, path, Some((call, result)));
    }
    value
}

pub fn cache(operation: &'static str, outcome: &'static str) {
    if let Some(context) = context() {
        context.record("cache", operation, context.path.as_deref().unwrap_or_default(), outcome);
    }
}

/// Writes the cumulative process report. Called with the ordinary extended-diagnostics counters.
pub fn write_report() {
    let Some(collector) = COLLECTOR.as_ref() else {
        return;
    };
    let Some(destination) = &collector.destination else {
        return;
    };
    let result =
        serde_json::to_vec_pretty(&collector.json()).map_err(std::io::Error::other).and_then(|json| std::fs::write(destination, json));
    if let Err(error) = result {
        eprintln!("Could not write resolver filesystem statistics to {}: {error}", destination.display());
    }
}

#[cfg(test)]
#[path = "enabled_test.rs"]
mod tests;
