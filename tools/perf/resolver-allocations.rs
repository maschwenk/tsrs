// Standalone allocation probe. Build and measurement commands: notes/perf-resolver-allocations.md.
use std::hash::{Hash, Hasher};
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, P};
use tsrs_module::{new_resolver, ResolutionHost, ResolverOptions};
use tsrs_vfs::{cachedvfs, vfstest, FS};
struct Host {
    fs: Box<dyn FS>,
}
impl ResolutionHost for Host {
    fn fs(&self) -> &dyn FS {
        self.fs.as_ref()
    }
    fn get_current_directory(&self) -> &str {
        "/repo"
    }
}
fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "resolve".into());
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    if mode == "json" {
        let source = std::fs::read_to_string(std::env::args().nth(2).expect("json mode requires the path to xstate-main/package.json")).unwrap();
        for _ in 0..20_000 {
            let fields = tsrs_module::packagejson::parse(&source).unwrap();
            fields.name.value.hash(&mut hash);
            std::hint::black_box(fields);
        }
    } else {
        let mut files = Vec::new();
        for i in 0..1000 {
            files.push((format!("/repo/src/mod{i}.ts"), "export const x = 1".to_string()));
            files.push((
                format!("/repo/node_modules/pkg{i}/package.json"),
                format!(r#"{{"name":"pkg{i}","version":"1.0.0","exports":{{"types":"./index.d.ts","default":"./index.js"}}}}"#),
            ));
            files.push((format!("/repo/node_modules/pkg{i}/index.d.ts"), "export const x: number".to_string()));
        }
        let host = Box::leak(Box::new(Host { fs: Box::new(cachedvfs::from(vfstest::from_map(files, true))) }));
        let mut options = CompilerOptions { module_resolution: ModuleResolutionKind::Bundler, module: ModuleKind::ESNext, ..Default::default() };
        if mode == "suffix" {
            options.module_suffixes = Some(vec![".native".into(), ".web".into(), "".into()]);
        }
        let options = P::new(options);
        let requests: Vec<String> = (0..1000).flat_map(|i| [format!("./mod{i}"), format!("./missing{i}"), format!("pkg{i}"), format!("absent{i}")]).collect();
        let rounds = if mode == "hits" { 20 } else { 10 };
        let mut resolver = new_resolver(ResolverOptions::new(host, options));
        for round in 0..rounds {
            if mode != "hits" && round > 0 {
                resolver = new_resolver(ResolverOptions::new(host, options));
            }
            for name in &requests {
                let (r, _) = resolver.resolve_module_name(name, "/repo/src/index.ts", ModuleKind::ESNext, None).unwrap();
                r.resolved_file_name.hash(&mut hash);
                r.extension.hash(&mut hash);
                r.package_id.to_string().hash(&mut hash);
            }
        }
    }
    println!("{:016x}", hash.finish());
    tsrs_core::alloc_profile_dump();
}
