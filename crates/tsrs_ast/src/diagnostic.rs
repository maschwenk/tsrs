use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::tspath::Path;
use tsrs_core::{alloc_slice, alloc_str, undefined_text_range, OwnedCell, ResolutionMode, TextPos, TextRange, P};
use tsrs_diagnostics::{self as diagnostics, Category, Key, Message};

use crate::SourceFile;

// RepopulateDiagnosticKind indicates the kind of repopulation for a diagnostic chain entry.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RepopulateDiagnosticKind {
    ModeMismatch = 1,
    ModuleNotFound = 2,
}

// RepopulateDiagnosticInfo stores information needed to recompute a diagnostic chain entry
// during incremental builds when the program state may have changed.
#[derive(Clone, Debug)]
pub struct RepopulateDiagnosticInfo {
    pub kind: RepopulateDiagnosticKind,
    pub module_reference: String,
    pub mode: ResolutionMode,
    pub package_name: String,
}

// Diagnostic

pub struct Diagnostic {
    file: OwnedCell<Option<P<SourceFile>>>,
    loc: OwnedCell<TextRange>,
    code: i32,
    category: OwnedCell<Category>,
    // source, when non-empty, is a custom prefix (e.g. a content mapper's name) shown instead of "TS"
    // before the code. It marks the diagnostic as coming from an external source whose ranges point
    // into the file's original, untransformed text.
    source: OwnedCell<&'static str>,
    // Original message; may be nil.
    message: Option<&'static Message>,
    // messageText is an already-localized message used when message is nil, e.g. a diagnostic
    // deserialized from an external process that owns its own localization.
    message_text: OwnedCell<&'static str>,
    message_key: Key,
    message_args: Vec<String>,
    message_chain: OwnedCell<&'static [P<Diagnostic>]>,
    related_information: OwnedCell<&'static [P<Diagnostic>]>,
    reports_unnecessary: bool,
    reports_deprecated: bool,
    skipped_on_no_emit: OwnedCell<bool>,
    repopulate_info: OwnedCell<Option<&'static RepopulateDiagnosticInfo>>,
}

/// Census builds: the scalar fields and the padding after them (`crate::census_layouts`).
pub(crate) fn census_layout() {
    use std::mem::{offset_of, size_of};
    let offsets = [
        offset_of!(Diagnostic, file),
        offset_of!(Diagnostic, loc),
        offset_of!(Diagnostic, code),
        offset_of!(Diagnostic, category),
        offset_of!(Diagnostic, source),
        offset_of!(Diagnostic, message),
        offset_of!(Diagnostic, message_text),
        offset_of!(Diagnostic, message_key),
        offset_of!(Diagnostic, message_args),
        offset_of!(Diagnostic, message_chain),
        offset_of!(Diagnostic, related_information),
        offset_of!(Diagnostic, reports_unnecessary),
        offset_of!(Diagnostic, reports_deprecated),
        offset_of!(Diagnostic, skipped_on_no_emit),
        offset_of!(Diagnostic, repopulate_info),
    ];
    let size = size_of::<Diagnostic>();
    let scalars = [
        offset_of!(Diagnostic, loc),
        offset_of!(Diagnostic, code),
        offset_of!(Diagnostic, category),
        offset_of!(Diagnostic, reports_unnecessary),
        offset_of!(Diagnostic, reports_deprecated),
        offset_of!(Diagnostic, skipped_on_no_emit),
    ];
    let fields: Vec<_> = scalars.iter().map(|&o| tsrs_core::CensusField::scalar(0, o, &offsets, size)).collect();
    tsrs_core::census_layout(std::any::type_name::<Diagnostic>(), &fields);
}

