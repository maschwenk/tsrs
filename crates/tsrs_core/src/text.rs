pub type TextPos = i32;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TextRange {
    pos: TextPos,
    end: TextPos,
}

impl TextRange {
    #[inline]
    pub const fn new(pos: i32, end: i32) -> TextRange {
        TextRange { pos, end }
    }

    #[inline]
    pub fn pos(self) -> i32 {
        self.pos
    }

    #[inline]
    pub fn end(self) -> i32 {
        self.end
    }

    #[inline]
    pub fn len(self) -> i32 {
        self.end - self.pos
    }

    pub fn is_valid(self) -> bool {
        self.pos >= 0 || self.end >= 0
    }

    pub fn contains(self, pos: i32) -> bool {
        pos >= self.pos && pos < self.end
    }

    pub fn contains_inclusive(self, pos: i32) -> bool {
        pos >= self.pos && pos <= self.end
    }

    pub fn contains_exclusive(self, pos: i32) -> bool {
        self.pos < pos && pos < self.end
    }

    pub fn with_pos(self, pos: i32) -> TextRange {
        TextRange { pos, end: self.end }
    }

    pub fn with_end(self, end: i32) -> TextRange {
        TextRange { pos: self.pos, end }
    }

    pub fn contained_by(self, t2: TextRange) -> bool {
        t2.pos <= self.pos && t2.end >= self.end
    }

    pub fn overlaps(self, t2: TextRange) -> bool {
        let start = self.pos.max(t2.pos);
        let end = self.end.min(t2.end);
        start < end
    }

    // Similar to Overlaps, but treats touching ranges as intersecting.
    // For example, [0, 5) intersects [5, 10).
    pub fn intersects(self, t2: TextRange) -> bool {
        let start = self.pos.max(t2.pos);
        let end = self.end.min(t2.end);
        start <= end
    }
}

#[inline]
pub const fn new_text_range(pos: i32, end: i32) -> TextRange {
    TextRange { pos, end }
}

#[inline]
pub const fn undefined_text_range() -> TextRange {
    TextRange { pos: -1, end: -1 }
}

pub fn compare_text_ranges(r1: TextRange, r2: TextRange) -> i32 {
    let c = r1.pos - r2.pos;
    if c != 0 {
        return c;
    }
    r1.end - r2.end
}
