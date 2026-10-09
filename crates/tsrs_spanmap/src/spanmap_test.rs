// spanmap/spanmap_test.go. Go's subtests run as loops; BenchmarkOriginalToVirtualPositionNearEnd is a benchmark,
// not a test, and is not ported.

use tsrs_core::{new_text_range, TextPos};

use crate::*;

fn segment(virtual_start: TextPos, virtual_end: TextPos, original_start: TextPos, original_end: TextPos, kind: Kind, features: Feature) -> Segment {
    Segment { virtual_start, virtual_end, original_start, original_end, kind, features }
}

// spanmap_test.go:11
#[test]
fn test_virtual_to_original_span_verbatim() {
    // Virtual [0,10) is a verbatim copy of original [100,110).
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::All)]);

    let (got, fidelity) = m.virtual_to_original_span(new_text_range(3, 7));
    assert_eq!(got.pos(), 103);
    assert_eq!(got.end(), 107);
    assert_eq!(fidelity, Fidelity::Exact);
}

// spanmap_test.go:25
#[test]
fn test_virtual_to_original_span_atom() {
    // Virtual [0,3) is a synthesized gap; [3,14) ("MyComponent") is an atom of the original [60,71).
    let m = new(&[segment(3, 14, 60, 71, Kind::Atom, Feature::All)]);

    // A span inside the atom maps to the whole atom span.
    let (got, fidelity) = m.virtual_to_original_span(new_text_range(5, 9));
    assert_eq!(got.pos(), 60);
    assert_eq!(got.end(), 71);
    assert_eq!(fidelity, Fidelity::Atom);
}

// spanmap_test.go:40
#[test]
fn test_virtual_alias() {
    let m = new(&[segment(3, 6, 10, 11, Kind::Alias, Feature::All)]);

    let (got, fidelity) = m.virtual_to_original_span(new_text_range(3, 6));
    assert_eq!(got, new_text_range(10, 11));
    assert_eq!(fidelity, Fidelity::Atom);
    let alias = m.alias_for_virtual_span(new_text_range(3, 6));
    assert!(alias.is_some());
    assert_eq!(alias.unwrap().kind, Kind::Alias);
    let partial = m.alias_for_virtual_span(new_text_range(4, 6));
    assert!(partial.is_none());

    let data = m.marshal().unwrap();
    let decoded = unmarshal(&data).unwrap();
    assert_eq!(decoded.segments()[0].kind, Kind::Alias);
}

// spanmap_test.go:63
#[test]
fn test_virtual_to_original_span_synthesized_gap() {
    // A gap between two verbatim segments is synthesized: it maps to the insertion point (the preceding
    // segment's original end) with no fidelity.
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::All), segment(20, 30, 200, 210, Kind::Verbatim, Feature::All)]);

    let (got, fidelity) = m.virtual_to_original_span(new_text_range(12, 15));
    assert_eq!(got.pos(), 110);
    assert_eq!(got.end(), 110);
    assert_eq!(fidelity, Fidelity::None);
}

// spanmap_test.go:79
#[test]
fn test_original_to_virtual_intersecting_spans_allows_uncovered_endpoints() {
    let m = new(&[segment(10, 20, 100, 110, Kind::Verbatim, Feature::SemanticTokens | Feature::InlayHints)]);

    for feature in [Feature::SemanticTokens, Feature::InlayHints] {
        let got = m.original_to_virtual_intersecting_spans(new_text_range(90, 120), feature);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].span, new_text_range(10, 20));
        assert_eq!(got[0].fidelity, Fidelity::Exact);
    }
}

// spanmap_test.go:95
#[test]
fn test_virtual_to_original_span_empty_is_synthesized() {
    // An empty map describes fully synthesized output: everything maps to the start with no fidelity.
    let m = new(&[]);
    let (got, fidelity) = m.virtual_to_original_span(new_text_range(5, 10));
    assert_eq!(got.pos(), 0);
    assert_eq!(got.end(), 0);
    assert_eq!(fidelity, Fidelity::None);
}

