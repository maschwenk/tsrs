// PLACEHOLDER for Go `internal/spanmap` (content mappers are out of scope, docs/LSP.md "Known gaps").
//
// No content mapper is ever registered, so no file has a span map: `SpanMap` is uninhabited and every
// `script.span_map()` is `None`. The value types the language service passes around (`Feature`, `Fidelity`,
// `MappedPosition`, `MappedSpan`) are ported as in Go so callers keep Go's branch structure; the `SpanMap`
// methods exist with Go's signatures and are statically unreachable.

use tsrs_core::{TextPos, TextRange};

bitflags::bitflags! {
    // spanmap.go:41
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct Feature: i32 {
        const Hover = 1 << 0;
        const SignatureHelp = 1 << 1;
        const Completion = 1 << 2;
        const Definition = 1 << 3;
        const TypeDefinition = 1 << 4;
        const Implementation = 1 << 5;
        const References = 1 << 6;
        const DocumentHighlights = 1 << 7;
        const Rename = 1 << 8;
        const CallHierarchy = 1 << 9;
        const CodeActions = 1 << 10;
        const Formatting = 1 << 11;
        const InlayHints = 1 << 12;
        const SemanticTokens = 1 << 13;
        const FoldingRanges = 1 << 14;
        const SelectionRanges = 1 << 15;
        const LinkedEditing = 1 << 16;
        const AutoInsert = 1 << 17;
        const DocumentSymbols = 1 << 18;
        const CodeLens = 1 << 19;
        const All = (1 << 20) - 1;
    }
}

impl Feature {
    pub const None: Feature = Feature::empty();
}

// spanmap.go:71
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum Fidelity {
    // FidelityExact means the span fell entirely within a single verbatim segment and maps precisely.
    #[default]
    Exact,
    // FidelityAtom means the span fell within a single atom segment and maps to that atom's span.
    Atom,
    // FidelityApproximate means the span crossed segment boundaries; its endpoints were mapped and clamped.
    Approximate,
    // FidelityNone means the span had no original counterpart (it was entirely synthesized).
    None,
}

impl Fidelity {
    // spanmap.go:86
    pub fn is_exact(self) -> bool {
        self == Fidelity::Exact
    }

    // spanmap.go:92
    pub fn is_single_segment(self) -> bool {
        self == Fidelity::Exact || self == Fidelity::Atom
    }

    // spanmap.go:98
    pub fn is_none(self) -> bool {
        self == Fidelity::None
    }
}

// spanmap.go:115
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MappedPosition {
    pub position: TextPos,
    pub fidelity: Fidelity,
}

// spanmap.go:121
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MappedSpan {
    pub span: TextRange,
    pub fidelity: Fidelity,
}

// spanmap.go:129 (uninhabited: no span map is ever built)
#[derive(Debug)]
pub enum SpanMap {}

impl SpanMap {
    // spanmap.go:241
    pub fn virtual_to_original_span(&self, _r: TextRange) -> (TextRange, Fidelity) {
        match *self {}
    }

    // spanmap.go:279
    pub fn virtual_to_original_span_for_feature(&self, _r: TextRange, _feature: Feature) -> (TextRange, Fidelity) {
        match *self {}
    }

    // spanmap.go:313
    pub fn virtual_to_original_position(&self, _pos: TextPos) -> (TextPos, Fidelity) {
        match *self {}
    }

    // spanmap.go:330
    pub fn virtual_to_original_position_exact(&self, _pos: TextPos) -> Option<TextPos> {
        match *self {}
    }

    // spanmap.go:350
    pub fn virtual_to_original_position_for_feature(&self, _pos: TextPos, _feature: Feature) -> (TextPos, Fidelity) {
        match *self {}
    }

    // spanmap.go:431
    pub fn original_to_virtual_positions(&self, _pos: TextPos, _feature: Feature) -> Vec<MappedPosition> {
        match *self {}
    }

    // spanmap.go:482
    pub fn original_to_virtual_spans(&self, _r: TextRange, _feature: Feature) -> Vec<MappedSpan> {
        match *self {}
    }

    // spanmap.go:539
    pub fn original_to_virtual_intersecting_spans(&self, _r: TextRange, _feature: Feature) -> Vec<MappedSpan> {
        match *self {}
    }
}
