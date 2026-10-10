use crate::stringutil;
use crate::tspath;
use crate::{CompilerOptions, ScriptKind, TextPos};
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::hash::Hash;

// Go helpers that return their input slice unchanged when nothing changes return `Cow::Borrowed`
// so callers can preserve slice identity (see `same`).

pub fn apply_debug_stack_limit() {}

pub fn filter<T: Clone>(slice: &[T], mut f: impl FnMut(&T) -> bool) -> Cow<'_, [T]> {
    for (i, value) in slice.iter().enumerate() {
        if !f(value) {
            let mut result = slice[..i].to_vec();
            for value in &slice[i + 1..] {
                if f(value) {
                    result.push(value.clone());
                }
            }
            return Cow::Owned(result);
        }
    }
    Cow::Borrowed(slice)
}

pub fn filter_index<T: Clone>(slice: &[T], mut f: impl FnMut(&T, usize, &[T]) -> bool) -> Cow<'_, [T]> {
    for (i, value) in slice.iter().enumerate() {
        if !f(value, i, slice) {
            let mut result = slice[..i].to_vec();
            for j in i + 1..slice.len() {
                if f(&slice[j], j, slice) {
                    result.push(slice[j].clone());
                }
            }
            return Cow::Owned(result);
        }
    }
    Cow::Borrowed(slice)
}

pub fn map<T, U>(slice: &[T], f: impl FnMut(&T) -> U) -> Vec<U> {
    slice.iter().map(f).collect()
}

pub fn try_map<T, U, E>(slice: &[T], mut f: impl FnMut(&T) -> Result<U, E>) -> Result<Vec<U>, E> {
    let mut result = Vec::with_capacity(slice.len());
    for value in slice {
        result.push(f(value)?);
    }
    Ok(result)
}

pub fn map_index<T, U>(slice: &[T], mut f: impl FnMut(&T, usize) -> U) -> Vec<U> {
    slice.iter().enumerate().map(|(i, v)| f(v, i)).collect()
}

/// Go `MapNonNil`: the callback returns `None` where Go returns the zero value.
pub fn map_non_nil<T, U>(slice: &[T], f: impl FnMut(&T) -> Option<U>) -> Vec<U> {
    slice.iter().filter_map(f).collect()
}

pub fn map_filtered<T, U>(slice: &[T], f: impl FnMut(&T) -> Option<U>) -> Vec<U> {
    slice.iter().filter_map(f).collect()
}

pub fn flat_map<T, U, I: IntoIterator<Item = U>>(slice: &[T], mut f: impl FnMut(&T) -> I) -> Vec<U> {
    let mut result = Vec::new();
    for value in slice {
        result.extend(f(value));
    }
    result
}

pub fn same_map<T: Clone + PartialEq>(slice: &[T], mut f: impl FnMut(&T) -> T) -> Cow<'_, [T]> {
    for (i, value) in slice.iter().enumerate() {
        let mapped = f(value);
        if mapped != *value {
            let mut result = Vec::with_capacity(slice.len());
            result.extend_from_slice(&slice[..i]);
            result.push(mapped);
            for v in &slice[i + 1..] {
                result.push(f(v));
            }
            return Cow::Owned(result);
        }
    }
    Cow::Borrowed(slice)
}

pub fn same_map_index<T: Clone + PartialEq>(slice: &[T], mut f: impl FnMut(&T, usize) -> T) -> Cow<'_, [T]> {
    for (i, value) in slice.iter().enumerate() {
        let mapped = f(value, i);
        if mapped != *value {
            let mut result = Vec::with_capacity(slice.len());
            result.extend_from_slice(&slice[..i]);
            result.push(mapped);
            for j in i + 1..slice.len() {
                result.push(f(&slice[j], j));
            }
            return Cow::Owned(result);
        }
    }
    Cow::Borrowed(slice)
}

/// Go `Same`: slices are identical (same length and same backing storage).
pub fn same<T>(s1: &[T], s2: &[T]) -> bool {
    if s1.len() == s2.len() {
        return s1.is_empty() || std::ptr::eq(s1.as_ptr(), s2.as_ptr());
    }
    false
}

pub fn some<T>(slice: &[T], f: impl FnMut(&T) -> bool) -> bool {
    slice.iter().any(f)
}