// spanmap_test.go:106
#[test]
fn test_virtual_to_original_span_crossing_segments() {
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::None), segment(10, 20, 200, 210, Kind::Verbatim, Feature::None)]);

    let (got, fidelity) = m.virtual_to_original_span(new_text_range(5, 15));
    assert_eq!(got.pos(), 105);
    assert_eq!(got.end(), 205);
    assert_eq!(fidelity, Fidelity::Approximate);
}

// spanmap_test.go:120
#[test]
fn test_virtual_to_original_span_nil_identity() {
    let (got, fidelity) = virtual_to_original_span(None, new_text_range(3, 7));
    assert_eq!(got.pos(), 3);
    assert_eq!(got.end(), 7);
    assert_eq!(fidelity, Fidelity::Exact);
}

// spanmap_test.go:130
#[test]
fn test_virtual_to_original_position() {
    // Virtual [0,10) is a verbatim copy of original [100,110); [10,20) is an atom of original [200,210).
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::All), segment(20, 30, 200, 210, Kind::Atom, Feature::All)]);

    struct Case {
        name: &'static str,
        pos: TextPos,
        want: TextPos,
        fidelity: Fidelity,
    }
    let test_cases = [
        Case { name: "verbatim interpolates", pos: 3, want: 103, fidelity: Fidelity::Exact },
        Case { name: "atom maps to its start", pos: 25, want: 200, fidelity: Fidelity::Atom },
        Case { name: "gap maps to insertion point", pos: 15, want: 110, fidelity: Fidelity::None },
    ];
    for tc in &test_cases {
        let (got, fidelity) = m.virtual_to_original_position(tc.pos);
        assert_eq!(got, tc.want, "{}", tc.name);
        assert_eq!(fidelity, tc.fidelity, "{}", tc.name);
        // VirtualToOriginalPosition must agree with VirtualToOriginalSpan on a zero-length range.
        let (span, span_fidelity) = m.virtual_to_original_span(new_text_range(tc.pos, tc.pos));
        assert_eq!(got, span.pos(), "{}", tc.name);
        assert_eq!(fidelity, span_fidelity, "{}", tc.name);
    }
}

// spanmap_test.go:163
#[test]
fn test_virtual_to_original_position_exact() {
    let m = new(&[
        segment(0, 10, 100, 110, Kind::Verbatim, Feature::All),
        segment(10, 20, 110, 120, Kind::Atom, Feature::All),
        segment(20, 30, 120, 130, Kind::Verbatim, Feature::All),
    ]);

    for (pos, want, ok) in [(5, 105, true), (10, 110, false), (15, 110, false), (20, 120, false), (25, 125, true)] {
        let (got, got_ok) = m.virtual_to_original_position_exact(pos);
        assert_eq!(got, want, "pos {pos}");
        assert_eq!(got_ok, ok, "pos {pos}");
    }
}

// spanmap_test.go:189
#[test]
fn test_virtual_to_original_position_exact_rejects_discontinuous_boundary() {
    let m = new(&[segment(0, 10, 0, 10, Kind::Verbatim, Feature::None), segment(10, 20, 100, 110, Kind::Verbatim, Feature::None)]);

    let (mapped, ok) = m.virtual_to_original_position_exact(10);
    assert_eq!(mapped, 100);
    assert!(!ok);
}

// spanmap_test.go:202
#[test]
fn test_zero_length_spans_at_segment_ends() {
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::Hover), segment(20, 30, 200, 210, Kind::Verbatim, Feature::Hover)]);

    let (position, fidelity) = m.virtual_to_original_position(30);
    assert_eq!(position, 210);
    assert_eq!(fidelity, Fidelity::Exact);
    let (virtual_span, fidelity) = m.virtual_to_original_span(new_text_range(30, 30));
    assert_eq!(virtual_span, new_text_range(210, 210));
    assert_eq!(fidelity, Fidelity::Exact);

    for (name, original_end) in [("before gap", 110), ("final", 210)] {
        let positions = m.original_to_virtual_positions(original_end, Feature::Hover);
        let spans = m.original_to_virtual_spans(new_text_range(original_end, original_end), Feature::Hover);
        assert_eq!(positions.len(), 1, "{name}");
        assert_eq!(spans.len(), 1, "{name}");
        assert_eq!(spans[0].span, new_text_range(positions[0].position, positions[0].position), "{name}");
        assert_eq!(spans[0].fidelity, positions[0].fidelity, "{name}");
    }
}

