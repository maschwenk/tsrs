use std::sync::Mutex;

use rustc_hash::FxHashMap;
use tsrs_compiler::Program;

// Programs are keyed by address (Go `map[*compiler.Program]int32`).
#[derive(Default)]
pub(crate) struct programCounter {
    mu: Mutex<FxHashMap<usize, i32>>,
}

fn key(program: &'static Program) -> usize {
    std::ptr::from_ref::<Program>(program) as usize
}

impl programCounter {
    // programcounter.go:16
    // Ref increments the reference count for a program. If the program is not
    // yet tracked, it is added with a reference count of 1.
    pub(crate) fn ref_(&self, program: &'static Program) {
        *self.mu.lock().unwrap().entry(key(program)).or_insert(0) += 1;
    }

    // programcounter.go:25
    pub(crate) fn deref(&self, program: &'static Program) -> bool {
        let mut refs = self.mu.lock().unwrap();
        let Some(&count) = refs.get(&key(program)) else {
            return false;
        };
        let count = count - 1;
        if count < 0 {
            panic!("program reference count went below zero");
        }
        if count == 0 {
            refs.remove(&key(program));
            return true;
        }
        refs.insert(key(program), count);
        false
    }

    // programcounter.go:44
    pub(crate) fn len(&self) -> usize {
        self.mu.lock().unwrap().len()
    }
}
