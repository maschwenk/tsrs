//! Value-symbol link payloads use a typed state and an owned rare tail. Graph edges migrate with their graphs.
#![forbid(unsafe_code)]

use crate::{Symbol, Type, TypeMapper, escape_mapper};
use std::cell::{Cell, RefCell};
use tsrs_core::P;

#[derive(Default)]
pub struct ValueSymbolLinks {
    pub resolved_type: Cell<Option<P<Type>>>,
    fields: RefCell<Fields>,
}

// Common records retain two fields inline; only records that mix the two shapes or write additional fields
// allocate a tail. Rust's enum discriminant replaces erased pointers and mode bits.
enum Fields {
    Plain { target: Option<P<Symbol>>, mapper: Option<P<TypeMapper>> },
    Synthetic { containing_type: Option<P<Type>>, name_type: Option<P<Type>> },
    Tail(Box<Tail>),
}

impl Default for Fields {
    fn default() -> Self {
        Self::Plain { target: None, mapper: None }
    }
}

#[derive(Default)]
struct Tail {
    target: Option<P<Symbol>>,
    mapper: Option<P<TypeMapper>>,
    write_type: Option<P<Type>>,
    name_type: Option<P<Type>>,
    containing_type: Option<P<Type>>,
    function_or_constructor_checked: bool,
}

impl Fields {
    fn tail_for_write(&mut self) -> &mut Tail {
        if !matches!(self, Self::Tail(_)) {
            let mut tail = Box::<Tail>::default();
            match std::mem::take(self) {
                Self::Plain { target, mapper } => {
                    tail.target = target;
                    tail.mapper = mapper;
                }
                Self::Synthetic { containing_type, name_type } => {
                    tail.containing_type = containing_type;
                    tail.name_type = name_type;
                }
                Self::Tail(_) => unreachable!("tail was already selected"),
            }
            *self = Self::Tail(tail);
        }
        match self {
            Self::Tail(tail) => tail,
            _ => unreachable!("tail was just initialized"),
        }
    }

    fn enter_synthetic(&mut self) {
        if matches!(self, Self::Plain { target: None, mapper: None }) {
            *self = Self::Synthetic { containing_type: None, name_type: None };
        }
    }
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<ValueSymbolLinks>() == 40);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<ValueSymbolLinks>() == 20);

impl ValueSymbolLinks {
    #[inline]
    pub fn target(&self) -> Option<P<Symbol>> {
        match &*self.fields.borrow() {
            Fields::Plain { target, .. } => *target,
            Fields::Synthetic { .. } => None,
            Fields::Tail(tail) => tail.target,
        }
    }
    #[inline]
    pub fn set_target(&self, target: Option<P<Symbol>>) {
        let mut fields = self.fields.borrow_mut();
        match &mut *fields {
            Fields::Plain { target: stored, .. } => *stored = target,
            Fields::Synthetic { .. } if target.is_none() => {}
            _ => fields.tail_for_write().target = target,
        }
    }
    #[inline]
    pub fn mapper(&self) -> Option<P<TypeMapper>> {
        match &*self.fields.borrow() {
            Fields::Plain { mapper, .. } => *mapper,
            Fields::Synthetic { .. } => None,
            Fields::Tail(tail) => tail.mapper,
        }
    }
    #[inline]
    pub fn set_mapper(&self, mapper: Option<P<TypeMapper>>) {
        if let Some(mapper) = mapper {
            escape_mapper(mapper);
        }
        let mut fields = self.fields.borrow_mut();
        match &mut *fields {
            Fields::Plain { mapper: stored, .. } => *stored = mapper,
            Fields::Synthetic { .. } if mapper.is_none() => {}
            _ => fields.tail_for_write().mapper = mapper,
        }
    }
    #[inline]
    pub fn containing_type(&self) -> Option<P<Type>> {
        match &*self.fields.borrow() {
            Fields::Plain { .. } => None,
            Fields::Synthetic { containing_type, .. } => *containing_type,
            Fields::Tail(tail) => tail.containing_type,
        }
    }
    #[inline]
    pub fn set_containing_type(&self, containing_type: Option<P<Type>>) {
        let mut fields = self.fields.borrow_mut();
        if containing_type.is_none() && matches!(&*fields, Fields::Plain { .. }) {
            return;
        }
        fields.enter_synthetic();
        match &mut *fields {
            Fields::Synthetic { containing_type: stored, .. } => *stored = containing_type,
            _ => fields.tail_for_write().containing_type = containing_type,
        }
    }
    #[inline]
    pub fn name_type(&self) -> Option<P<Type>> {
        match &*self.fields.borrow() {
            Fields::Plain { .. } => None,
            Fields::Synthetic { name_type, .. } => *name_type,
            Fields::Tail(tail) => tail.name_type,
        }
    }
    #[inline]
    pub fn set_name_type(&self, name_type: Option<P<Type>>) {
        let mut fields = self.fields.borrow_mut();
        if name_type.is_none() && matches!(&*fields, Fields::Plain { .. }) {
            return;
        }
        fields.enter_synthetic();
        match &mut *fields {
            Fields::Synthetic { name_type: stored, .. } => *stored = name_type,
            _ => fields.tail_for_write().name_type = name_type,
        }
    }
    #[inline]
    pub fn write_type(&self) -> Option<P<Type>> {
        match &*self.fields.borrow() {
            Fields::Tail(tail) => tail.write_type,
            _ => None,
        }
    }
    #[inline]
    pub fn set_write_type(&self, write_type: Option<P<Type>>) {
        let mut fields = self.fields.borrow_mut();
        if write_type.is_some() || matches!(&*fields, Fields::Tail(_)) {
            fields.tail_for_write().write_type = write_type;
        }
    }
    #[inline]
    pub fn function_or_constructor_checked(&self) -> bool {
        match &*self.fields.borrow() {
            Fields::Tail(tail) => tail.function_or_constructor_checked,
            _ => false,
        }
    }
    #[inline]
    pub fn set_function_or_constructor_checked(&self, checked: bool) {
        let mut fields = self.fields.borrow_mut();
        if checked || matches!(&*fields, Fields::Tail(_)) {
            fields.tail_for_write().function_or_constructor_checked = checked;
        }
    }
}