pub fn every<T>(slice: &[T], f: impl FnMut(&T) -> bool) -> bool {
    slice.iter().all(f)
}

pub fn or<T>(funcs: Vec<Box<dyn Fn(&T) -> bool>>) -> impl Fn(&T) -> bool {
    move |input| funcs.iter().any(|f| f(input))
}

pub fn find<T: Clone>(slice: &[T], mut f: impl FnMut(&T) -> bool) -> Option<T> {
    slice.iter().find(|v| f(v)).cloned()
}

pub fn find_last<T: Clone>(slice: &[T], mut f: impl FnMut(&T) -> bool) -> Option<T> {
    slice.iter().rev().find(|v| f(v)).cloned()
}

pub fn find_index<T>(slice: &[T], f: impl FnMut(&T) -> bool) -> i32 {
    match slice.iter().position(f) {
        Some(i) => i as i32,
        None => -1,
    }
}

pub fn find_last_index<T>(slice: &[T], f: impl FnMut(&T) -> bool) -> i32 {
    match slice.iter().rposition(f) {
        Some(i) => i as i32,
        None => -1,
    }
}

pub fn first_or_nil<T: Clone>(slice: &[T]) -> Option<T> {
    slice.first().cloned()
}

pub fn last_or_nil<T: Clone>(slice: &[T]) -> Option<T> {
    slice.last().cloned()
}

pub fn element_or_nil<T: Clone>(slice: &[T], index: usize) -> Option<T> {
    slice.get(index).cloned()
}

pub fn first_or_nil_seq<T>(seq: impl IntoIterator<Item = T>) -> Option<T> {
    seq.into_iter().next()
}

pub fn first_non_nil<T, U>(slice: &[T], f: impl FnMut(&T) -> Option<U>) -> Option<U> {
    slice.iter().find_map(f)
}

pub fn first_non_zero<T: Clone + Default + PartialEq>(values: &[T]) -> T {
    let zero = T::default();
    for value in values {
        if *value != zero {
            return value.clone();
        }
    }
    zero
}

pub fn concatenate<'a, T: Clone>(s1: &'a [T], s2: &'a [T]) -> Cow<'a, [T]> {
    if s2.is_empty() {
        return Cow::Borrowed(s1);
    }
    if s1.is_empty() {
        return Cow::Borrowed(s2);
    }
    let mut result = Vec::with_capacity(s1.len() + s2.len());
    result.extend_from_slice(s1);
    result.extend_from_slice(s2);
    Cow::Owned(result)
}

pub fn splice<'a, T: Clone>(s1: &'a [T], start: i32, delete_count: i32, items: &[T]) -> Cow<'a, [T]> {
    let len = s1.len() as i32;
    let mut start = start;
    if start < 0 {
        start += len;
    }
    if start < 0 {
        start = 0;
    }
    if start > len {
        start = len;
    }
    let delete_count = delete_count.max(0);
    let end = (start + delete_count).min(len);
    if start == end && items.is_empty() {
        return Cow::Borrowed(s1);
    }
    let (start, end) = (start as usize, end as usize);
    let mut result = Vec::with_capacity(s1.len() - (end - start) + items.len());
    result.extend_from_slice(&s1[..start]);
    result.extend_from_slice(items);
    result.extend_from_slice(&s1[end..]);
    Cow::Owned(result)
}

pub fn count_where<T>(slice: &[T], mut f: impl FnMut(&T) -> bool) -> usize {
    slice.iter().filter(|v| f(v)).count()
}

pub fn replace_element<T: Clone>(slice: &[T], i: usize, t: T) -> Vec<T> {
    let mut result = slice.to_vec();
    result[i] = t;
    result
}

pub fn insert_sorted<T>(slice: &mut Vec<T>, element: T, mut cmp: impl FnMut(&T, &T) -> i32) {
    let i = match slice.binary_search_by(|probe| cmp(probe, &element).cmp(&0)) {
        Ok(i) => {
            // slices.BinarySearchFunc returns the first matching position.
            let mut i = i;
            while i > 0 && cmp(&slice[i - 1], &element) == 0 {
                i -= 1;
            }
            i
        }
        Err(i) => i,
    };
    slice.insert(i, element);
}

