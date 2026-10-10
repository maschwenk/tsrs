use std::cell::Cell;

#[derive(Clone, Copy)]
pub(crate) enum PathBuffer {
    Extension,
    Suffix,
    PackageJson,
    NodeModules,
    Join,
}

thread_local! {
    static PATHS: [Cell<String>; 5] = const { [const { Cell::new(String::new()) }; 5] };
}

// Like oxc-resolver's scratch paths, keep capacity across requests on each worker. Take the buffer out before
// calling the host: a custom filesystem may resolve another module on this same thread. The nested request
// then owns a different buffer, and neither call holds a TLS borrow across filesystem or resolver callbacks.
pub(crate) fn with_path_buffer<T>(kind: PathBuffer, f: impl FnOnce(&mut String) -> T) -> T {
    PATHS.with(|paths| {
        let slot = &paths[kind as usize];
        let mut path = slot.take();
        path.clear();
        if path.capacity() == 0 {
            path.reserve(256);
        }
        let result = f(&mut path);
        if path.capacity() <= 64 * 1024 {
            path.clear();
            slot.set(path);
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_and_concurrent_paths_do_not_overwrite_each_other() {
        with_path_buffer(PathBuffer::Extension, |outer| {
            outer.push_str("/outer/é.ts");
            with_path_buffer(PathBuffer::Extension, |inner| {
                assert!(inner.is_empty());
                inner.push_str("/inner.ts");
            });
            std::thread::spawn(|| with_path_buffer(PathBuffer::Extension, |other| other.push_str("/other.ts"))).join().unwrap();
            assert_eq!(outer, "/outer/é.ts");
        });
        with_path_buffer(PathBuffer::Extension, |path| assert!(path.is_empty()));
    }

    #[test]
    fn long_paths_and_unwinding_leave_the_buffer_usable() {
        with_path_buffer(PathBuffer::Suffix, |path| path.push_str(&"é".repeat(40_000)));
        assert!(std::panic::catch_unwind(|| with_path_buffer(PathBuffer::Suffix, |path| {
            path.push_str("/abandoned");
            panic!("host callback");
        }))
        .is_err());
        with_path_buffer(PathBuffer::Suffix, |path| {
            assert!(path.is_empty());
            assert!(path.capacity() <= 64 * 1024);
            path.push_str("/next.ts");
        });
    }
}
