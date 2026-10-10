use std::fmt::Write as _;
use std::sync::Arc;

use tsrs_compiler::{new_compiler_host, new_program, CompilerHost, ProgramOptions};
use tsrs_core::context::Context;
use tsrs_core::P;
use tsrs_lsproto as lsproto;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{bundled, vfstest, FS};

use crate::autoimport::{ProjectID, Registry};
use crate::lsconv::{self, compute_lsp_line_starts, Converters};
use crate::lsutil::{new_default_user_preferences, UserPreferences};
use crate::sourcemap::ECMALineInfo;
use crate::{new_language_service, Host};

struct testHost {
    fs: Arc<dyn FS>,
    converters: Arc<Converters>,
}

impl Host for testHost {
    fn use_case_sensitive_file_names(&self) -> bool {
        false
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.fs.read_file(path)
    }
    fn converters(&self) -> Arc<Converters> {
        self.converters.clone()
    }
    fn get_preferences(&self, _active_file: &str) -> UserPreferences {
        new_default_user_preferences()
    }
    fn get_ecma_line_info(&self, _file_name: &str) -> Option<Arc<ECMALineInfo>> {
        None
    }
    fn auto_import_registry(&self) -> Option<Arc<Registry>> {
        None
    }
    fn read_directory(&self, _: &str, _: &str, _: &[String], _: &[String], _: &[String], _: usize) -> Vec<String> {
        Vec::new()
    }
    fn get_directories(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
    fn directory_exists(&self, path: &str) -> bool {
        self.fs.directory_exists(path)
    }
    fn file_exists(&self, path: &str) -> bool {
        self.fs.file_exists(path)
    }
}

struct parseConfigHost {
    fs: Arc<dyn FS>,
}

impl ParseConfigHost for parseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        "/"
    }
}

// provideSymbolsAndEntries drives go-to-implementation with a breadth-first worklist. When an
// interface member has K implementations, every one of those K program-wide searches returns
// all K implementations. Without deduplicating, the retained references, the work queue, and the
// retained SymbolsAndEntries groups all grow O(K^2), which can exhaust memory on large,
// deeply-typed programs.
//
// The final LSP response is deduplicated by node, so the blow-up is invisible from the response;
// this white-box test inspects the pre-deduplication data that provideSymbolsAndEntries returns
// and asserts that both the accumulated reference count and the group count grow ~linearly with K
// (quadratic growth roughly quadruples when K doubles; deduplicated growth roughly doubles).
// findallreferences_test.go:35
#[test]
fn test_implementations_worklist_does_not_blow_up() {
    let measure = |k: usize| -> (usize, usize) {
        let mut b = String::new();
        b.push_str("interface I { m(): void; }\n");
        for i in 0..k {
            writeln!(b, "const a{}: I = {{ m() {{}} }};", i).unwrap();
        }
        b.push_str("declare const i: I;\n");
        b.push_str("i.m();\n");
        let content = b;

        let files = [("/repro.ts", content.as_str()), ("/tsconfig.json", r#"{ "compilerOptions": {}, "files": ["repro.ts"] }"#)];
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|&(k, v)| (k, v)), false /*useCaseSensitiveFileNames*/)));

        let config_host: &'static parseConfigHost = Box::leak(Box::new(parseConfigHost { fs: fs.clone() }));
        let (parsed, errors) = tsoptions::get_parsed_command_line_of_config_file("/tsconfig.json", None, None, config_host, None);
        assert_eq!(errors.len(), 0);
        let host: Arc<dyn CompilerHost> = new_compiler_host("/", fs.clone(), &bundled::lib_path(), None, None);
        let program = new_program(ProgramOptions::new(P::new(parsed.unwrap()), host));
        program.bind_source_files();
        let ctx = Context::background();
        program.get_semantic_diagnostics(&ctx, program.get_source_file("/repro.ts"));

        let source_file = program.get_source_file("/repro.ts").unwrap();
        let line_content = content.clone();
        let converters = lsconv::new_converters(lsproto::PositionEncodingKind::UTF8, move |_| Some(compute_lsp_line_starts(&line_content)));
        let l = new_language_service(ProjectID("/tsconfig.json".to_string()), program, Arc::new(testHost { fs, converters: converters.clone() }), "/repro.ts");

        // Position of the `m` property in the final `i.m();`.
        let offset = content.rfind("i.m").unwrap() + "i.".len();
        let (pos, _) = converters.to_lsp_position(&source_file, offset as i32);

        let data = l.provide_symbols_and_entries(&ctx, &lsproto::DocumentUri("file:///repro.ts".to_string()), pos, false /*isRename*/, true /*implementations*/);
        assert!(data.is_some());
        let data = data.unwrap();
        let mut refs = 0;
        for se in &data.symbols_and_entries {
            refs += se.references.len();
        }
        (refs, data.symbols_and_entries.len())
    };

    const K: usize = 40;
    let (small_refs, small_groups) = measure(K);
    let (large_refs, large_groups) = measure(2 * K);

    // Retained references (and, since each is enqueued at most once, the work queue) must grow
    // ~linearly (~2x when K doubles). The un-deduplicated worklist grows ~4x. Fail above 3x.
    assert!(
        large_refs <= small_refs * 3,
        "implementations worklist references scale superlinearly: K={} -> {}, K={} -> {} (expected ~linear); provideSymbolsAndEntries accumulates references without deduplicating by node",
        K,
        small_refs,
        2 * K,
        large_refs
    );

    // Retained SymbolsAndEntries groups must also grow ~linearly. Appending one group per search
    // result (K searches, each returning all K implementations) grows ~4x; dropping duplicate
    // empty groups keeps it bounded by the distinct definitions. Fail above 3x.
    assert!(
        large_groups <= small_groups * 3,
        "implementations worklist groups scale superlinearly: K={} -> {}, K={} -> {} (expected ~linear); provideSymbolsAndEntries retains a group per search result without deduplicating by definition",
        K,
        small_groups,
        2 * K,
        large_groups
    );
    // Not in Go: guard against a vacuous pass (nothing found at all).
    assert!(small_refs >= K, "expected at least K implementations, got {}", small_refs);
}