// MinAllFunc returns all minimum elements from xs according to the comparison function cmp.
pub fn min_all_func<T: Clone>(xs: &[T], mut cmp: impl FnMut(&T, &T) -> i32) -> Vec<T> {
    if xs.is_empty() {
        return Vec::new();
    }

    let mut m = xs[0].clone();
    let mut mins = vec![m.clone()];

    for x in &xs[1..] {
        let c = cmp(x, &m);
        if c < 0 {
            m = x.clone();
            mins.clear();
            mins.push(x.clone());
        } else if c == 0 {
            mins.push(x.clone());
        }
    }

    mins
}

pub fn append_if_unique<T: PartialEq>(slice: &mut Vec<T>, element: T) {
    if slice.contains(&element) {
        return;
    }
    slice.push(element);
}

pub fn memoize<T: Clone>(create: impl FnOnce() -> T) -> impl FnMut() -> T {
    let mut create = Some(create);
    let mut value: Option<T> = None;
    move || {
        if let Some(c) = create.take() {
            value = Some(c());
        }
        value.clone().unwrap()
    }
}

// Returns whenTrue if b is true; otherwise, returns whenFalse. IfElse should only be used when branches are either
// constant or precomputed as both branches will be evaluated regardless as to the value of b.
#[inline]
pub fn if_else<T>(b: bool, when_true: T, when_false: T) -> T {
    if b {
        when_true
    } else {
        when_false
    }
}

// Returns value if value is not the zero value of T; Otherwise, returns defaultValue. OrElse should only be used when
// defaultValue is constant or precomputed as its argument will be evaluated regardless as to the content of value.
pub fn or_else<T: Default + PartialEq>(value: T, default_value: T) -> T {
    if value != T::default() {
        return value;
    }
    default_value
}

// Returns `a` if `a` is not `nil`; Otherwise, returns `b`. Coalesce is roughly analogous to `??` in JS, except that it
// non-shortcutting, so it is advised to only use a constant or precomputed value for `b`
#[inline]
pub fn coalesce<T>(a: Option<T>, b: Option<T>) -> Option<T> {
    a.or(b)
}

pub type ECMALineStarts = Vec<TextPos>;

pub fn compute_ecma_line_starts(text: &str) -> ECMALineStarts {
    let mut result = Vec::with_capacity(memchr::memchr_iter(b'\n', text.as_bytes()).count() + 1);
    compute_ecma_line_starts_seq(text, |p| {
        result.push(p);
        true
    });
    result
}

/// Go returns an `iter.Seq`; here the positions are pushed to `yield_` until it returns false.
pub fn compute_ecma_line_starts_seq(text: &str, mut yield_: impl FnMut(TextPos) -> bool) {
    let bytes = text.as_bytes();
    let mut line_start = 0usize;
    while let Some(offset) = stringutil::find_line_break(&text[line_start..]) {
        let pos = line_start + offset;
        let b = bytes[pos];
        let size = if b == 0xE2 {
            3
        } else if b == b'\r' && bytes.get(pos + 1) == Some(&b'\n') {
            2
        } else {
            1
        };
        if !yield_(line_start as TextPos) {
            return;
        }
        line_start = pos + size;
    }
    yield_(line_start as TextPos);
}

/// tsrs-only: the number of positions `compute_ecma_line_starts_seq` yields (one per line terminator, with CR LF as
/// one, plus the last line), counted with vectorized byte searches instead of the per-byte loop.
pub fn count_ecma_line_starts(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut n = 1 + memchr::memchr_iter(b'\n', bytes).count();
    n += memchr::memchr_iter(b'\r', bytes).filter(|&i| bytes.get(i + 1) != Some(&b'\n')).count();
    if !text.is_ascii() {
        // U+2028 and U+2029 are E2 80 A8 and E2 80 A9; in valid UTF-8, 0xE2 is always a lead byte.
        n += memchr::memchr_iter(0xE2, bytes).filter(|&i| bytes.get(i + 1) == Some(&0x80) && matches!(bytes.get(i + 2), Some(0xA8 | 0xA9))).count();
    }
    n
}

