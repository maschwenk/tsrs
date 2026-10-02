use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use tsrs_core::context::Context;

use super::*;

// queue_test.go:15 TestQueue/BasicEnqueue
#[test]
fn queue_basic_enqueue() {
    let q = new_queue();

    let executed = Arc::new(AtomicBool::new(false));
    let e = executed.clone();
    q.enqueue(&Context::background(), move |_ctx| {
        e.store(true, Ordering::SeqCst);
    });

    q.wait();
    q.close();

    assert!(executed.load(Ordering::SeqCst));
}

// queue_test.go:30 TestQueue/MultipleTasksExecution
#[test]
fn queue_multiple_tasks_execution() {
    let q = new_queue();

    let counter = Arc::new(AtomicI64::new(0));
    let num_tasks = 10;

    for _ in 0..num_tasks {
        let c = counter.clone();
        q.enqueue(&Context::background(), move |_ctx| {
            c.fetch_add(1, Ordering::SeqCst);
        });
    }

    q.wait();
    q.close();

    assert_eq!(counter.load(Ordering::SeqCst), num_tasks);
}

// queue_test.go:49 TestQueue/NestedEnqueue
#[test]
fn queue_nested_enqueue() {
    let q = Arc::new(new_queue());

    let executed = Arc::new(Mutex::new(Vec::<String>::new()));

    let (q2, e) = (q.clone(), executed.clone());
    q.enqueue(&Context::background(), move |ctx| {
        e.lock().unwrap().push("parent".to_string());

        let e2 = e.clone();
        q2.enqueue(ctx, move |_child_ctx| {
            e2.lock().unwrap().push("child".to_string());
        });
    });

    q.wait();
    q.close();

    assert_eq!(executed.lock().unwrap().len(), 2);
}

// queue_test.go:77 TestQueue/ClosedQueueRejectsNewTasks
#[test]
fn queue_closed_queue_rejects_new_tasks() {
    let q = new_queue();
    q.close();

    let executed = Arc::new(AtomicBool::new(false));
    let e = executed.clone();
    q.enqueue(&Context::background(), move |_ctx| {
        e.store(true, Ordering::SeqCst);
    });

    q.wait();

    assert!(!executed.load(Ordering::SeqCst), "Task should not execute after queue is closed");
}