impl Diagnostic {
    pub fn file(&self) -> Option<P<SourceFile>> {
        self.file.get()
    }
    pub fn pos(&self) -> i32 {
        self.loc.get().pos()
    }
    pub fn end(&self) -> i32 {
        self.loc.get().end()
    }
    pub fn len(&self) -> i32 {
        self.loc.get().len()
    }
    pub fn loc(&self) -> TextRange {
        self.loc.get()
    }
    pub fn code(&self) -> i32 {
        self.code
    }
    pub fn category(&self) -> Category {
        self.category.get()
    }
    pub fn source(&self) -> &'static str {
        self.source.get()
    }
    pub fn message(&self) -> Option<&'static Message> {
        self.message
    }
    pub fn message_text(&self) -> &'static str {
        self.message_text.get()
    }
    pub fn message_key(&self) -> Key {
        self.message_key
    }
    pub fn message_args(&self) -> &[String] {
        &self.message_args
    }
    pub fn message_chain(&self) -> &'static [P<Diagnostic>] {
        self.message_chain.get()
    }
    pub fn related_information(&self) -> &'static [P<Diagnostic>] {
        self.related_information.get()
    }
    pub fn reports_unnecessary(&self) -> bool {
        self.reports_unnecessary
    }
    pub fn reports_deprecated(&self) -> bool {
        self.reports_deprecated
    }
    pub fn skipped_on_no_emit(&self) -> bool {
        self.skipped_on_no_emit.get()
    }
    pub fn repopulate_info(&self) -> Option<&'static RepopulateDiagnosticInfo> {
        self.repopulate_info.get()
    }

    pub fn set_file(&self, file: Option<P<SourceFile>>) {
        self.file.set(file)
    }
    pub fn set_location(&self, loc: TextRange) {
        self.loc.set(loc)
    }
    pub fn set_category(&self, category: Category) {
        self.category.set(category)
    }
    pub fn set_skipped_on_no_emit(&self) {
        self.skipped_on_no_emit.set(true)
    }
    pub fn set_repopulate_info(&self, info: RepopulateDiagnosticInfo) {
        self.repopulate_info.set(Some(tsrs_core::alloc(info)))
    }
}

// Go's chaining setters return the *Diagnostic; `P` is a foreign type, so they live on an extension trait.
pub trait DiagnosticExt {
    fn set_external_data(self, source: &str, message_text: &str) -> P<Diagnostic>;
    fn set_message_chain(self, message_chain: &[P<Diagnostic>]) -> P<Diagnostic>;
    fn add_message_chain(self, message_chain: impl Into<Option<P<Diagnostic>>>) -> P<Diagnostic>;
    fn set_related_info(self, related_information: &[P<Diagnostic>]) -> P<Diagnostic>;
    fn add_related_info(self, related_information: impl Into<Option<P<Diagnostic>>>) -> P<Diagnostic>;
    fn clone_diagnostic(self) -> P<Diagnostic>;
}

impl DiagnosticExt for P<Diagnostic> {
    fn set_external_data(self, source: &str, message_text: &str) -> P<Diagnostic> {
        // Diagnostics outlive a scratch region (one file's emit) they are reported in.
        let _outer = tsrs_core::arena::escape_scratch();
        self.source.set(alloc_str(source));
        self.message_text.set(alloc_str(message_text));
        self
    }

    fn set_message_chain(self, message_chain: &[P<Diagnostic>]) -> P<Diagnostic> {
        // Diagnostics outlive a scratch region (one file's emit) they are reported in.
        let _outer = tsrs_core::arena::escape_scratch();
        self.message_chain.set(alloc_slice(message_chain));
        self
    }

    fn add_message_chain(self, message_chain: impl Into<Option<P<Diagnostic>>>) -> P<Diagnostic> {
        // Diagnostics outlive a scratch region (one file's emit) they are reported in.
        let _outer = tsrs_core::arena::escape_scratch();
        if let Some(message_chain) = message_chain.into() {
            let mut chain = self.message_chain.get().to_vec();
            chain.push(message_chain);
            self.message_chain.set(alloc_slice(&chain));
        }
        self
    }

    fn set_related_info(self, related_information: &[P<Diagnostic>]) -> P<Diagnostic> {
        // Diagnostics outlive a scratch region (one file's emit) they are reported in.
        let _outer = tsrs_core::arena::escape_scratch();
        self.related_information.set(alloc_slice(related_information));
        self
    }