// PositionToLineAndByteOffset returns the 0-based line and byte offset from the
// start of that line for the given byte position, using the provided line starts.
// The byte offset is a raw UTF-8 byte offset from the line start, not a UTF-16 code unit count.
pub fn position_to_line_and_byte_offset(position: i32, line_starts: &[TextPos]) -> (usize, i32) {
    let idx = line_starts.partition_point(|&s| s <= position);
    let line = if idx == 0 { 0 } else { idx - 1 };
    (line, position - line_starts[line])
}

// UTF16Offset represents a character offset measured in UTF-16 code units.
pub type UTF16Offset = i32;

// UTF16Len returns the number of UTF-16 code units needed to
// represent the given UTF-8 encoded string.
pub fn utf16_len(s: &str) -> UTF16Offset {
    if s.is_ascii() {
        return s.len() as UTF16Offset;
    }
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] >= 0x80 {
            // Found non-ASCII; count the ASCII prefix, then decode the rest.
            let mut n = i as UTF16Offset;
            let mut j = i;
            while j < bytes.len() {
                let (r, size) = stringutil::decode_rune(&bytes[j..]);
                j += size;
                n += if r >= 0x10000 { 2 } else { 1 };
            }
            return n;
        }
    }
    bytes.len() as UTF16Offset
}

pub fn flatten<T: Clone>(array: &[Vec<T>]) -> Vec<T> {
    let mut result = Vec::new();
    for sub_array in array {
        result.extend_from_slice(sub_array);
    }
    result
}

pub fn get_script_kind_from_file_name(file_name: &str) -> ScriptKind {
    if let Some(dot_pos) = file_name.rfind('.') {
        let ext = stringutil::go_strings_to_lower(&file_name[dot_pos..]);
        match ext.as_str() {
            tspath::EXTENSION_JS | tspath::EXTENSION_CJS | tspath::EXTENSION_MJS => return ScriptKind::JS,
            tspath::EXTENSION_JSX => return ScriptKind::JSX,
            tspath::EXTENSION_TS | tspath::EXTENSION_CTS | tspath::EXTENSION_MTS => return ScriptKind::TS,
            tspath::EXTENSION_TSX => return ScriptKind::TSX,
            tspath::EXTENSION_JSON => return ScriptKind::JSON,
            _ => {}
        }
    }
    ScriptKind::Unknown
}

pub fn get_default_extension_for_script_kind(script_kind: ScriptKind) -> &'static str {
    match script_kind {
        ScriptKind::JS => tspath::EXTENSION_JS,
        ScriptKind::JSX => tspath::EXTENSION_JSX,
        ScriptKind::TSX => tspath::EXTENSION_TSX,
        ScriptKind::JSON => tspath::EXTENSION_JSON,
        _ => tspath::EXTENSION_TS,
    }
}

// EnsureScriptKindFromFileName is like GetScriptKindFromFileName, but defaults to
// ScriptKindTS when the file name has no recognized extension (e.g. files included
// with allowNonTsExtensions), so the result is always safe to hand to the parser.
pub fn ensure_script_kind_from_file_name(file_name: &str) -> ScriptKind {
    let kind = get_script_kind_from_file_name(file_name);
    if kind != ScriptKind::Unknown {
        return kind;
    }
    ScriptKind::TS
}

// Given a name and a list of names that are *not* equal to the name, return a spelling suggestion if there is one that is close enough.
// Names less than length 3 only check for case-insensitive equality.
//
// find the candidate with the smallest Levenshtein distance,
//
//	except for candidates:
//	  * With no name
//	  * Whose length differs from the target name by more than 0.34 of the length of the name.
//	  * Whose levenshtein distance is more than 0.4 of the length of the name
//	    (0.4 allows 1 substitution/transposition for every 5 characters,
//	     and 1 insertion/deletion at 3 characters)
pub fn get_spelling_suggestion<T, S: AsRef<str>>(
    name: &str,
    candidates: impl IntoIterator<Item = T>,
    get_name: impl FnMut(&T) -> S,
    compare: impl FnMut(&T, &T) -> i32,
) -> Option<T> {
    get_spelling_suggestion_worker(name, candidates, get_name, compare, 0 /*maxCandidates*/)
}

pub fn get_spelling_suggestion_with_max_candidate_count<T, S: AsRef<str>>(
    name: &str,
    candidates: impl IntoIterator<Item = T>,
    get_name: impl FnMut(&T) -> S,
    compare: impl FnMut(&T, &T) -> i32,
    max_candidates: usize,
) -> Option<T> {
    get_spelling_suggestion_worker(name, candidates, get_name, compare, max_candidates)
}