// spanmap_test.go:236
#[test]
fn test_map_position_nil_identity() {
    let (got, fidelity) = virtual_to_original_position(None, 7);
    assert_eq!(got, 7);
    assert_eq!(fidelity, Fidelity::Exact);
}

// spanmap_test.go:245
#[test]
fn test_original_to_virtual_span_verbatim() {
    // Virtual [0,10) is a verbatim copy of original [100,110).
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::All)]);

    let results = m.original_to_virtual_spans(new_text_range(103, 107), Feature::All);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].span.pos(), 3);
    assert_eq!(results[0].span.end(), 7);
    assert_eq!(results[0].fidelity, Fidelity::Exact);
}

// spanmap_test.go:260
#[test]
fn test_original_to_virtual_span_atom() {
    // Virtual [3,14) is an atom of the original [60,71).
    let m = new(&[segment(3, 14, 60, 71, Kind::Atom, Feature::All)]);

    // A span inside the original atom maps to the whole virtual span.
    let results = m.original_to_virtual_spans(new_text_range(63, 67), Feature::All);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].span.pos(), 3);
    assert_eq!(results[0].span.end(), 14);
    assert_eq!(results[0].fidelity, Fidelity::Atom);
}

// spanmap_test.go:276
#[test]
fn test_original_to_virtual_span_gap() {
    // An original range with no covering segment has no virtual counterpart.
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::All), segment(20, 30, 200, 210, Kind::Verbatim, Feature::All)]);

    assert_eq!(m.original_to_virtual_spans(new_text_range(150, 160), Feature::All).len(), 0);
}

// spanmap_test.go:288
#[test]
fn test_original_to_virtual_span_nil_identity() {
    let results = original_to_virtual_spans(None, new_text_range(3, 7), Feature::All);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].span.pos(), 3);
    assert_eq!(results[0].span.end(), 7);
    assert_eq!(results[0].fidelity, Fidelity::Exact);
}

// spanmap_test.go:299
#[test]
fn test_original_to_virtual_positions() {
    // Original [100,110) is a verbatim copy of virtual [0,10); [200,210) is an atom of virtual [20,30).
    let m = new(&[segment(0, 10, 100, 110, Kind::Verbatim, Feature::All), segment(20, 30, 200, 210, Kind::Atom, Feature::All)]);

    struct Case {
        name: &'static str,
        pos: TextPos,
        want: TextPos,
        fidelity: Fidelity,
    }
    let test_cases = [
        Case { name: "verbatim interpolates", pos: 103, want: 3, fidelity: Fidelity::Exact },
        Case { name: "atom maps to its start", pos: 205, want: 20, fidelity: Fidelity::Atom },
        Case { name: "gap has no projection", pos: 150, want: 0, fidelity: Fidelity::None },
    ];
    for tc in &test_cases {
        let positions = m.original_to_virtual_positions(tc.pos, Feature::All);
        let spans = m.original_to_virtual_spans(new_text_range(tc.pos, tc.pos), Feature::All);
        if tc.fidelity.is_none() {
            assert_eq!(positions.len(), 0, "{}", tc.name);
            assert_eq!(spans.len(), 0, "{}", tc.name);
            continue;
        }
        assert_eq!(positions.len(), 1, "{}", tc.name);
        assert_eq!(positions[0].position, tc.want, "{}", tc.name);
        assert_eq!(positions[0].fidelity, tc.fidelity, "{}", tc.name);
        assert_eq!(spans.len(), 1, "{}", tc.name);
        assert_eq!(spans[0].span.pos(), tc.want, "{}", tc.name);
        assert_eq!(spans[0].fidelity, tc.fidelity, "{}", tc.name);
    }
}

