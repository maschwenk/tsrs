// Port of Go's fourslash/tests/util/util.go. The package-level variables are generated (util_gen.rs); the
// functions are here.

use tsrs_core::stringutil;
use tsrs_lsproto as lsproto;

use crate::go::{self, Any};
use tsrs_ls as ls;

pub use super::util_gen::*;

// util.go:18
pub fn insert_replace_text_edit(new_text: &str, edit_range: lsproto::Range) -> Option<lsproto::TextEditOrInsertReplaceEdit> {
    Some(lsproto::TextEditOrInsertReplaceEdit {
        insert_replace_edit: Some(lsproto::InsertReplaceEdit { new_text: new_text.to_string(), insert: edit_range, replace: edit_range }),
        ..Default::default()
    })
}

// util.go:1364
pub fn sort_completion_items(items: &[Any]) -> Vec<Any> {
    let compare_strings = stringutil::compare_strings_case_insensitive_then_sensitive;
    let mut items = items.to_vec();
    items.sort_by(|a, b| {
        let default_sort_text = ls::SORT_TEXT_LOCATION_PRIORITY.to_string();
        let mut a_sort_text = String::new();
        let mut b_sort_text = String::new();
        if let Any::CompletionItem(a) = a {
            if let Some(s) = &a.sort_text {
                a_sort_text = s.clone();
            }
        }
        if let Any::CompletionItem(b) = b {
            if let Some(s) = &b.sort_text {
                b_sort_text = s.clone();
            }
        }
        let a_sort_text = go::or_else(a_sort_text, default_sort_text.clone());
        let b_sort_text = go::or_else(b_sort_text, default_sort_text);
        let by_sort_text = compare_strings(&a_sort_text, &b_sort_text);
        if by_sort_text != 0 {
            return by_sort_text.cmp(&0);
        }
        let a_label = match a {
            Any::CompletionItem(a) => a.label.clone(),
            Any::String(a) => a.clone(),
            _ => panic!("unexpected completion item type: {}", a.type_name()),
        };
        let b_label = match b {
            Any::CompletionItem(b) => b.label.clone(),
            Any::String(b) => b.clone(),
            _ => panic!("unexpected completion item type: {}", b.type_name()),
        };
        compare_strings(&a_label, &b_label).cmp(&0)
    });
    items
}

// util.go:1410
pub fn completion_globals_plus(items: &[Any], no_lib: bool) -> Vec<Any> {
    let all: Vec<Any> = if no_lib {
        [items, &[Any::CompletionItem(COMPLETION_GLOBAL_THIS_ITEM.clone().unwrap()), Any::CompletionItem(COMPLETION_UNDEFINED_VAR_ITEM.clone().unwrap())], &COMPLETION_GLOBAL_KEYWORDS[..]]
            .concat()
    } else {
        [items, &COMPLETION_GLOBALS[..]].concat()
    };
    sort_completion_items(&all)
}

// util.go:1424
pub fn completion_global_types_plus(items: &[Any]) -> Vec<Any> {
    sort_completion_items(&[&COMPLETION_GLOBAL_TYPE_DECLS[..], &[Any::CompletionItem(COMPLETION_GLOBAL_THIS_ITEM.clone().unwrap())], &COMPLETION_TYPE_KEYWORDS[..], items].concat())
}

// util.go:1435
pub fn get_in_js_keywords(keywords: &[Any]) -> Vec<Any> {
    go::filter(keywords, |item| {
        let label = match item {
            Any::CompletionItem(item) => item.label.clone(),
            Any::String(item) => item.clone(),
            _ => panic!("unexpected completion item type: {}", item.type_name()),
        };
        !matches!(
            label.as_str(),
            "enum"
                | "interface"
                | "implements"
                | "private"
                | "protected"
                | "public"
                | "abstract"
                | "any"
                | "boolean"
                | "declare"
                | "infer"
                | "is"
                | "keyof"
                | "module"
                | "namespace"
                | "never"
                | "readonly"
                | "number"
                | "object"
                | "string"
                | "symbol"
                | "type"
                | "unique"
                | "override"
                | "unknown"
                | "global"
                | "bigint"
        )
    })
}

// util.go:1462
pub fn completion_globals_in_js_plus(items: &[Any], no_lib: bool) -> Vec<Any> {
    let mut all = [
        items,
        &[Any::CompletionItem(COMPLETION_GLOBAL_THIS_ITEM.clone().unwrap()), Any::CompletionItem(COMPLETION_UNDEFINED_VAR_ITEM.clone().unwrap())],
        &COMPLETION_GLOBAL_IN_JS_KEYWORDS[..],
    ]
    .concat();
    if !no_lib {
        all.extend(COMPLETION_GLOBAL_VARS.iter().cloned());
    }
    sort_completion_items(&all)
}

// util.go:1533
pub fn completion_function_members_plus(items: &[Any]) -> Vec<Any> {
    sort_completion_items(&[&COMPLETION_FUNCTION_MEMBERS[..], items].concat())
}

// util.go:1552
pub fn completion_function_members_with_prototype_plus(items: &[Any]) -> Vec<Any> {
    sort_completion_items(&[&COMPLETION_FUNCTION_MEMBERS_WITH_PROTOTYPE[..], items].concat())
}

// util.go:1561
pub fn completion_type_keywords_plus(items: &[Any]) -> Vec<Any> {
    sort_completion_items(&[&COMPLETION_TYPE_KEYWORDS[..], items].concat())
}

// util.go:1578
pub fn to_any<I: Clone + Into<Any>>(items: &[I]) -> Vec<Any> {
    items.iter().map(|item| item.clone().into()).collect()
}
