use rustc_hash::FxHashSet;
use std::sync::LazyLock;

// require('module').builtinModules.filter(x => !x.match(/^(?:_|node:)/))
pub static UNPREFIXED_NODE_CORE_MODULES: &[&str] = &[
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

// require('module').builtinModules.filter(x => x.startsWith('node:'))
pub static EXCLUSIVELY_PREFIXED_NODE_CORE_MODULES: &[&str] = &["node:quic", "node:sea", "node:sqlite", "node:test", "node:test/reporters"];

static NODE_CORE_MODULES: LazyLock<FxHashSet<String>> = LazyLock::new(|| {
    let mut node_core_modules = FxHashSet::default();
    for unprefixed in UNPREFIXED_NODE_CORE_MODULES {
        node_core_modules.insert(unprefixed.to_string());
        node_core_modules.insert(format!("node:{unprefixed}"));
    }
    for m in EXCLUSIVELY_PREFIXED_NODE_CORE_MODULES {
        node_core_modules.insert(m.to_string());
    }
    node_core_modules
});

pub fn is_unprefixed_node_core_module(name: &str) -> bool {
    UNPREFIXED_NODE_CORE_MODULES.contains(&name)
}

pub fn is_exclusively_prefixed_node_core_module(name: &str) -> bool {
    EXCLUSIVELY_PREFIXED_NODE_CORE_MODULES.contains(&name)
}

pub fn node_core_modules() -> &'static FxHashSet<String> {
    &NODE_CORE_MODULES
}

pub fn non_relative_module_name_for_typing_cache(module_name: &str) -> &str {
    if node_core_modules().contains(module_name) {
        return "node";
    }
    module_name
}