    fn add_related_info(self, related_information: impl Into<Option<P<Diagnostic>>>) -> P<Diagnostic> {
        // Diagnostics outlive a scratch region (one file's emit) they are reported in.
        let _outer = tsrs_core::arena::escape_scratch();
        if let Some(related_information) = related_information.into() {
            let mut related = self.related_information.get().to_vec();
            related.push(related_information);
            self.related_information.set(alloc_slice(&related));
        }
        self
    }

    fn clone_diagnostic(self) -> P<Diagnostic> {
        // Diagnostics outlive a scratch region (one file's emit) they are reported in.
        let _outer = tsrs_core::arena::escape_scratch();
        P::new(Diagnostic {
            file: OwnedCell::new(self.file.get()),
            loc: OwnedCell::new(self.loc.get()),
            code: self.code,
            category: OwnedCell::new(self.category.get()),
            source: OwnedCell::new(self.source.get()),
            message: self.message,
            message_text: OwnedCell::new(self.message_text.get()),
            message_key: self.message_key,
            message_args: self.message_args.clone(),
            message_chain: OwnedCell::new(self.message_chain.get()),
            related_information: OwnedCell::new(self.related_information.get()),
            reports_unnecessary: self.reports_unnecessary,
            reports_deprecated: self.reports_deprecated,
            skipped_on_no_emit: OwnedCell::new(self.skipped_on_no_emit.get()),
            repopulate_info: OwnedCell::new(self.repopulate_info.get()),
        })
    }
}

impl Diagnostic {
    // Go `Localize(locale)`; only English messages are ported.
    pub fn localize(&self) -> String {
        if self.message.is_none() && !self.message_text.get().is_empty() {
            return self.message_text.get().to_string();
        }
        diagnostics::localize(self.message, self.message_key, &self.display_message_args())
    }

    // displayMessageArgs substitutes the original text for a complete alias span when a diagnostic argument
    // exactly matches the virtual alias. Stored arguments remain unchanged for code fixes and serialization.
    // diagnostic.go:134
    fn display_message_args(&self) -> Cow<'_, [String]> {
        let Some(file) = self.file.get() else {
            return Cow::Borrowed(&self.message_args);
        };
        if !self.source.get().is_empty() {
            return Cow::Borrowed(&self.message_args);
        }
        let Some(segment) = tsrs_spanmap::alias_for_virtual_span(file.span_map().as_deref(), self.loc.get()) else {
            return Cow::Borrowed(&self.message_args);
        };
        let virtual_text = file.text();
        let original_text = file.original_text();
        if segment.virtual_start < 0
            || segment.virtual_end > virtual_text.len() as TextPos
            || segment.original_start < 0
            || segment.original_end > original_text.len() as TextPos
        {
            return Cow::Borrowed(&self.message_args);
        }
        // Go slices at byte offsets, which need not fall on character boundaries (the original name is then
        // converted lossily).
        let virtual_name = &virtual_text.as_bytes()[segment.virtual_start as usize..segment.virtual_end as usize];
        let original_name = &original_text.as_bytes()[segment.original_start as usize..segment.original_end as usize];
        let mut result: Option<Vec<String>> = None;
        for (i, arg) in self.message_args.iter().enumerate() {
            if arg.as_bytes() != virtual_name {
                continue;
            }
            let result = result.get_or_insert_with(|| self.message_args.clone());
            result[i] = tsrs_core::utf8::from_utf8_lossy(original_name).into_owned();
        }
        match result {
            Some(result) => Cow::Owned(result),
            None => Cow::Borrowed(&self.message_args),
        }
    }
}

// For debugging only.
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localize())
    }
}

impl fmt::Debug for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Diagnostic({}, {:?}, {})", self.code, self.loc.get(), self.localize())
    }
}