// spanmap_test.go:338
#[test]
fn test_original_to_virtual_positions_at_endpoint() {
    let m = new(&[
        segment(2, 5, 10, 13, Kind::Verbatim, Feature::Completion),
        segment(8, 11, 13, 16, Kind::Verbatim, Feature::Completion),
        segment(20, 23, 30, 35, Kind::Atom, Feature::Completion),
    ]);

    assert_eq!(
        m.original_to_virtual_positions(13, Feature::Completion),
        vec![MappedPosition { position: 5, fidelity: Fidelity::Exact }, MappedPosition { position: 8, fidelity: Fidelity::Exact }]
    );
    assert_eq!(m.original_to_virtual_positions(35, Feature::Completion), vec![MappedPosition { position: 23, fidelity: Fidelity::Atom }]);

    let filtered = new(&[segment(20, 23, 10, 13, Kind::Verbatim, Feature::Hover), segment(2, 5, 13, 16, Kind::Verbatim, Feature::Completion)]);
    assert_eq!(
        filtered.original_to_virtual_positions(13, Feature::All),
        vec![MappedPosition { position: 2, fidelity: Fidelity::Exact }, MappedPosition { position: 23, fidelity: Fidelity::Exact }]
    );
    assert_eq!(filtered.original_to_virtual_positions(13, Feature::Completion), vec![MappedPosition { position: 2, fidelity: Fidelity::Exact }]);
}

// spanmap_test.go:368
#[test]
fn test_original_to_virtual_duplicate_group() {
    let m = new(&[
        segment(0, 3, 10, 13, Kind::Verbatim, Feature::Definition),
        segment(10, 13, 10, 13, Kind::Verbatim, Feature::Hover),
        segment(20, 25, 10, 13, Kind::Atom, Feature::Definition),
    ]);

    let semantic = m.original_to_virtual_positions(11, Feature::Hover);
    assert_eq!(semantic.len(), 1);
    assert_eq!(semantic[0].position, 11);
    assert_eq!(semantic[0].fidelity, Fidelity::Exact);

    let navigation = m.original_to_virtual_positions(11, Feature::Definition);
    assert_eq!(navigation.len(), 2);
    assert_eq!(navigation[0].position, 1);
    assert_eq!(navigation[0].fidelity, Fidelity::Exact);
    assert_eq!(navigation[1].position, 20);
    assert_eq!(navigation[1].fidelity, Fidelity::Atom);

    let spans = m.original_to_virtual_spans(new_text_range(10, 13), Feature::Definition);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].span.pos(), 0);
    assert_eq!(spans[0].span.end(), 3);
    assert_eq!(spans[1].span.pos(), 20);
    assert_eq!(spans[1].span.end(), 25);
}

// spanmap_test.go:397
#[test]
fn test_original_to_virtual_overlapping_spans() {
    let m = new(&[
        segment(0, 6, 0, 6, Kind::Verbatim, Feature::Hover),
        segment(10, 12, 2, 4, Kind::Verbatim, Feature::Hover),
        segment(20, 24, 3, 7, Kind::Verbatim, Feature::Hover),
    ]);

    assert_eq!(
        m.original_to_virtual_positions(3, Feature::Hover),
        vec![
            MappedPosition { position: 3, fidelity: Fidelity::Exact },
            MappedPosition { position: 11, fidelity: Fidelity::Exact },
            MappedPosition { position: 20, fidelity: Fidelity::Exact },
        ]
    );
    let spans = m.original_to_virtual_spans(new_text_range(3, 4), Feature::Hover);
    let want_spans = [
        MappedSpan { span: new_text_range(3, 4), fidelity: Fidelity::Exact },
        MappedSpan { span: new_text_range(11, 12), fidelity: Fidelity::Exact },
        MappedSpan { span: new_text_range(20, 21), fidelity: Fidelity::Exact },
    ];
    assert_eq!(spans.len(), want_spans.len());
    for i in 0..spans.len() {
        assert_eq!(spans[i], want_spans[i]);
    }
}

