// Memory regions of the language server (docs/LSP.md "Memory plan for a long-lived server"; not in Go, where the GC
// frees a program version and everything only it refers to).
//
// A program version owns:
// - its own region: what `CreateProgram` allocated (the cloned file list and resolution data of a reused program,
//   or everything a full build allocated on the calling thread);
// - the base region: the full build's region, shared by every version cloned from it (`UpdateProgram` hands the
//   shared processed-file data and the symlink cache to the next version);
// - the regions of its source files (each parsed file version has one, see parsecache.rs);
// - through its checker pool, the checkers' regions.
//
// `programOwner` is held by every `Project` value that refers to the program (projects are cloned per snapshot and
// live as long as a snapshot or a language service refers to them), so it is dropped exactly when nothing can reach
// the program any more. Dropping it frees the checkers and their regions, the `Program` itself, then the regions.

use std::sync::Arc;

use tsrs_compiler::Program;
use tsrs_core::arena::Region;

use crate::checkerpool::checkerPool;

pub(crate) struct programOwner {
    program: &'static Program,
    checker_pool: Option<Arc<checkerPool>>,
    // Dropped in this order after `drop` freed the checkers and the program.
    region: Region,
    pub(crate) base: Region,
    files: Vec<Region>,
}

impl programOwner {
    // `full_build`: the program does not share data with an older version; `base` is then its own region, which
    // remains the owner of legacy graph allocations (`Program::get_symlink_cache` routes there).
    pub(crate) fn new(program: &'static Program, checker_pool: Option<Arc<checkerPool>>, region: Region, base: Region, full_build: bool) -> programOwner {
        if full_build {
            base.adopt_owner(processed_files_addr(program));
        }
        let files = Region::containing_all(program.source_files().iter().map(|f| f.addr())).into_iter().flatten().collect();
        log_region(|| format!("program {:p}: created (region {} KiB, base {} KiB)", program, region.allocated_bytes() >> 10, base.allocated_bytes() >> 10));
        programOwner { program, checker_pool, region, base, files }
    }
}

// The address `ProgramData::get_symlink_cache` routes by: the shared processed-file container.
fn processed_files_addr(program: &'static Program) -> usize {
    std::ptr::from_ref(&***program).cast::<()>() as usize
}

impl Drop for programOwner {
    fn drop(&mut self) {
        log_region(|| format!("program {:p}: freed ({} files, region {} KiB)", self.program, self.files.len(), self.region.allocated_bytes() >> 10));
        if let Some(pool) = &self.checker_pool {
            pool.free_checkers();
        }
        // SAFETY: the last project value referring to the program is being dropped: no snapshot, language service or
        // held checker can reach it (see the file comment).
        unsafe { tsrs_compiler::free_program(self.program) };
    }
}

// `TSRS_REGION_LOG=1`: one stderr line per program created / freed.
fn log_region(msg: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("TSRS_REGION_LOG").is_some_and(|v| v == "1")) {
        eprintln!("regions: {}", msg());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsrs_compiler::{new_compiler_host, new_program, ProgramOptions};
    use tsrs_core::{CompilerOptions, Tristate, P};
    use tsrs_core::tspath::Path;

    #[test]
    fn symlink_cache_uses_shared_base_after_program_data_split() {
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let base = Region::new(4096);
            let observed = Arc::clone(&dropped);
            base.on_free(Box::new(move || { observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst); }));
            let host = |text: &str| {
                new_compiler_host("/", Arc::new(tsrs_vfs::vfstest::from_map([("/index.ts", text)], true)), "", None, None)
            };
            let old = {
                let _scope = base.enter();
                let options = P::new(CompilerOptions { no_lib: Tristate::True, ..Default::default() });
                let config = P::new(tsrs_tsoptions::new_parsed_command_line(options, vec!["/index.ts".into()], Vec::new(), Default::default()));
                let mut opts = ProgramOptions::new(config, host("export const value = 1;"));
                opts.single_threaded = Tristate::True;
                new_program(opts)
            };
            let old_owner = programOwner::new(old, None, base.clone(), base.clone(), true);
            let processed_addr = processed_files_addr(old);
            assert!(Region::containing(processed_addr).is_some());
            let cache = old.get_symlink_cache();
            assert!(Region::containing(cache.addr()).is_some(), "the cache must allocate in the base region");

            let version = Region::new(4096);
            let new = {
                let _scope = version.enter();
                let (new, _, reused) = old.reuse_program(&Path::from("/index.ts"), host("export const value = 2;"), None, None);
                assert!(reused);
                new.unwrap()
            };
            let _new_owner = programOwner::new(new, None, version, base, false);
            assert_eq!(processed_files_addr(new), processed_addr);
            drop(old_owner);
            assert_eq!(new.get_symlink_cache(), cache);
            assert!(Region::containing(cache.addr()).is_some());
            assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
        // Freed native addresses may already belong to another concurrently running test. Observe this owner's
        // destruction directly rather than looking the old addresses up after their lifetime ends.
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
