use tsrs_lsproto as lsproto;

use super::trackerimpl::text_edits_conflict;

// trackerimpl_test.go:9
#[test]
fn test_text_edits_conflict_at_same_insertion_point_across_projections() {
    let position = lsproto::Position { line: 1, character: 2 };
    let a = lsproto::TextEdit { range: lsproto::Range { start: position, end: position }, new_text: "a".to_string() };
    let b = lsproto::TextEdit { range: lsproto::Range { start: position, end: position }, new_text: "b".to_string() };

    if !text_edits_conflict(&a, &b, true) {
        panic!("different insertions at the same position across projections should conflict");
    }
    if text_edits_conflict(&a, &b, false) {
        panic!("insertions at the same position within one projection should retain their existing ordering");
    }
}
