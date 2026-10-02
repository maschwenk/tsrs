use tsrs_core::context::{Context, ContextError};

use crate::dynamic_queue::new_dynamic_queue;

// dynamic_queue_test.go:9
#[test]
fn test_dynamic_queue_fifo() {
    let ctx = Context::background();
    let q = new_dynamic_queue::<i32>();

    for i in 0..1000 {
        q.put(&ctx, i).unwrap();
    }

    for i in 0..1000 {
        let got = q.get(&ctx).unwrap();
        assert_eq!(got, i, "Get() = {}, want {}", got, i);
    }
}

// dynamic_queue_test.go:32
#[test]
fn test_dynamic_queue_get_cancellation() {
    let (ctx, cancel) = Context::background().with_cancel();
    cancel.call();

    let q = new_dynamic_queue::<i32>();
    let got = q.get(&ctx);
    assert_eq!(got, Err(ContextError::Canceled));
}

// dynamic_queue_test.go:47
#[test]
fn test_dynamic_queue_put_cancellation_while_state_unavailable() {
    let background = Context::background();
    let q = new_dynamic_queue::<i32>();
    let state = q.get_any(&background).unwrap();

    let (ctx, cancel) = background.with_cancel();
    cancel.call();

    let put_err = q.put(&ctx, 1);
    assert_eq!(put_err, Err(ContextError::Canceled));

    drop(state);

    q.put(&background, 2).unwrap();
    let got = q.get(&background).unwrap();
    assert_eq!(got, 2);
}

// Get blocks until an item is put from another thread, and a cancellation wakes a blocked Get.
#[test]
fn test_dynamic_queue_get_blocks_until_put_or_cancel() {
    let q = std::sync::Arc::new(new_dynamic_queue::<i32>());
    let q2 = q.clone();
    let t = std::thread::spawn(move || q2.get(&Context::background()));
    std::thread::sleep(std::time::Duration::from_millis(20));
    q.put(&Context::background(), 7).unwrap();
    assert_eq!(t.join().unwrap(), Ok(7));

    let (ctx, cancel) = Context::background().with_cancel();
    let q3 = q.clone();
    let t = std::thread::spawn(move || q3.get(&ctx));
    std::thread::sleep(std::time::Duration::from_millis(20));
    cancel.call();
    assert_eq!(t.join().unwrap(), Err(ContextError::Canceled));
    assert_eq!(q.len(), 0);
}