// spanmap_test.go:423
#[test]
fn test_original_to_virtual_position_finds_early_covering_segment() {
    // Binary search lands near [90,95), which does not contain 97. The interval index must still find the
    // earlier [0,100) segment without scanning every segment whose start precedes the query.
    let m = new(&[
        segment(0, 100, 0, 100, Kind::Verbatim, Feature::Hover),
        segment(100, 105, 80, 85, Kind::Verbatim, Feature::Hover),
        segment(105, 110, 90, 95, Kind::Verbatim, Feature::Hover),
        segment(110, 113, 100, 103, Kind::Verbatim, Feature::Hover),
    ]);

    assert_eq!(m.original_to_virtual_positions(97, Feature::Hover), vec![MappedPosition { position: 97, fidelity: Fidelity::Exact }]);
    let spans = m.original_to_virtual_spans(new_text_range(97, 98), Feature::Hover);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0], MappedSpan { span: new_text_range(97, 98), fidelity: Fidelity::Exact });

    // Point lookup includes both sides of a shared endpoint, including an early interval found through the
    // max-end tree. Nonempty span lookup treats segment ends as exclusive and uses only the right segment.
    assert_eq!(
        m.original_to_virtual_positions(100, Feature::Hover),
        vec![MappedPosition { position: 100, fidelity: Fidelity::Exact }, MappedPosition { position: 110, fidelity: Fidelity::Exact }]
    );
    let spans = m.original_to_virtual_spans(new_text_range(100, 101), Feature::Hover);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0], MappedSpan { span: new_text_range(110, 111), fidelity: Fidelity::Exact });
}

// spanmap_test.go:473
#[test]
fn test_original_to_virtual_overlap_falls_back_from_disabled_container() {
    let m = new(&[
        segment(0, 6, 0, 6, Kind::Verbatim, Feature::Definition),
        segment(10, 13, 0, 3, Kind::Verbatim, Feature::Hover),
        segment(13, 16, 3, 6, Kind::Verbatim, Feature::Hover),
    ]);

    let spans = m.original_to_virtual_spans(new_text_range(1, 5), Feature::Hover);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0], MappedSpan { span: new_text_range(11, 15), fidelity: Fidelity::Approximate });
}

// spanmap_test.go:487
#[test]
fn test_original_to_virtual_cross_group_projections() {
    let m = new(&[
        segment(0, 2, 0, 2, Kind::Verbatim, Feature::Hover),
        segment(2, 4, 2, 4, Kind::Verbatim, Feature::Hover),
        segment(10, 12, 0, 2, Kind::Verbatim, Feature::Hover),
        segment(12, 14, 2, 4, Kind::Verbatim, Feature::Hover),
    ]);

    let spans = m.original_to_virtual_spans(new_text_range(1, 3), Feature::Hover);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].span, new_text_range(1, 3));
    assert_eq!(spans[1].span, new_text_range(11, 13));
    for mapped in &spans {
        assert_eq!(mapped.fidelity, Fidelity::Approximate);
    }
}

// spanmap_test.go:506
#[test]
fn test_original_to_virtual_explicit_zero_features() {
    let m = new(&[segment(0, 3, 10, 13, Kind::Verbatim, Feature::None)]);

    assert_eq!(m.original_to_virtual_positions(11, Feature::Hover).len(), 0);
    assert_eq!(m.original_to_virtual_positions(11, Feature::Definition).len(), 0);
    assert_eq!(m.original_to_virtual_spans(new_text_range(10, 13), Feature::Hover).len(), 0);

    let data = m.marshal().unwrap();
    assert_eq!(String::from_utf8(data.clone()).unwrap(), "[[0,3,10,3,0,0]]");
    let decoded = unmarshal(&data).unwrap();
    let segments = decoded.segments();
    assert_eq!(segments[0].features, Feature::None);

    let legacy = unmarshal(b"[[0,3,10,3,0]]").unwrap();
    assert_eq!(legacy.segments()[0].features, Feature::All);
    assert_eq!(legacy.original_to_virtual_positions(11, Feature::Hover).len(), 1);
}