fn to_runes(s: &str) -> Vec<i32> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let (r, size) = stringutil::decode_rune(&b[i..]);
        out.push(r);
        i += size;
    }
    out
}

// Go: unexported getSpellingSuggestion (renamed: collides with GetSpellingSuggestion after snake-casing).
fn get_spelling_suggestion_worker<T, S: AsRef<str>>(
    name: &str,
    candidates: impl IntoIterator<Item = T>,
    mut get_name: impl FnMut(&T) -> S,
    mut compare: impl FnMut(&T, &T) -> i32,
    max_candidates: usize,
) -> Option<T> {
    let rune_name = to_runes(name);
    let maximum_length_difference = 2usize.max((rune_name.len() as f64 * 0.34) as usize);
    let mut best_distance = (rune_name.len() as f64 * 0.4).floor() + 0.9; // If the best result is worse than this, don't bother.
    let mut buffers = LevenshteinBuffers::default();
    let mut best_candidate: Option<T> = None;
    let mut checked_candidates = 0;
    for candidate in candidates {
        checked_candidates += 1;
        if max_candidates > 0 && checked_candidates > max_candidates {
            return None;
        }
        let candidate_name_s = get_name(&candidate);
        let candidate_name = candidate_name_s.as_ref();
        // Go compares the candidate's byte length with the name's rune count here.
        let max_len = candidate_name.len().max(rune_name.len());
        let min_len = candidate_name.len().min(rune_name.len());
        if !candidate_name.is_empty() && max_len - min_len <= maximum_length_difference {
            if candidate_name == name {
                continue;
            }
            // Only consider candidates less than 3 characters long when they differ by case.
            // Otherwise, don't bother, since a user would usually notice differences of a 2-character name.
            if candidate_name.len() < 3 && !stringutil::equal_fold(candidate_name, name) {
                continue;
            }
            let distance = levenshtein_with_max(&mut buffers, &rune_name, &to_runes(candidate_name), best_distance);
            if distance < 0.0 {
                continue;
            }
            assert!(distance <= best_distance); // Else `levenshteinWithMax` should return undefined
            if distance < best_distance {
                best_distance = distance;
                best_candidate = Some(candidate);
            } else if best_candidate.as_ref().is_none_or(|best| compare(&candidate, best) < 0) {
                best_candidate = Some(candidate);
            }
        }
    }
    best_candidate
}

pub fn get_spelling_suggestion_for_strings<S: AsRef<str>>(name: &str, candidates: impl IntoIterator<Item = S>) -> Option<S> {
    get_spelling_suggestion(name, candidates, |s| s.as_ref().to_string(), |a, b| stringutil::compare_strings_case_sensitive(a.as_ref(), b.as_ref()))
}

#[derive(Default)]
struct LevenshteinBuffers {
    previous: Vec<f64>,
    current: Vec<f64>,
}