pub fn new_diagnostic(file: Option<P<SourceFile>>, loc: TextRange, message: &'static Message, args: &[&dyn fmt::Display]) -> P<Diagnostic> {
    // Diagnostics outlive a scratch region (one file's emit) they are reported in.
    let _outer = tsrs_core::arena::escape_scratch();
    P::new(Diagnostic {
        file: OwnedCell::new(file),
        loc: OwnedCell::new(loc),
        code: message.code(),
        category: OwnedCell::new(message.category()),
        source: OwnedCell::new(""),
        message: Some(message),
        message_text: OwnedCell::new(""),
        message_key: message.key(),
        message_args: diagnostics::stringify_args(args),
        message_chain: OwnedCell::new(&[]),
        related_information: OwnedCell::new(&[]),
        reports_unnecessary: message.reports_unnecessary(),
        reports_deprecated: message.reports_deprecated(),
        skipped_on_no_emit: OwnedCell::new(false),
        repopulate_info: OwnedCell::new(None),
    })
}

// diagnostic.go:166
pub fn new_diagnostic_from_serialized(
    file: Option<P<SourceFile>>,
    loc: TextRange,
    code: i32,
    category: Category,
    message_key: Key,
    message_args: Vec<String>,
    message_chain: &[P<Diagnostic>],
    related_information: &[P<Diagnostic>],
    reports_unnecessary: bool,
    reports_deprecated: bool,
    skipped_on_no_emit: bool,
) -> P<Diagnostic> {
    // Diagnostics outlive a scratch region (one file's emit) they are reported in.
    let _outer = tsrs_core::arena::escape_scratch();
    P::new(Diagnostic {
        file: OwnedCell::new(file),
        loc: OwnedCell::new(loc),
        code,
        category: OwnedCell::new(category),
        source: OwnedCell::new(""),
        message: None,
        message_text: OwnedCell::new(""),
        message_key,
        message_args,
        message_chain: OwnedCell::new(alloc_slice(message_chain)),
        related_information: OwnedCell::new(alloc_slice(related_information)),
        reports_unnecessary,
        reports_deprecated,
        skipped_on_no_emit: OwnedCell::new(skipped_on_no_emit),
        repopulate_info: OwnedCell::new(None),
    })
}

pub fn new_diagnostic_chain(chain: impl Into<Option<P<Diagnostic>>>, message: &'static Message, args: &[&dyn fmt::Display]) -> P<Diagnostic> {
    if let Some(chain) = chain.into() {
        return new_diagnostic(chain.file.get(), chain.loc.get(), message, args)
            .add_message_chain(chain)
            .set_related_info(chain.related_information.get());
    }
    new_diagnostic(None, TextRange::default(), message, args)
}

pub fn new_compiler_diagnostic(message: &'static Message, args: &[&dyn fmt::Display]) -> P<Diagnostic> {
    new_diagnostic(None, undefined_text_range(), message, args)
}

// NewExternalDiagnostic creates a diagnostic reported by an external source such as a content mapper.
// The message text is already localized (the external source owns localization) and the code is shown
// with the given source prefix (e.g. "vue") instead of "TS". The location refers to the file's original,
// untransformed content.
pub fn new_external_diagnostic(
    file: Option<P<SourceFile>>,
    loc: TextRange,
    source: &str,
    category: Category,
    code: i32,
    message_text: &str,
) -> P<Diagnostic> {
    // Diagnostics outlive a scratch region (one file's emit) they are reported in.
    let _outer = tsrs_core::arena::escape_scratch();
    P::new(Diagnostic {
        file: OwnedCell::new(file),
        loc: OwnedCell::new(loc),
        code,
        category: OwnedCell::new(category),
        source: OwnedCell::new(alloc_str(source)),
        message: None,
        message_text: OwnedCell::new(alloc_str(message_text)),
        message_key: Key::default(),
        message_args: Vec::new(),
        message_chain: OwnedCell::new(&[]),
        related_information: OwnedCell::new(&[]),
        reports_unnecessary: false,
        reports_deprecated: false,
        skipped_on_no_emit: OwnedCell::new(false),
        repopulate_info: OwnedCell::new(None),
    })
}