// spanmap_test.go:531
#[test]
fn test_feature_participation_original_and_virtual() {
    let m = new(&[segment(0, 3, 10, 13, Kind::Verbatim, Feature::Hover), segment(3, 6, 20, 23, Kind::Verbatim, Feature::Completion)]);

    assert_eq!(m.original_to_virtual_positions(11, Feature::Hover).len(), 1);
    assert_eq!(m.original_to_virtual_positions(11, Feature::Completion).len(), 0);

    let (mapped, fidelity) = m.virtual_to_original_span_for_feature(new_text_range(0, 3), Feature::Hover);
    assert_eq!(mapped, new_text_range(10, 13));
    assert_eq!(fidelity, Fidelity::Exact);
    let (_, fidelity) = m.virtual_to_original_span_for_feature(new_text_range(0, 3), Feature::Completion);
    assert_eq!(fidelity, Fidelity::None);

    // Diagnostics and edit safety use unfiltered geometry and cannot be disabled by feature flags.
    let (mapped, fidelity) = m.virtual_to_original_span(new_text_range(0, 3));
    assert_eq!(mapped, new_text_range(10, 13));
    assert_eq!(fidelity, Fidelity::Exact);
}

// spanmap_test.go:553
#[test]
fn test_original_to_virtual_span_round_trip() {
    // Original spans are out of order relative to virtual spans, exercising the reverse index.
    let m = new(&[segment(0, 10, 200, 210, Kind::Verbatim, Feature::All), segment(10, 20, 100, 110, Kind::Verbatim, Feature::All)]);

    for r in [new_text_range(2, 8), new_text_range(12, 18)] {
        let (orig, fidelity) = m.virtual_to_original_span(r);
        assert_eq!(fidelity, Fidelity::Exact);
        let back = m.original_to_virtual_spans(orig, Feature::All);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].fidelity, Fidelity::Exact);
        assert_eq!(back[0].span.pos(), r.pos());
        assert_eq!(back[0].span.end(), r.end());
    }
}

// spanmap_test.go:573
#[test]
fn test_marshal_round_trip() {
    let original = new(&[segment(3, 14, 60, 71, Kind::Atom, Feature::None), segment(14, 24, 71, 81, Kind::Verbatim, Feature::None)]);

    let data = original.marshal().unwrap();
    let decoded = unmarshal(&data).unwrap();

    for r in [new_text_range(1, 2), new_text_range(4, 10), new_text_range(16, 20)] {
        let (want_range, want_fidelity) = original.virtual_to_original_span(r);
        let (got_range, got_fidelity) = decoded.virtual_to_original_span(r);
        assert_eq!(got_range, want_range);
        assert_eq!(got_fidelity, want_fidelity);
    }
}