fn levenshtein_with_max(buffers: &mut LevenshteinBuffers, s1: &[i32], s2: &[i32], max_value: f64) -> f64 {
    let buffer_size = s2.len() + 1;
    buffers.previous.clear();
    buffers.previous.resize(buffer_size, 0.0);
    buffers.current.clear();
    buffers.current.resize(buffer_size, 0.0);

    let mut previous = std::mem::take(&mut buffers.previous);
    let mut current = std::mem::take(&mut buffers.current);

    let big = max_value + 0.01;
    for (i, p) in previous.iter_mut().enumerate() {
        *p = i as f64;
    }
    for i in 1..=s1.len() {
        let c1 = s1[i - 1];
        let min_j = ((i as f64 - max_value).ceil() as i64).max(1) as usize;
        let max_j = ((max_value + i as f64).floor() as i64).min(s2.len() as i64).max(0) as usize;
        let mut col_min = i as f64;
        current[0] = col_min;
        for j in 1..min_j.min(buffer_size) {
            current[j] = big;
        }
        for j in min_j..=max_j {
            let substitution_distance = if stringutil::unicode_to_lower(s1[i - 1]) == stringutil::unicode_to_lower(s2[j - 1]) {
                previous[j - 1] + 0.1
            } else {
                previous[j - 1] + 2.0
            };
            let dist = if c1 == s2[j - 1] {
                previous[j - 1]
            } else {
                (previous[j] + 1.0).min((current[j - 1] + 1.0).min(substitution_distance))
            };
            current[j] = dist;
            col_min = col_min.min(dist);
        }
        for j in max_j + 1..=s2.len() {
            current[j] = big;
        }
        if col_min > max_value {
            // Give up -- everything in this column is > max and it can't get better in future columns.
            buffers.previous = previous;
            buffers.current = current;
            return -1.0;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let res = previous[s2.len()];
    buffers.previous = previous;
    buffers.current = current;
    if res > max_value {
        return -1.0;
    }
    res
}

#[inline]
pub fn identity<T>(t: T) -> T {
    t
}

pub fn index_after(s: &str, pattern: &str, start_index: usize) -> i32 {
    match s[start_index..].find(pattern) {
        None => -1,
        Some(matched) => (matched + start_index) as i32,
    }
}

pub fn should_rewrite_module_specifier(specifier: &str, compiler_options: &CompilerOptions) -> bool {
    compiler_options.rewrite_relative_import_extensions.is_true()
        && tspath::path_is_relative(specifier)
        && !tspath::is_declaration_file_name(specifier)
        && tspath::has_ts_file_extension(specifier)
}

pub fn single_element_slice<T>(element: Option<T>) -> Vec<T> {
    match element {
        None => Vec::new(),
        Some(e) => vec![e],
    }
}

// DiffMaps compares two maps m1 and m2 and calls the provided callbacks for added, removed, and changed entries.
// onAdded is called for each key-value pair that is in m2 but not in m1.
// onRemoved is called for each key-value pair that is in m1 but not in m2.
// onChanged is called for each key where the value in m1 differs from the value in m2.
pub fn diff_maps<K: Hash + Eq, V: PartialEq>(
    m1: &FxHashMap<K, V>,
    m2: &FxHashMap<K, V>,
    on_added: impl FnMut(&K, &V),
    on_removed: impl FnMut(&K, &V),
    on_changed: impl FnMut(&K, &V, &V),
) {
    diff_maps_func(m1, m2, |a, b| a == b, Some(on_added), Some(on_removed), Some(on_changed))
}

#[expect(
    clippy::iter_over_hash_type,
    reason = "Go's DiffMapsFunc ranges over the maps too; the one caller (tsrs_project update_watches) collects watch changes, whose order is not output"
)]
pub fn diff_maps_func<K: Hash + Eq, V1, V2>(
    m1: &FxHashMap<K, V1>,
    m2: &FxHashMap<K, V2>,
    mut equal_values: impl FnMut(&V1, &V2) -> bool,
    on_added: Option<impl FnMut(&K, &V2)>,
    on_removed: Option<impl FnMut(&K, &V1)>,
    on_changed: Option<impl FnMut(&K, &V1, &V2)>,
) {
    if let Some(mut on_added) = on_added {
        for (k, v2) in m2 {
            if !m1.contains_key(k) {
                on_added(k, v2);
            }
        }
    }
    if on_changed.is_none() && on_removed.is_none() {
        return;
    }
    let mut on_changed = on_changed;
    let mut on_removed = on_removed;
    for (k, v1) in m1 {
        if let Some(v2) = m2.get(k) {
            if let Some(on_changed) = on_changed.as_mut() {
                if !equal_values(v1, v2) {
                    on_changed(k, v1, v2);
                }
            }
        } else if let Some(on_removed) = on_removed.as_mut() {
            on_removed(k, v1);
        }
    }
}

// CopyMapInto is maps.Copy, unless dst is nil, in which case it clones and returns src.
pub fn copy_map_into<K: Hash + Eq + Clone, V: Clone>(dst: Option<FxHashMap<K, V>>, src: &FxHashMap<K, V>) -> FxHashMap<K, V> {
    match dst {
        None => src.clone(),
        Some(mut dst) => {
            #[expect(clippy::iter_over_hash_type, reason = "inserts distinct keys into a map")]
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
            dst
        }
    }
}

