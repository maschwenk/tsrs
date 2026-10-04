// `tsrs-test types-dump`: the `.types` / `.symbols` baseline walk over a whole tsconfig project, the Rust side of
// tools/oracle/project-types (same program setup, file selection, walk order and output layout; see there).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustc_hash::FxHashSet;
use tsrs_compiler::{get_diagnostics_of_any_program, new_cached_fs_compiler_host, new_program, Context, ProgramOptions};
use tsrs_core::{tspath, Tristate, P};
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{bundled, osvfs, FS};

use crate::type_symbol_baseline::{project_file_baseline, TypeWriterWalker};

struct OsParseConfigHost {
    fs: Arc<dyn FS>,
    cwd: String,
}

impl ParseConfigHost for OsParseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }

    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

pub struct DumpArgs {
    pub project: String,
    pub out: PathBuf,
    pub mode: String,
    pub text: String,
    pub sample: Option<String>,
}

pub fn run(args: DumpArgs) {
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let cwd = tspath::normalize_path(&std::env::current_dir().unwrap().to_string_lossy());
    let host: &'static OsParseConfigHost = Box::leak(Box::new(OsParseConfigHost { fs: fs.clone(), cwd: cwd.clone() }));
    let mut config_path = tspath::get_normalized_absolute_path(&args.project, &cwd);
    if fs.directory_exists(&config_path) {
        config_path = tspath::combine_paths(&config_path, &["tsconfig.json"]);
    }
    let (config, errs) = tsoptions::get_parsed_command_line_of_config_file(&config_path, Some(&Default::default()), None, host, None);
    if !errs.is_empty() || config.is_none() {
        eprintln!("config errors: {}", errs.len());
        std::process::exit(1);
    }
    let compiler_host = new_cached_fs_compiler_host(&cwd, fs, &bundled::lib_path(), None, None);
    let mut opts = ProgramOptions::new(P::new(config.unwrap()), compiler_host);
    opts.single_threaded = Tristate::True;
    let program = new_program(opts);
    let diags = get_diagnostics_of_any_program(
        &Context::default(),
        program,
        None,
        false,
        &mut |ctx, file| program.get_bind_diagnostics(ctx, file),
        &mut |ctx, file| program.get_semantic_diagnostics(ctx, file),
    );
    eprintln!("diagnostics: {}", diags.len());

    let config_dir = tspath::get_directory_path(&config_path);
    let mut files = Vec::new();
    for &f in program.source_files() {
        if f.file_name().contains("/node_modules/") || program.is_source_file_default_library(&f.path()) {
            continue;
        }
        files.push((f, rel_name(&config_dir, f.file_name())));
    }

    let mut wanted: Option<FxHashSet<String>> = match args.text.as_str() {
        "all" | "none" => None,
        list => Some(read_list(list)),
    };
    if let Some(sample) = &args.sample {
        let sample = read_list(sample);
        files.retain(|(_, rel)| sample.contains(rel));
        eprintln!("sample: {} of {} listed files are in the program", files.len(), sample.len());
    }

    let kinds: &[bool] = match args.mode.as_str() {
        "types" => &[false],
        "symbols" => &[true],
        "both" => &[false, true],
        m => {
            eprintln!("bad --mode {m}");
            std::process::exit(2)
        }
    };
    let mut walker = TypeWriterWalker::new(program, !diags.is_empty());
    std::fs::create_dir_all(&args.out).unwrap();
    for &is_symbol in kinds {
        let kind = if is_symbol { "symbols" } else { "types" };
        let mut mw = std::io::BufWriter::new(std::fs::File::create(args.out.join(format!("manifest.{kind}"))).unwrap());
        for (i, (f, rel)) in files.iter().enumerate() {
            let section = project_file_baseline(&mut walker, *f, rel, is_symbol);
            writeln!(mw, "{:016x}\t{}\t{}", fnv1a64(&section), section.matches('\n').count(), rel).unwrap();
            if args.text == "all" || wanted.as_ref().is_some_and(|w| w.contains(rel)) {
                let p = args.out.join(kind).join(format!("{rel}.{kind}"));
                std::fs::create_dir_all(p.parent().unwrap_or(Path::new("."))).unwrap();
                std::fs::write(&p, section.as_bytes()).unwrap();
            }
            if let Some(w) = wanted.as_mut().filter(|_| kinds.len() == 1) {
                if w.remove(rel) && w.is_empty() {
                    // Every listed file is written; later files cannot change them.
                    break;
                }
            }
            if (i + 1) % 1000 == 0 {
                eprintln!("{kind}: {}/{}", i + 1, files.len());
            }
        }
        let c = program.get_type_checker(&Context::default());
        writeln!(mw, "#counts\t{}\t{}\t{}", c.type_count, c.symbol_count, c.total_instantiation_count).unwrap();
        drop(c);
        mw.flush().unwrap();
    }
}

// One relative path per line; blank lines and `#` comments are ignored.
fn read_list(path: &str) -> FxHashSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn rel_name(dir: &str, file_name: &str) -> String {
    let rel = tspath::get_relative_path_from_directory(dir, file_name, &tspath::ComparePathsOptions { use_case_sensitive_file_names: true, ..Default::default() });
    rel.split('/').map(|p| if p == ".." { "_up_" } else { p }).collect::<Vec<_>>().join("/")
}

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}
