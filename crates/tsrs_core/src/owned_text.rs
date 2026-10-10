//! Rust-owned text fields and retained snapshots. No graph owner or arena is needed to read the text.

#![forbid(unsafe_code)]

use std::borrow::Borrow;
use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

/// A retained immutable string. Empty strings require no allocation.
#[derive(Clone, Default)]
#[expect(
    clippy::rc_buffer,
    reason = "a thin Arc<String> keeps text fields one word wide and retains the original string allocation"
)]
pub struct TextView(Option<Arc<String>>);

impl From<String> for TextView {
    fn from(text: String) -> Self {
        Self((!text.is_empty()).then(|| Arc::new(text)))
    }
}

impl From<&str> for TextView {
    fn from(text: &str) -> Self {
        Self::from(text.to_owned())
    }
}

impl From<&String> for TextView {
    fn from(text: &String) -> Self {
        Self::from(text.as_str())
    }
}

impl From<&TextView> for TextView {
    fn from(text: &TextView) -> Self {
        text.clone()
    }
}

impl Deref for TextView {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.as_ref().map_or("", |text| text.as_str())
    }
}

impl TextView {
    pub fn as_str(&self) -> &str {
        self
    }
}

impl AsRef<str> for TextView {
    fn as_ref(&self) -> &str {
        self
    }
}

impl Borrow<str> for TextView {
    fn borrow(&self) -> &str {
        self
    }
}

impl fmt::Debug for TextView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Display for TextView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self)
    }
}

impl<R: AsRef<str> + ?Sized> PartialEq<R> for TextView {
    fn eq(&self, other: &R) -> bool {
        self.as_ref() == other.as_ref()
    }
}
impl Eq for TextView {}
impl PartialEq<TextView> for str {
    fn eq(&self, other: &TextView) -> bool {
        self == other.as_ref()
    }
}
impl PartialEq<TextView> for &str {
    fn eq(&self, other: &TextView) -> bool {
        *self == other.as_ref()
    }
}
impl PartialEq<TextView> for String {
    fn eq(&self, other: &TextView) -> bool {
        self.as_str() == other.as_ref()
    }
}
impl PartialOrd for TextView {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TextView {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}
impl Hash for TextView {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

/// A text field whose reads retain their storage across replacement and owner destruction.
#[derive(Default)]
#[repr(transparent)]
#[expect(
    clippy::rc_buffer,
    reason = "a thin Arc<String> keeps owned mutable text fields one word wide"
)]
pub struct TextCell(Cell<Option<Arc<String>>>);

impl TextCell {
    pub fn new(text: impl Into<TextView>) -> Self {
        Self(Cell::new(text.into().0))
    }

    pub fn get(&self) -> TextView {
        let data = self.0.take();
        let view = TextView(data.as_ref().map(Arc::clone));
        self.0.set(data);
        view
    }

    pub fn set(&self, text: impl Into<TextView>) {
        self.0.set(text.into().0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_snapshots_retain_input_through_replacement_and_owner_drop() {
        let input = String::from("retained λ text");
        let buffer = input.as_ptr();
        let text = TextView::from(input);
        assert_eq!(text.as_ptr(), buffer);
        let weak = Arc::downgrade(text.0.as_ref().unwrap());
        let field = TextCell::new(text);
        let view = field.get();
        field.set("replacement");
        drop(field);
        assert_eq!(view, "retained λ text");
        assert!(weak.upgrade().is_some());
        drop(view);
        assert!(weak.upgrade().is_none());
    }
}
