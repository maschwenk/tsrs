// One test function on purpose: the harness then runs no other test thread while this one forks, which is the
// precondition of `fork_children` (every other thread idle).
#![cfg(unix)]

use std::io::Write;

use tsrs_core::procs::fork_children;

#[test]
fn forked_children() {
    // Results come back per child, in child order, also when they exceed a pipe buffer.
    let base = vec![7u8; 1 << 20];
    // SAFETY: no other thread runs (see the top of the file); the children only read `base` and allocate.
    let children = unsafe { fork_children(4, &mut |i| base.iter().take(100_000 * (i + 1)).map(|&b| b + i as u8).collect()) }.unwrap();
    let outputs = children.join().unwrap();
    assert_eq!(outputs.len(), 4);
    for (i, out) in outputs.iter().enumerate() {
        assert_eq!(out.data.len(), 100_000 * (i + 1));
        assert!(out.data.iter().all(|&b| b == 7 + i as u8));
    }

    // A child that exits badly is an error that carries what it printed; the other child is still reaped.
    // SAFETY: as above.
    let children = unsafe {
        fork_children(2, &mut |i| {
            if i == 1 {
                let _ = std::io::stderr().write_all(b"child one gives up\n");
                // SAFETY: ends the child at once.
                unsafe { libc_exit(3) };
            }
            vec![1, 2, 3]
        })
    }
    .unwrap();
    let error = children.join().err().unwrap();
    assert!(error.contains("exited with status 3"), "{error}");
    assert!(error.contains("child one gives up"), "{error}");

    // A panicking child is reported as such (its message goes to the test harness's capture, not to the pipe).
    // SAFETY: as above.
    let children = unsafe { fork_children(1, &mut |_| panic!("boom")) }.unwrap();
    let error = children.join().err().unwrap();
    assert!(error.contains("panicked"), "{error}");

    // Many rounds of many children: nothing hangs, nothing leaks a descriptor.
    for round in 0..50 {
        // SAFETY: as above.
        let children = unsafe { fork_children(16, &mut |i| vec![(round + i) as u8; 1000]) }.unwrap();
        let outputs = children.join().unwrap();
        assert!(outputs.iter().enumerate().all(|(i, o)| o.data == vec![(round + i) as u8; 1000]));
    }
}

extern "C" {
    #[link_name = "_exit"]
    fn libc_exit(status: i32) -> !;
}