// spanmap_test.go:594
#[test]
fn test_validate() {
    const TRANSFORMED: &str = "const greeting = 1;\n";
    const ORIGINAL: &str = "<x>const greeting = 1;\n</x>";
    let script_start = 3; // index of "const" in original
    let transformed_len = TRANSFORMED.len() as TextPos;
    let original_len = ORIGINAL.len() as TextPos;

    struct Case {
        name: &'static str,
        segs: Vec<Segment>,
        want_kind: Option<MappingErrorKind>,
    }
    let test_cases = [
        Case {
            name: "valid verbatim",
            segs: vec![segment(0, transformed_len, script_start, script_start + transformed_len, Kind::Verbatim, Feature::None)],
            want_kind: None,
        },
        Case { name: "empty is valid", segs: vec![], want_kind: None },
        Case { name: "gap is allowed", segs: vec![segment(3, transformed_len, 0, 0, Kind::Atom, Feature::None)], want_kind: None },
        Case {
            name: "overlap",
            segs: vec![segment(0, 10, 0, 0, Kind::Atom, Feature::None), segment(5, transformed_len, 0, 0, Kind::Atom, Feature::None)],
            want_kind: Some(MappingErrorKind::Overlap),
        },
        Case {
            name: "original out of bounds",
            segs: vec![segment(0, transformed_len, 0, original_len + 10, Kind::Atom, Feature::None)],
            want_kind: Some(MappingErrorKind::OutOfBounds),
        },
        Case {
            name: "verbatim text mismatch",
            segs: vec![segment(0, transformed_len, 0, transformed_len, Kind::Verbatim, Feature::None)],
            want_kind: Some(MappingErrorKind::VerbatimMismatch),
        },
        Case { name: "unknown kind", segs: vec![segment(0, 1, 0, 1, Kind(3), Feature::None)], want_kind: Some(MappingErrorKind::Kind) },
    ];

    for tc in &test_cases {
        let problem = new(&tc.segs).validate(TRANSFORMED, ORIGINAL);
        match tc.want_kind {
            None => assert!(problem.is_none(), "{}: expected valid, got {:?}", tc.name, problem),
            Some(want_kind) => {
                assert!(problem.is_some(), "{}: expected a problem", tc.name);
                assert_eq!(problem.unwrap().kind, want_kind, "{}", tc.name);
            }
        }
    }
}

// spanmap_test.go:661
#[test]
fn test_validate_original_overlap_and_features() {
    struct Case {
        name: &'static str,
        segments: Vec<Segment>,
        want_kind: Option<MappingErrorKind>,
    }
    let tests = [
        Case {
            name: "identical duplicate group",
            segments: vec![segment(0, 3, 0, 3, Kind::Verbatim, Feature::Definition), segment(3, 6, 0, 3, Kind::Verbatim, Feature::Hover)],
            want_kind: None,
        },
        Case {
            name: "partial original overlap is valid",
            segments: vec![segment(0, 3, 0, 3, Kind::Atom, Feature::None), segment(3, 6, 2, 5, Kind::Atom, Feature::None)],
            want_kind: None,
        },
        Case {
            name: "nested original overlap is valid",
            segments: vec![segment(0, 5, 0, 5, Kind::Atom, Feature::None), segment(5, 6, 1, 4, Kind::Atom, Feature::None)],
            want_kind: None,
        },
        Case {
            name: "duplicate without explicit features is tolerant",
            segments: vec![segment(0, 3, 0, 3, Kind::Atom, Feature::None), segment(3, 6, 0, 3, Kind::Atom, Feature::Definition)],
            want_kind: None,
        },
        Case {
            name: "duplicate with shared feature members is tolerant",
            segments: vec![
                segment(0, 3, 0, 3, Kind::Atom, Feature::Hover),
                segment(3, 6, 0, 3, Kind::Atom, Feature::Hover | Feature::Definition),
            ],
            want_kind: None,
        },
        Case {
            name: "features on sole cover are valid",
            segments: vec![segment(0, 3, 0, 3, Kind::Atom, Feature::Definition)],
            want_kind: None,
        },
        Case {
            name: "unknown feature flag",
            segments: vec![segment(0, 3, 0, 3, Kind::Atom, Feature::from_bits_retain(1 << 22))],
            want_kind: Some(MappingErrorKind::Feature),
        },
    ];

    for test in &tests {
        let problem = new(&test.segments).validate("abcabc", "abcdef");
        match test.want_kind {
            None => assert!(problem.is_none(), "{}: expected valid, got {:?}", test.name, problem),
            Some(want_kind) => {
                assert!(problem.is_some(), "{}", test.name);
                assert_eq!(problem.unwrap().kind, want_kind, "{}", test.name);
            }
        }
    }
}

// spanmap_test.go:740
#[test]
fn test_validate_nil_is_valid() {
    assert!(validate(None, "abc", "abc").is_none());
}