// Go guards this with a mutex; in the port each owner holds the collection mutably.
#[derive(Default)]
pub struct DiagnosticsCollection {
    count: usize,
    file_diagnostics: FxHashMap<Path, Vec<P<Diagnostic>>>,
    file_diagnostics_sorted: FxHashSet<Path>,
    non_file_diagnostics: Vec<P<Diagnostic>>,
    non_file_diagnostics_sorted: bool,
    diagnostic_index: FxHashMap<DiagnosticLocationKey, P<Diagnostic>>,
    diagnostic_collisions: FxHashMap<DiagnosticLocationKey, Vec<P<Diagnostic>>>,
}

impl DiagnosticsCollection {
    pub fn add(&mut self, diagnostic: P<Diagnostic>) -> P<Diagnostic> {
        let key = get_diagnostic_location_key(diagnostic);
        if let Some(&existing) = self.diagnostic_index.get(&key) {
            if equal_diagnostics(existing, diagnostic) {
                return existing;
            }
            if let Some(collisions) = self.diagnostic_collisions.get(&key) {
                for &collision in collisions {
                    if equal_diagnostics(collision, diagnostic) {
                        return collision;
                    }
                }
            }
        }
        match self.diagnostic_index.entry(key) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(diagnostic);
            }
            std::collections::hash_map::Entry::Occupied(e) => self.diagnostic_collisions.entry(e.key().clone()).or_default().push(diagnostic),
        }

        self.count += 1;

        if let Some(file) = diagnostic.file() {
            // Look up before cloning: a clone of the file's `Path` (an `Arc`) is an atomic increment on a counter that
            // every checker adding a diagnostic of that file writes.
            let path = file.path();
            match self.file_diagnostics.get_mut(path) {
                Some(diagnostics) => diagnostics.push(diagnostic),
                None => {
                    self.file_diagnostics.insert(path.clone(), vec![diagnostic]);
                }
            }
            self.file_diagnostics_sorted.remove(path);
        } else {
            self.non_file_diagnostics.push(diagnostic);
            self.non_file_diagnostics_sorted = false;
        }
        diagnostic
    }

    pub fn lookup(&mut self, diagnostic: P<Diagnostic>) -> Option<P<Diagnostic>> {
        let diagnostics = if let Some(file) = diagnostic.file() {
            self.get_diagnostics_for_file_locked(file)
        } else {
            self.get_global_diagnostics_locked()
        };
        let (i, ok) = crate::binary_search_func(&diagnostics, diagnostic, compare_diagnostics);
        if ok {
            return Some(diagnostics[i]);
        }
        None
    }

    pub fn get_global_diagnostics(&mut self) -> Vec<P<Diagnostic>> {
        self.get_global_diagnostics_locked()
    }

    fn get_global_diagnostics_locked(&mut self) -> Vec<P<Diagnostic>> {
        if !self.non_file_diagnostics_sorted {
            self.non_file_diagnostics.sort_by(|a, b| compare_diagnostics(*a, *b).cmp(&0));
            self.non_file_diagnostics_sorted = true;
        }
        self.non_file_diagnostics.clone()
    }

    pub fn get_diagnostics_for_file(&mut self, file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        self.get_diagnostics_for_file_locked(file)
    }

    fn get_diagnostics_for_file_locked(&mut self, file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        let path = file.path().clone();
        if !self.file_diagnostics_sorted.contains(&path) {
            if let Some(diagnostics) = self.file_diagnostics.get_mut(&path) {
                diagnostics.sort_by(|a, b| compare_diagnostics(*a, *b).cmp(&0));
            }
            self.file_diagnostics_sorted.insert(path.clone());
        }
        self.file_diagnostics.get(&path).cloned().unwrap_or_default()
    }

    pub fn get_diagnostics(&self) -> Vec<P<Diagnostic>> {
        let mut diagnostics = Vec::with_capacity(self.count);
        diagnostics.extend_from_slice(&self.non_file_diagnostics);
        #[expect(
            clippy::iter_over_hash_type,
            reason = "sorted by compare_diagnostics below, which orders by file first; the stable sort keeps each file's own order"
        )]
        for diags in self.file_diagnostics.values() {
            diagnostics.extend_from_slice(diags);
        }
        diagnostics.sort_by(|a, b| compare_diagnostics(*a, *b).cmp(&0));
        diagnostics
    }
}

