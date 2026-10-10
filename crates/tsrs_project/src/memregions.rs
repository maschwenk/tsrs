// Regression coverage for compiler-owned program regions.
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tsrs_core::arena::Region;
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
            let processed_addr = std::ptr::from_ref(&***old).cast::<()>() as usize;
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
            drop(version);
            drop(base);
            assert_eq!(std::ptr::from_ref(&***new).cast::<()>() as usize, processed_addr);
            drop(old);
            assert_eq!(new.get_symlink_cache(), cache);
            assert!(Region::containing(cache.addr()).is_some());
            assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
        // Freed native addresses may already belong to another concurrently running test. Observe this owner's
        // destruction directly rather than looking the old addresses up after their lifetime ends.
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
