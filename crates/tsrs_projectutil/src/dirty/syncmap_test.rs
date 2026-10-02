use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::*;

// testValue is a simple cloneable type for testing
#[derive(Clone, Debug)]
struct testValue {
    data: String,
}

impl Cloneable for testValue {
    fn clone_value(&self) -> testValue {
        testValue { data: self.data.clone() }
    }
}

fn base_of(entries: &[(&str, &str)]) -> SharedMap<String, testValue> {
    let mut m = FxHashMap::default();
    for (k, v) in entries {
        m.insert(k.to_string(), Shared::new(testValue { data: v.to_string() }));
    }
    Arc::new(m)
}

fn key(s: &str) -> String {
    s.to_string()
}

// syncmap_test.go:19 TestSyncMapProxyFor/proxy for race condition
#[test]
fn sync_map_proxy_for_race_condition() {
    // Create a sync map with a base value
    let sync_map = Arc::new(new_sync_map(base_of(&[("key1", "original")])));

    // Load the same entry from multiple threads to simulate race condition
    let (entry1, entry2) = std::thread::scope(|s| {
        let h1 = s.spawn(|| sync_map.load(&key("key1")).expect("entry1 should be loaded"));
        let h2 = s.spawn(|| sync_map.load(&key("key1")).expect("entry2 should be loaded"));
        (h1.join().unwrap(), h2.join().unwrap())
    });

    // Both entries should exist and have the same initial value
    assert_eq!("original", entry1.value().unwrap().data);
    assert_eq!("original", entry2.value().unwrap().data);
    assert!(!entry1.dirty());
    assert!(!entry2.dirty());

    // Now try to change both entries concurrently to trigger the proxy mechanism.
    std::thread::scope(|s| {
        s.spawn(|| entry1.change(|v| v.data = "changed_by_entry1".to_string()));
        s.spawn(|| entry2.change(|v| v.data = "changed_by_entry2".to_string()));
    });

    // After the race, one entry should have proxyFor set and both should reflect the same final state
    let final_value1 = entry1.value().unwrap().data.clone();
    let final_value2 = entry2.value().unwrap().data.clone();
    assert_eq!(final_value1, final_value2, "both entries should have the same final value");

    // Both entries should be marked as dirty
    assert!(entry1.dirty());
    assert!(entry2.dirty());

    // At least one entry should have proxyFor set (the one that lost the race)
    let has_proxy = entry1.proxy_for().is_some() || entry2.proxy_for().is_some();
    assert!(has_proxy, "at least one entry should have proxyFor set");

    // If entry1 has a proxy, it should point to entry2, and vice versa
    if let Some(p) = entry1.proxy_for() {
        assert!(Arc::ptr_eq(&p, &entry2), "entry1 should proxy to entry2");
    }
    if let Some(p) = entry2.proxy_for() {
        assert!(Arc::ptr_eq(&p, &entry1), "entry2 should proxy to entry1");
    }
}

// syncmap_test.go:105 TestSyncMapProxyFor/proxy operations delegation
#[test]
fn sync_map_proxy_operations_delegation() {
    let sync_map = new_sync_map(base_of(&[("key1", "original")]));

    // Load two entries for the same key
    let entry1 = sync_map.load(&key("key1")).unwrap();
    let entry2 = sync_map.load(&key("key1")).unwrap();

    // Force one to become a proxy by making them both dirty in sequence
    entry1.change(|v| v.data = "changed_by_entry1".to_string());
    entry2.change(|v| v.data = "changed_by_entry2".to_string());

    // Determine which is the proxy and which is the target
    let (proxy, target) = if entry1.proxy_for().is_some() { (entry1, entry2) } else { (entry2, entry1) };

    // Test that proxy operations are delegated to the target
    // Change through proxy should affect target
    proxy.change(|v| v.data = "changed_through_proxy".to_string());
    assert_eq!("changed_through_proxy", target.value().unwrap().data);
    assert_eq!("changed_through_proxy", proxy.value().unwrap().data);

    // ChangeIf through proxy should work
    let changed = proxy.change_if(|v| v.data == "changed_through_proxy", |v| v.data = "conditional_change".to_string());
    assert!(changed);
    assert_eq!("conditional_change", target.value().unwrap().data);
    assert_eq!("conditional_change", proxy.value().unwrap().data);

    // Dirty status should be consistent
    assert_eq!(target.dirty(), proxy.dirty());

    // Locked operations should work through proxy
    proxy.locked(&mut |v: &dyn Value<testValue>| {
        v.change(&mut |val: &mut testValue| val.data = "locked_change".to_string());
    });
    assert_eq!("locked_change", target.value().unwrap().data);
    assert_eq!("locked_change", proxy.value().unwrap().data);
}

// syncmap_test.go:167 TestSyncMapProxyFor/proxy delete operations
#[test]
fn sync_map_proxy_delete_operations() {
    let sync_map = new_sync_map(base_of(&[("key1", "original")]));

    // Load two entries and make one a proxy
    let entry1 = sync_map.load(&key("key1")).unwrap();
    let entry2 = sync_map.load(&key("key1")).unwrap();

    entry1.change(|v| v.data = "modified".to_string());
    entry2.change(|v| v.data = "modified2".to_string());

    // Determine which is the proxy
    let proxy = if entry1.proxy_for().is_some() { entry1 } else { entry2 };

    // Delete through proxy should affect target
    proxy.delete();

    // Both should reflect the deletion
    assert!(sync_map.load(&key("key1")).is_none(), "key should be deleted from sync map");

    // DeleteIf through proxy should work
    let sync_map2 = new_sync_map(base_of(&[("key2", "test")]));

    let entry3 = sync_map2.load(&key("key2")).unwrap();
    let entry4 = sync_map2.load(&key("key2")).unwrap();

    entry3.change(|v| v.data = "modified".to_string());
    entry4.change(|v| v.data = "modified2".to_string());

    let proxy2 = if entry3.proxy_for().is_some() { entry3 } else { entry4 };

    proxy2.delete_if(|v| v.data == "modified2" || v.data == "modified");

    assert!(sync_map2.load(&key("key2")).is_none(), "key2 should be deleted conditionally");
}

// syncmap_test.go:224 TestSyncMapProxyFor/no proxy when no race
#[test]
fn sync_map_no_proxy_when_no_race() {
    let sync_map = new_sync_map(base_of(&[("key1", "original")]));

    // Load and modify a single entry - no race condition
    let entry = sync_map.load(&key("key1")).unwrap();

    entry.change(|v| v.data = "changed".to_string());

    // Should not have a proxy since there was no race
    assert!(entry.proxy_for().is_none(), "entry should not have proxyFor when no race occurs");
    assert!(entry.dirty());
    assert_eq!("changed", entry.value().unwrap().data);
}