/// Keyed by the file's path as Go keys by file name; it holds the file and compares and hashes its path, so that
/// building a key does not clone the path's `Arc` (a contended atomic when many checkers report diagnostics of one
/// file: drizzle-orm's 10,846 errors at 32 checkers).
#[derive(Clone)]
struct DiagnosticLocationKey {
    file: Option<P<SourceFile>>,
    loc: TextRange,
    code: i32,
}

impl PartialEq for DiagnosticLocationKey {
    fn eq(&self, other: &Self) -> bool {
        self.loc == other.loc && self.code == other.code && self.file.map(|f| f.get().path()) == other.file.map(|f| f.get().path())
    }
}

impl Eq for DiagnosticLocationKey {}

impl std::hash::Hash for DiagnosticLocationKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.file.map(|f| f.get().path()).hash(state);
        self.loc.hash(state);
        self.code.hash(state);
    }
}

fn get_diagnostic_location_key(diagnostic: P<Diagnostic>) -> DiagnosticLocationKey {
    DiagnosticLocationKey { file: diagnostic.file(), loc: diagnostic.loc(), code: diagnostic.code() }
}

fn get_diagnostic_path(d: P<Diagnostic>) -> &'static str {
    match d.file() {
        Some(file) => file.get().file_name(),
        None => "",
    }
}

pub fn equal_diagnostics(d1: P<Diagnostic>, d2: P<Diagnostic>) -> bool {
    if d1 == d2 {
        return true;
    }
    equal_diagnostics_no_related_info(d1, d2) && slices_equal_func(d1.related_information(), d2.related_information(), equal_diagnostics)
}

pub fn equal_diagnostics_no_related_info(d1: P<Diagnostic>, d2: P<Diagnostic>) -> bool {
    if d1 == d2 {
        return true;
    }
    get_diagnostic_path(d1) == get_diagnostic_path(d2)
        && d1.loc() == d2.loc()
        && d1.code() == d2.code()
        && d1.category() == d2.category()
        && d1.source() == d2.source()
        && get_diagnostic_message_identity(d1) == get_diagnostic_message_identity(d2)
        && d1.message_args() == d2.message_args()
        && slices_equal_func(d1.message_chain(), d2.message_chain(), equal_message_chain)
}

fn slices_equal_func(a: &[P<Diagnostic>], b: &[P<Diagnostic>], eq: fn(P<Diagnostic>, P<Diagnostic>) -> bool) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| eq(*x, *y))
}

fn get_diagnostic_message_identity(diagnostic: P<Diagnostic>) -> &'static str {
    if !diagnostic.message_text().is_empty() {
        return diagnostic.message_text();
    }
    if let Some(message) = diagnostic.message {
        if diagnostic.code() == -1 {
            return message.text();
        }
    }
    diagnostic.message_key().as_str()
}

fn equal_message_chain(c1: P<Diagnostic>, c2: P<Diagnostic>) -> bool {
    if c1 == c2 {
        return true;
    }
    c1.code() == c2.code()
        && c1.message_args() == c2.message_args()
        && slices_equal_func(c1.message_chain(), c2.message_chain(), equal_message_chain)
}

