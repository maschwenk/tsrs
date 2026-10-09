pub type TextPos = u32;

/// The largest byte position representable in source text. `u32::MAX` is kept for
/// nodes and ranges that do not have a position in the original source.
pub const MAX_TEXT_POS: TextPos = u32::MAX - 1;
pub const SYNTHETIC_POSITION: TextPos = u32::MAX;

#[inline]
pub fn text_pos_from_len(len: usize) -> TextPos {
    let pos = TextPos::try_from(len).expect("source text is longer than u32::MAX bytes");
    assert!(pos <= MAX_TEXT_POS, "source text is longer than {MAX_TEXT_POS} bytes");
    pos
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TextRange {
    pos: TextPos,
    end: TextPos,
}

impl TextRange {
    #[inline]
    pub const fn new(pos: TextPos, end: TextPos) -> TextRange {
        TextRange { pos, end }
    }

    #[inline]
    pub fn pos(self) -> TextPos {
        self.pos
    }

    #[inline]
    pub fn end(self) -> TextPos {
        self.end
    }

    #[inline]
    pub fn len(self) -> u32 {
        match (self.pos, self.end) {
            (SYNTHETIC_POSITION, SYNTHETIC_POSITION) => 0,
            (SYNTHETIC_POSITION, end) => end + 1,
            (_, SYNTHETIC_POSITION) => panic!("a real range start cannot have a synthetic end"),
            (pos, end) => end.checked_sub(pos).expect("a text range's end precedes its start"),
        }
    }

    pub fn is_valid(self) -> bool {
        self.pos != SYNTHETIC_POSITION || self.end != SYNTHETIC_POSITION
    }

    pub fn contains(self, pos: TextPos) -> bool {
        position_le(self.pos, pos) && position_lt(pos, self.end)
    }

    pub fn contains_inclusive(self, pos: TextPos) -> bool {
        position_le(self.pos, pos) && position_le(pos, self.end)
    }

    pub fn contains_exclusive(self, pos: TextPos) -> bool {
        position_lt(self.pos, pos) && position_lt(pos, self.end)
    }

    pub fn with_pos(self, pos: TextPos) -> TextRange {
        TextRange { pos, end: self.end }
    }

    pub fn with_end(self, end: TextPos) -> TextRange {
        TextRange { pos: self.pos, end }
    }

    pub fn contained_by(self, t2: TextRange) -> bool {
        position_le(t2.pos, self.pos) && position_le(self.end, t2.end)
    }

    pub fn overlaps(self, t2: TextRange) -> bool {
        let start = position_max(self.pos, t2.pos);
        let end = position_min(self.end, t2.end);
        position_lt(start, end)
    }

    // Similar to Overlaps, but treats touching ranges as intersecting.
    // For example, [0, 5) intersects [5, 10).
    pub fn intersects(self, t2: TextRange) -> bool {
        let start = position_max(self.pos, t2.pos);
        let end = position_min(self.end, t2.end);
        position_le(start, end)
    }
}

#[inline]
pub const fn new_text_range(pos: TextPos, end: TextPos) -> TextRange {
    TextRange { pos, end }
}

#[inline]
pub const fn undefined_text_range() -> TextRange {
    TextRange { pos: SYNTHETIC_POSITION, end: SYNTHETIC_POSITION }
}

pub fn compare_text_ranges(r1: TextRange, r2: TextRange) -> i32 {
    match position_cmp(r1.pos, r2.pos) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Equal => match position_cmp(r1.end, r2.end) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        },
    }
}

#[inline]
pub const fn position_is_synthetic(pos: TextPos) -> bool {
    pos == SYNTHETIC_POSITION
}

#[inline]
pub fn position_cmp(a: TextPos, b: TextPos) -> std::cmp::Ordering {
    match (position_is_synthetic(a), position_is_synthetic(b)) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => a.cmp(&b),
    }
}

#[inline]
fn position_lt(a: TextPos, b: TextPos) -> bool {
    position_cmp(a, b).is_lt()
}

#[inline]
fn position_le(a: TextPos, b: TextPos) -> bool {
    !position_cmp(a, b).is_gt()
}

#[inline]
fn position_min(a: TextPos, b: TextPos) -> TextPos {
    if position_le(a, b) { a } else { b }
}

#[inline]
fn position_max(a: TextPos, b: TextPos) -> TextPos {
    if position_le(a, b) { b } else { a }
}

const _: () = assert!(std::mem::size_of::<TextRange>() == 8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_cross_the_signed_boundary_and_reserve_the_sentinel() {
        let above_i32 = i32::MAX as TextPos + 1;
        let range = TextRange::new(i32::MAX as TextPos, above_i32);
        assert_eq!(range.len(), 1);
        assert!(range.contains(i32::MAX as TextPos));
        assert_eq!(MAX_TEXT_POS, u32::MAX - 1);
        assert!(position_cmp(SYNTHETIC_POSITION, 0).is_lt());
        assert_eq!(undefined_text_range().len(), 0);
        let mixed = TextRange::new(SYNTHETIC_POSITION, MAX_TEXT_POS);
        assert!(mixed.is_valid());
        assert_eq!(mixed.len(), u32::MAX);
        assert_eq!(std::mem::size_of::<TextRange>(), 8);
    }

    #[test]
    fn maximum_real_text_position_is_accepted() {
        assert_eq!(text_pos_from_len(MAX_TEXT_POS as usize), MAX_TEXT_POS);
    }
}
