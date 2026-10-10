//! Owned array fields and snapshots for recursive graph operations.
//!
//! A view retains its array storage across field replacement and owner destruction. Graph keys inside an array
//! do not retain their referents; the graph owner remains responsible for those records.

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Bound, Deref, RangeBounds};
use std::sync::Arc;

/// A retained view, including subranges, of an immutable owned array. Empty views require no allocation.
#[expect(clippy::rc_buffer, reason = "a thin Arc<Vec<T>> preserves one-word array fields and retains the original vector allocation")]
pub struct ArrayView<T> {
    data: Option<Arc<Vec<T>>>,
    start: usize,
    end: usize,
}

impl<T> Default for ArrayView<T> {
    fn default() -> Self {
        Self {
            data: None,
            start: 0,
            end: 0,
        }
    }
}

impl<T> Clone for ArrayView<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            start: self.start,
            end: self.end,
        }
    }
}

impl<T> ArrayView<T> {
    pub fn from_vec(values: Vec<T>) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let end = values.len();
        Self {
            data: Some(Arc::new(values)),
            start: 0,
            end,
        }
    }

    #[expect(clippy::rc_buffer, reason = "shares the thin owner of an existing array field without copying its elements")]
    fn from_shared(data: Arc<Vec<T>>) -> Self {
        let end = data.len();
        Self {
            data: Some(data),
            start: 0,
            end,
        }
    }

    /// Heap storage retained by the view, including the complete backing vector of a subrange.
    pub fn heap_usage(&self) -> (usize, usize, usize) {
        self.data.as_ref().map_or((0, 0, 0), |data| {
            let bytes = 2 * std::mem::size_of::<usize>() + std::mem::size_of::<Vec<T>>()
                + data.capacity() * std::mem::size_of::<T>();
            (data.len(), data.capacity(), bytes)
        })
    }

    pub fn slice(&self, range: impl RangeBounds<usize>) -> Self {
        let start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n.checked_add(1).expect("array range overflow"),
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&n) => n.checked_add(1).expect("array range overflow"),
            Bound::Excluded(&n) => n,
            Bound::Unbounded => self.len(),
        };
        assert!(
            start <= end && end <= self.len(),
            "array range out of bounds"
        );
        Self {
            data: self.data.clone(),
            start: self.start + start,
            end: self.start + end,
        }
    }
}

impl<T> Deref for ArrayView<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.data
            .as_ref()
            .map_or(&[], |data| &data[self.start..self.end])
    }
}

impl<T> AsRef<[T]> for ArrayView<T> {
    fn as_ref(&self) -> &[T] {
        self
    }
}

impl<T: fmt::Debug> fmt::Debug for ArrayView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<T: PartialEq, R: AsRef<[T]> + ?Sized> PartialEq<R> for ArrayView<T> {
    fn eq(&self, other: &R) -> bool {
        self.as_ref() == other.as_ref()
    }
}
impl<T: Eq> Eq for ArrayView<T> {}
impl<T: Hash> Hash for ArrayView<T> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.as_ref().hash(h);
    }
}

impl<'a, T> IntoIterator for &'a ArrayView<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct ArrayIter<T> {
    view: ArrayView<T>,
    next: usize,
}

impl<T: Copy> Iterator for ArrayIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        let value = *self.view.get(self.next)?;
        self.next += 1;
        Some(value)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.view.len() - self.next;
        (remaining, Some(remaining))
    }
}
impl<T: Copy> ExactSizeIterator for ArrayIter<T> {}
impl<T: Copy> IntoIterator for ArrayView<T> {
    type Item = T;
    type IntoIter = ArrayIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        ArrayIter {
            view: self,
            next: 0,
        }
    }
}

/// An array field. Reads clone a snapshot lease, so recursive writes cannot invalidate a reader.
#[repr(transparent)]
#[expect(clippy::rc_buffer, reason = "a thin Arc<Vec<T>> keeps an owned graph-array field one word wide")]
pub struct ArrayCell<T>(Cell<Option<Arc<Vec<T>>>>);

impl<T> Default for ArrayCell<T> {
    fn default() -> Self {
        Self(Cell::new(None))
    }
}

impl<T> ArrayCell<T> {
    pub fn from_vec(values: Vec<T>) -> Self {
        let cell = Self::default();
        cell.set_owned(values);
        cell
    }
    pub fn get(&self) -> ArrayView<T> {
        let data = self.0.take();
        let view = data.as_ref().map_or_else(ArrayView::default, |data| {
            ArrayView::from_shared(Arc::clone(data))
        });
        self.0.set(data);
        view
    }
    pub fn set_owned(&self, values: Vec<T>) {
        self.0.set((!values.is_empty()).then(|| Arc::new(values)));
    }
}

impl<T: Clone> ArrayCell<T> {
    pub fn new(values: &[T]) -> Self {
        let cell = Self::default();
        cell.set(values);
        cell
    }
    pub fn set(&self, values: &[T]) {
        self.set_owned(values.to_vec());
    }
}

/// An optional array field: nil and a computed empty array remain distinct.
#[repr(transparent)]
#[expect(clippy::rc_buffer, reason = "a thin Arc<Vec<T>> keeps optional owned graph arrays one word wide, including computed empty arrays")]
pub struct OptionArrayCell<T>(Cell<Option<Arc<Vec<T>>>>);
impl<T> Default for OptionArrayCell<T> {
    fn default() -> Self {
        Self(Cell::new(None))
    }
}
impl<T> OptionArrayCell<T> {
    pub fn get(&self) -> Option<ArrayView<T>> {
        let data = self.0.take();
        let view = data
            .as_ref()
            .map(|data| ArrayView::from_shared(Arc::clone(data)));
        self.0.set(data);
        view
    }
    pub fn set_owned(&self, values: Option<Vec<T>>) {
        self.0.set(values.map(Arc::new));
    }
}
impl<T: Clone> OptionArrayCell<T> {
    pub fn set(&self, values: Option<&[T]>) {
        self.set_owned(values.map(<[T]>::to_vec));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_keep_values_through_replacement_and_owner_drop() {
        let value = Arc::new(String::from("retained"));
        let weak = Arc::downgrade(&value);
        let cell = ArrayCell::default();
        cell.set_owned(vec![value]);
        let view = cell.get();
        let range = view.slice(..1);
        cell.set(&[]);
        drop(cell);
        drop(view);
        assert_eq!(&**range[0], "retained");
        assert!(weak.upgrade().is_some());
        drop(range);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn nil_and_computed_empty_survive_snapshot_reads() {
        let cell = OptionArrayCell::<u32>::default();
        assert!(cell.get().is_none());
        cell.set(Some(&[]));
        let empty = cell.get().unwrap();
        cell.set(None);
        assert!(empty.is_empty());
        assert!(cell.get().is_none());
    }
}