fn compare_strings(a: &str, b: &str) -> i32 {
    match a.cmp(b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

// Go `slices.Compare` over []string.
fn compare_string_slices(a: &[String], b: &[String]) -> i32 {
    match a.cmp(b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

fn compare_message_chain_size(c1: &[P<Diagnostic>], c2: &[P<Diagnostic>]) -> i32 {
    let c = c2.len() as i32 - c1.len() as i32;
    if c != 0 {
        return c;
    }
    for i in 0..c1.len() {
        let c = compare_message_chain_size(c1[i].message_chain(), c2[i].message_chain());
        if c != 0 {
            return c;
        }
    }
    0
}

fn compare_message_chain_content(c1: &[P<Diagnostic>], c2: &[P<Diagnostic>]) -> i32 {
    for i in 0..c1.len() {
        let c = compare_string_slices(c1[i].message_args(), c2[i].message_args());
        if c != 0 {
            return c;
        }
        if !c1[i].message_chain().is_empty() {
            let c = compare_message_chain_content(c1[i].message_chain(), c2[i].message_chain());
            if c != 0 {
                return c;
            }
        }
    }
    0
}

fn compare_related_info(r1: &[P<Diagnostic>], r2: &[P<Diagnostic>]) -> i32 {
    let c = r2.len() as i32 - r1.len() as i32;
    if c != 0 {
        return c;
    }
    for i in 0..r1.len() {
        let c = compare_diagnostics(r1[i], r2[i]);
        if c != 0 {
            return c;
        }
    }
    0
}

pub fn compare_diagnostics(d1: P<Diagnostic>, d2: P<Diagnostic>) -> i32 {
    if d1 == d2 {
        return 0;
    }
    let mut c = compare_strings(get_diagnostic_path(d1), get_diagnostic_path(d2));
    if c != 0 {
        return c;
    }
    c = d1.loc().pos() - d2.loc().pos();
    if c != 0 {
        return c;
    }
    c = d1.loc().end() - d2.loc().end();
    if c != 0 {
        return c;
    }
    c = d1.code() - d2.code();
    if c != 0 {
        return c;
    }
    c = d1.category() as i32 - d2.category() as i32;
    if c != 0 {
        return c;
    }
    c = compare_strings(d1.source(), d2.source());
    if c != 0 {
        return c;
    }
    c = compare_strings(get_diagnostic_message_identity(d1), get_diagnostic_message_identity(d2));
    if c != 0 {
        return c;
    }
    c = compare_string_slices(d1.message_args(), d2.message_args());
    if c != 0 {
        return c;
    }
    c = compare_message_chain_size(d1.message_chain(), d2.message_chain());
    if c != 0 {
        return c;
    }
    c = compare_message_chain_content(d1.message_chain(), d2.message_chain());
    if c != 0 {
        return c;
    }
    compare_related_info(d1.related_information(), d2.related_information())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_collection_deduplicates_exact_diagnostics_on_add() {
        let mut collection = DiagnosticsCollection::default();
        let first = new_compiler_diagnostic(&diagnostics::Cannot_find_name_0, &[&"x"])
            .add_related_info(Some(new_compiler_diagnostic(&diagnostics::X_0_is_declared_here, &[&"first"])));
        let second = new_compiler_diagnostic(&diagnostics::Cannot_find_name_0, &[&"x"])
            .add_related_info(Some(new_compiler_diagnostic(&diagnostics::X_0_is_declared_here, &[&"first"])));
        let different = new_compiler_diagnostic(&diagnostics::Cannot_find_name_0, &[&"x"])
            .add_related_info(Some(new_compiler_diagnostic(&diagnostics::X_0_is_declared_here, &[&"second"])));

        assert_eq!(collection.add(first), first);
        let canonical = collection.add(second);
        assert_eq!(canonical, first);
        assert_eq!(collection.add(different), different);

        canonical.add_related_info(Some(new_compiler_diagnostic(&diagnostics::X_0_is_declared_here, &[&"third"])));
        let collected = collection.get_global_diagnostics();
        assert_eq!(collected.len(), 2);
        assert_eq!(first.related_information().len(), 2);
    }

    #[test]
    fn diagnostics_collection_preserves_distinct_ad_hoc_messages() {
        let mut collection = DiagnosticsCollection::default();
        let first = new_compiler_diagnostic(diagnostics::new_ad_hoc_message("first"), &[]);
        let second = new_compiler_diagnostic(diagnostics::new_ad_hoc_message("second"), &[]);
        collection.add(first);
        collection.add(second);
        assert_eq!(collection.get_global_diagnostics().len(), 2);
    }

    #[test]
    fn external_diagnostic_identity() {
        let loc = TextRange::new(1, 2);
        let first = new_external_diagnostic(None, loc, "mapper-a", Category::Error, 0, "first");
        let all = [
            first,
            new_external_diagnostic(None, loc, "mapper-a", Category::Error, 0, "second"),
            new_external_diagnostic(None, loc, "mapper-b", Category::Error, 0, "first"),
            new_external_diagnostic(None, loc, "mapper-a", Category::Warning, 0, "first"),
        ];
        let mut collection = DiagnosticsCollection::default();
        for &diagnostic in &all {
            assert!(!equal_diagnostics_no_related_info(first, diagnostic) || diagnostic == first);
            assert!(compare_diagnostics(first, diagnostic) != 0 || diagnostic == first);
            collection.add(diagnostic);
        }
        assert_eq!(collection.get_diagnostics().len(), all.len());
    }

    // displayMessageArgs (diagnostic.go:134): a diagnostic on exactly a virtual alias shows the original name instead of
    // the virtual one; a partial span or an external source shows the stored arguments, which never change.
    #[test]
    fn display_message_args_substitutes_complete_alias() {
        let f = crate::NodeFactory::default();
        let statements = f.new_node_list(vec![]);
        let eof = f.new_token(crate::Kind::EndOfFile);
        let opts = crate::SourceFileParseOptions { file_name: "/a.vue".to_string(), ..Default::default() };
        let file = f.new_source_file(opts.clone(), "let __alias = 1;", statements, eof).as_source_file_p();
        let span_map = tsrs_spanmap::new(&[tsrs_spanmap::Segment {
            virtual_start: 4,
            virtual_end: 11,
            original_start: 5,
            original_end: 6,
            kind: tsrs_spanmap::Kind::Alias,
            features: tsrs_spanmap::Feature::All,
        }]);
        file.set_content_mapper_info(crate::ContentMapperSourceFileInfo {
            content_mapper: "mapper@1.0.0",
            parse_options: opts,
            original_text: "<a b=x>",
            span_map: Some(span_map),
            ..Default::default()
        });

        let alias = new_diagnostic(Some(file), TextRange::new(4, 11), &diagnostics::Cannot_find_name_0, &[&"__alias"]);
        assert_eq!(alias.localize(), "Cannot find name 'x'.");
        assert_eq!(alias.message_args(), ["__alias"]);
        let partial = new_diagnostic(Some(file), TextRange::new(5, 11), &diagnostics::Cannot_find_name_0, &[&"__alias"]);
        assert_eq!(partial.localize(), "Cannot find name '__alias'.");
        let external = new_diagnostic(Some(file), TextRange::new(4, 11), &diagnostics::Cannot_find_name_0, &[&"__alias"])
            .set_external_data("mapper", "");
        assert_eq!(external.localize(), "Cannot find name '__alias'.");
    }
}

// diagnostic.go NewDiagnosticFromText: a diagnostic whose message is already-localized text (the native API
// receives these from clients, e.g. `configFileParsingDiagnostics`).
pub fn new_diagnostic_from_text(
    file: Option<P<SourceFile>>,
    loc: TextRange,
    code: i32,
    category: Category,
    message_text: &str,
    message_chain: &[P<Diagnostic>],
    related_information: &[P<Diagnostic>],
    reports_unnecessary: bool,
    reports_deprecated: bool,
) -> P<Diagnostic> {
    P::new(Diagnostic {
        file: OwnedCell::new(file),
        loc: OwnedCell::new(loc),
        code,
        category: OwnedCell::new(category),
        source: OwnedCell::new(""),
        message: None,
        message_text: OwnedCell::new(alloc_str(message_text)),
        message_key: Key::default(),
        message_args: Vec::new(),
        message_chain: OwnedCell::new(alloc_slice(message_chain)),
        related_information: OwnedCell::new(alloc_slice(related_information)),
        reports_unnecessary,
        reports_deprecated,
        skipped_on_no_emit: OwnedCell::new(false),
        repopulate_info: OwnedCell::new(None),
    })
}