// UnorderedEqual returns true if s1 and s2 contain the same elements, regardless of order.
pub fn unordered_equal<T: Hash + Eq>(s1: &[T], s2: &[T]) -> bool {
    if s1.len() != s2.len() {
        return false;
    }
    let mut counts: FxHashMap<&T, i32> = FxHashMap::default();
    for v in s1 {
        *counts.entry(v).or_default() += 1;
    }
    for v in s2 {
        let c = counts.entry(v).or_default();
        *c -= 1;
        if *c < 0 {
            return false;
        }
    }
    true
}

pub fn deduplicate<T: Clone + PartialEq>(slice: &[T]) -> Cow<'_, [T]> {
    if slice.len() > 1 {
        for (i, value) in slice.iter().enumerate() {
            if slice[..i].contains(value) {
                let mut result = slice[..i].to_vec();
                for value in &slice[i + 1..] {
                    if !result.contains(value) {
                        result.push(value.clone());
                    }
                }
                return Cow::Owned(result);
            }
        }
    }
    Cow::Borrowed(slice)
}

pub fn deduplicate_sorted<T: Clone>(slice: &[T], mut is_equal: impl FnMut(&T, &T) -> bool) -> Vec<T> {
    if slice.is_empty() {
        return Vec::new();
    }
    let mut last = &slice[0];
    let mut deduplicated = vec![slice[0].clone()];
    for next in &slice[1..] {
        if is_equal(last, next) {
            continue;
        }

        deduplicated.push(next.clone());
        last = next;
    }

    deduplicated
}

// CompareBooleans treats true as greater than false.
pub fn compare_booleans(a: bool, b: bool) -> i32 {
    if a && !b {
        1
    } else if !a && b {
        -1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        let v = [1, 2, 3, 4];
        assert!(matches!(filter(&v, |x| *x > 0), Cow::Borrowed(_)));
        assert_eq!(&*filter(&v, |x| x % 2 == 0), &[2, 4]);
        assert_eq!(&*splice(&v, 1, 2, &[9]), &[1, 9, 4]);
        assert_eq!(&*splice(&v, -1, 1, &[]), &[1, 2, 3]);
        assert_eq!(&*deduplicate(&[1, 2, 1, 3, 2]), &[1, 2, 3]);
        assert!(same(&v[..], &v[..]));
        assert!(!same(&v[..], &[1, 2, 3, 4][..]));
        let mut s = vec![1, 3, 5];
        insert_sorted(&mut s, 4, |a, b| a - b);
        assert_eq!(s, vec![1, 3, 4, 5]);
        assert_eq!(compute_ecma_line_starts("a\r\nb\nc\u{2028}d\re"), vec![0, 3, 5, 9, 11]);
        assert_eq!(position_to_line_and_byte_offset(4, &[0, 3, 5]), (1, 1));
        assert_eq!(utf16_len("a😀é"), 4);
        assert_eq!(get_script_kind_from_file_name("a.D.TS"), ScriptKind::TS);
    }

    #[test]
    fn line_starts_stop_when_yield_returns_false() {
        let mut starts = Vec::new();
        compute_ecma_line_starts_seq("é\r\n😀\u{2028}next\u{2029}", |pos| {
            starts.push(pos);
            starts.len() < 2
        });
        assert_eq!(starts, [0, 4]);
    }

    #[test]
    fn line_start_count() {
        for text in [
            "",
            "a",
            "\n",
            "\r",
            "\r\n",
            "\n\r",
            "a\r\nb\nc\u{2028}d\re",
            "x\r",
            "\r\r\n\n",
            "\u{2029}\u{2028}",
            "é\u{2020}\u{2027}\u{202A}\u{2029}é\r\n",
            "\u{2028}\r\n\u{E280}",
        ] {
            let mut expected = 0;
            compute_ecma_line_starts_seq(text, |_| {
                expected += 1;
                true
            });
            assert_eq!(count_ecma_line_starts(text), expected, "{text:?}");
        }
    }

    #[test]
    fn spelling() {
        let names = ["foo", "bar", "length", "lengthy_name"];
        assert_eq!(get_spelling_suggestion_for_strings("lenght", names.iter().copied()), Some("length"));
        assert_eq!(get_spelling_suggestion_for_strings("xyzzy", names.iter().copied()), None);
        assert_eq!(get_spelling_suggestion_for_strings("Foo", names.iter().copied()), Some("foo"));
    }
}
