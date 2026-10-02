// eventlist_test.go: unit tests for eventList coalescing and drain semantics.

use crate::event::{eventList, EventKind};
use crate::watcher::Error;

// eventlist_test.go:19
#[test]
fn event_list_create_then_delete() {
    let el = eventList::default();
    el.create("a");
    el.remove("a");
    assert_eq!(el.size(), 1, "size after create+remove");
    assert!(el.get_events().is_empty(), "getEvents should drop create+delete");
}

// eventlist_test.go:32
#[test]
fn event_list_delete_then_create() {
    let el = eventList::default();
    el.remove("a");
    el.create("a");
    let got = el.get_events();
    assert_eq!(got.len(), 1);
    // "Assume update event when rapidly removed and created".
    assert_eq!(got[0].kind, EventKind::Update);
}

// eventlist_test.go:47
#[test]
fn event_list_create_delete_create() {
    let el = eventList::default();
    el.create("a");
    el.remove("a");
    el.create("a");
    let got = el.get_events();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].kind, EventKind::Update, "create+delete+create should coalesce to update");
}

// eventlist_test.go:62
#[test]
fn event_list_error_is_latched_and_cleared() {
    let el = eventList::default();
    assert!(!el.has_error(), "fresh eventList should have no error");
    assert!(el.get_error().is_none());
    el.set_error(Error::new("first"));
    el.set_error(Error::new("second")); // only first wins
    assert!(el.has_error());
    assert_eq!(el.get_error().unwrap().message(), "first");
    let _ = el.drain();
    assert!(!el.has_error(), "clear should drop the error");
    assert!(el.get_error().is_none());
}

// eventlist_test.go:87
#[test]
fn event_list_drain_is_atomic() {
    let el = eventList::default();
    el.create("a");
    el.update("b");
    el.set_error(Error::new("oops"));

    let (events, err) = el.drain();
    assert!(err.is_some(), "drain should return the error");
    assert_eq!(events.len(), 2);

    let (events2, err2) = el.drain();
    assert!(err2.is_none());
    assert!(events2.is_empty());
}

// eventlist_test.go:110
#[test]
fn event_list_drain_returns_error_with_events() {
    let el = eventList::default();
    el.create("file.txt");
    el.set_error(Error::new("overflow"));

    let (events, err) = el.drain();
    assert!(err.is_some());
    assert_eq!(events.len(), 1, "expected 1 event alongside error");
}

// eventlist_test.go:125
#[test]
fn event_list_drain_for_sequences() {
    let el = eventList::default();
    el.create("file.txt");
    let start_after_create = el.sequence();
    el.remove("file.txt");

    let (events_by_callback, err) = el.drain_for_sequences(&[0, start_after_create]);
    assert!(err.is_none());
    assert!(events_by_callback[0].is_empty(), "create+delete should cancel for original callback");
    assert_eq!(events_by_callback[1].len(), 1);
    let got = &events_by_callback[1][0];
    assert!(got.kind == EventKind::Delete && got.path == "file.txt");
}
