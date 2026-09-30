//! Non-function declarations of `jsx.go`.

use std::rc::Rc;

use bitflags::bitflags;

use crate::*;

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct JsxFlags: u32 {
        const None = 0;
        const IntrinsicNamedElement = 1 << 0; // An element from a named property of the JSX.IntrinsicElements interface
        const IntrinsicIndexedElement = 1 << 1; // An element inferred from the string index signature of the JSX.IntrinsicElements interface
        const IntrinsicElement = Self::IntrinsicNamedElement.bits() | Self::IntrinsicIndexedElement.bits();
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum JsxReferenceKind {
    #[default]
    Component,
    Function,
    Mixed,
}

#[derive(Default)]
pub struct JsxElementLinks {
    pub jsx_flags: Cell<JsxFlags>, // Flags for the JSX element
    pub resolved_jsx_element_attributes_type: Cell<Option<P<Type>>>, // Resolved element attributes type of a JSX opening-like element
    pub jsx_namespace: Cell<Option<P<Symbol>>>, // Resolved JSX namespace symbol for this node
    pub jsx_implicit_import_container: Cell<Option<P<Symbol>>>, // Resolved module symbol the implicit JSX import of this file should refer to
    pub first_jsx_tag_in_file: Cell<Option<P<Node>>>, // The first JSX tag in the file
}

pub struct JsxNamesStruct {
    pub jsx: &'static str,
    pub intrinsic_elements: &'static str,
    pub element_class: &'static str,
    pub element_attributes_property_name_container: &'static str,
    pub element_children_attribute_name_container: &'static str,
    pub element: &'static str,
    pub element_type: &'static str,
    pub intrinsic_attributes: &'static str,
    pub intrinsic_class_attributes: &'static str,
    pub library_managed_attributes: &'static str,
}

pub static JsxNames: JsxNamesStruct = JsxNamesStruct {
    jsx: "JSX",
    intrinsic_elements: "IntrinsicElements",
    element_class: "ElementClass",
    element_attributes_property_name_container: "ElementAttributesProperty",
    element_children_attribute_name_container: "ElementChildrenAttribute",
    element: "Element",
    element_type: "ElementType",
    intrinsic_attributes: "IntrinsicAttributes",
    intrinsic_class_attributes: "IntrinsicClassAttributes",
    library_managed_attributes: "LibraryManagedAttributes",
};

pub struct ReactNamesStruct {
    pub fragment: &'static str,
}

pub static ReactNames: ReactNamesStruct = ReactNamesStruct { fragment: "Fragment" };

#[derive(Clone, Default)]
pub struct JsxElaborationElement {
    pub error_node: Option<P<Node>>,
    pub inner_expression: Option<P<Node>>,
    pub name_type: Option<P<Type>>,
    pub create_diagnostic: Option<Rc<dyn Fn(&mut Checker, P<Node>) -> P<Diagnostic>>>, // Optional: creates a custom diagnostic for this element
}
