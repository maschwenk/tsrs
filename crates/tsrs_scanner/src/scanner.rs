//! Scanner API notes for consumers (parser, checker):
//!
//! Text and positions. The scanner scans a `&'static str` (the file's source text: leaked, embedded,
//! or copied into the current arena or region).
//! Positions are byte offsets (`i32`), exactly as in Go.
//!
//! Token values. `token_value()` returns `&'static str`. When the Go scanner would produce a
//! substring of the source text (the overwhelmingly common case: identifiers, keywords, simple
//! strings, numbers that are already canonical) this is a slice of the source text and costs
//! nothing. When the value is computed (escape sequences, numeric separators, normalized numbers,
//! template literals containing `\r`) it is allocated with `alloc_str` in the current allocation
//! target (the thread arena, or the entered region), where the parser also allocates nodes, so the
//! parser can store token values in nodes directly without copying.
//!
//! Errors (Go `ErrorCallback` / `SetOnError`). Go's scanner calls a callback synchronously. In
//! Rust the parser owns the scanner and is itself a `&mut self` state machine, so the scanner
//! instead *buffers* errors: `set_on_error(true)` enables reporting (Go: a non-nil callback),
//! each reported error is appended to an internal `Vec<ScanError>`, and the owner drains it
//! with `take_errors()` (or checks `has_errors()` first, which is the cheap fast path).
//! To preserve Go's diagnostic order exactly, the owner must drain right after every scanner
//! call that may report (`scan`, every `re_scan_*`, `scan_jsx_*`, `scan_jsdoc_*`, ...), before it
//! reports any diagnostics of its own. Message arguments are pre-formatted into `String`s
//! (`ScanError::args`); pass them on as `&[&dyn Display]`.
//!
//! Speculation (Go `Mark` / `Rewind`). `mark()` returns a `Copy` `ScannerState` snapshot
//! (positions, token, token value, flags, JSDoc asterisk depth and the slice of comment directives
//! collected so far); `rewind(state)` restores it. Adding a directive copies the slice into a new
//! arena slice, so a saved snapshot is never modified, which is what Go's shared-backing-array
//! slice restore amounts to. Buffered
//! errors are *not* part of the state (in Go they are the callback's business): an owner that
//! drains after every call has nothing pending at `mark()` time and truncates its own diagnostics
//! on rewind, as the Go parser does.

use std::borrow::Cow;
use std::fmt::Display;
use std::sync::OnceLock;

use rustc_hash::FxHashMap;
use tsrs_ast as ast;
use tsrs_ast::{CommentDirective, CommentDirectiveKind, CommentRange, Kind, Node, NodeFlags, SourceFile, SourceFileLike, TokenFlags};
use tsrs_core::{alloc_str, jsnum, stringutil, LanguageVariant, ScriptTarget, TextPos, TextRange, UTF16Offset, P};
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;

use crate::regexp::{char_code_to_reg_exp_flag, RegExpParser, RegularExpressionFlags};
use crate::utilities::token_is_identifier_or_keyword;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum IdentifierVariant {
    Standard,
    JSX,
    RegExpGroupName,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub struct EscapeSequenceScanningFlags: i32 {
        const String = 1 << 0;
        const ReportErrors = 1 << 1;
        const RegularExpression = 1 << 2;
        const AnnexB = 1 << 3;
        const AnyUnicodeMode = 1 << 4;
        const AtomEscape = 1 << 5;
        const ReportInvalidEscapeErrors = Self::RegularExpression.bits() | Self::ReportErrors.bits();
        const AllowExtendedUnicodeEscape = Self::String.bits() | Self::AnyUnicodeMode.bits();
    }
}

/// A diagnostic reported by the scanner (Go: the arguments of one `ErrorCallback` call).
#[derive(Clone, Debug)]
pub struct ScanError {
    pub message: &'static Message,
    pub start: i32,
    pub length: i32,
    pub args: Vec<String>,
}

macro_rules! keywords {
    ($($text:literal => $kind:ident,)*) => {
        const KEYWORDS: &[(&str, Kind)] = &[$(($text, Kind::$kind),)*];
        pub(crate) static TEXT_TO_KEYWORD: &[(&str, Kind)] = KEYWORDS;
    };
}

// Go `textToKeyword` is a map. The port looks keywords up in a perfect hash over the first two bytes, the last
// byte and the length (every keyword has at least two bytes); KEYWORD_HASH_MUL was searched for the current
// keyword list, and the table is built at compile time, which fails if two keywords collide.
const KEYWORD_HASH_MUL: u32 = 0xae8526c7;
const NO_KEYWORD: u8 = u8::MAX;

#[inline]
const fn keyword_hash(b: &[u8]) -> usize {
    let key = b[0] as u32 | (b[1] as u32) << 8 | (b[b.len() - 1] as u32) << 16 | (b.len() as u32) << 24;
    (key.wrapping_mul(KEYWORD_HASH_MUL) >> 24) as usize
}

static KEYWORD_TABLE: [u8; 256] = {
    let mut table = [NO_KEYWORD; 256];
    let mut i = 0;
    while i < KEYWORDS.len() {
        let h = keyword_hash(KEYWORDS[i].0.as_bytes());
        assert!(table[h] == NO_KEYWORD, "keyword hash collision: search a new KEYWORD_HASH_MUL");
        table[h] = i as u8;
        i += 1;
    }
    table
};

/// Go `textToKeyword[s]` (zero value `KindUnknown` when absent).
#[inline]
pub(crate) fn text_to_keyword(s: &str) -> Kind {
    let b = s.as_bytes();
    if b.len() < 2 {
        return Kind::Unknown;
    }
    let i = KEYWORD_TABLE[keyword_hash(b)];
    if i != NO_KEYWORD {
        let (text, kind) = KEYWORDS[i as usize];
        if text.as_bytes() == b {
            return kind;
        }
    }
    Kind::Unknown
}

keywords! {
    "abstract" => AbstractKeyword,
    "accessor" => AccessorKeyword,
    "any" => AnyKeyword,
    "as" => AsKeyword,
    "asserts" => AssertsKeyword,
    "assert" => AssertKeyword,
    "bigint" => BigIntKeyword,
    "boolean" => BooleanKeyword,
    "break" => BreakKeyword,
    "case" => CaseKeyword,
    "catch" => CatchKeyword,
    "class" => ClassKeyword,
    "continue" => ContinueKeyword,
    "const" => ConstKeyword,
    "constructor" => ConstructorKeyword,
    "debugger" => DebuggerKeyword,
    "declare" => DeclareKeyword,
    "default" => DefaultKeyword,
    "defer" => DeferKeyword,
    "delete" => DeleteKeyword,
    "do" => DoKeyword,
    "else" => ElseKeyword,
    "enum" => EnumKeyword,
    "export" => ExportKeyword,
    "extends" => ExtendsKeyword,
    "false" => FalseKeyword,
    "finally" => FinallyKeyword,
    "for" => ForKeyword,
    "from" => FromKeyword,
    "function" => FunctionKeyword,
    "get" => GetKeyword,
    "if" => IfKeyword,
    "immediate" => ImmediateKeyword,
    "implements" => ImplementsKeyword,
    "import" => ImportKeyword,
    "in" => InKeyword,
    "infer" => InferKeyword,
    "instanceof" => InstanceOfKeyword,
    "interface" => InterfaceKeyword,
    "intrinsic" => IntrinsicKeyword,
    "is" => IsKeyword,
    "keyof" => KeyOfKeyword,
    "let" => LetKeyword,
    "module" => ModuleKeyword,
    "namespace" => NamespaceKeyword,
    "never" => NeverKeyword,
    "new" => NewKeyword,
    "null" => NullKeyword,
    "number" => NumberKeyword,
    "object" => ObjectKeyword,
    "package" => PackageKeyword,
    "private" => PrivateKeyword,
    "protected" => ProtectedKeyword,
    "public" => PublicKeyword,
    "override" => OverrideKeyword,
    "out" => OutKeyword,
    "readonly" => ReadonlyKeyword,
    "require" => RequireKeyword,
    "global" => GlobalKeyword,
    "return" => ReturnKeyword,
    "satisfies" => SatisfiesKeyword,
    "set" => SetKeyword,
    "static" => StaticKeyword,
    "string" => StringKeyword,
    "super" => SuperKeyword,
    "switch" => SwitchKeyword,
    "symbol" => SymbolKeyword,
    "this" => ThisKeyword,
    "throw" => ThrowKeyword,
    "true" => TrueKeyword,
    "try" => TryKeyword,
    "type" => TypeKeyword,
    "typeof" => TypeOfKeyword,
    "undefined" => UndefinedKeyword,
    "unique" => UniqueKeyword,
    "unknown" => UnknownKeyword,
    "using" => UsingKeyword,
    "var" => VarKeyword,
    "void" => VoidKeyword,
    "while" => WhileKeyword,
    "with" => WithKeyword,
    "yield" => YieldKeyword,
    "async" => AsyncKeyword,
    "await" => AwaitKeyword,
    "of" => OfKeyword,
}

static TEXT_TO_PUNCTUATION: &[(&str, Kind)] = &[
    ("{", Kind::OpenBraceToken),
    ("}", Kind::CloseBraceToken),
    ("(", Kind::OpenParenToken),
    (")", Kind::CloseParenToken),
    ("[", Kind::OpenBracketToken),
    ("]", Kind::CloseBracketToken),
    (".", Kind::DotToken),
    ("...", Kind::DotDotDotToken),
    (";", Kind::SemicolonToken),
    (",", Kind::CommaToken),
    ("<", Kind::LessThanToken),
    (">", Kind::GreaterThanToken),
    ("<=", Kind::LessThanEqualsToken),
    (">=", Kind::GreaterThanEqualsToken),
    ("==", Kind::EqualsEqualsToken),
    ("!=", Kind::ExclamationEqualsToken),
    ("===", Kind::EqualsEqualsEqualsToken),
    ("!==", Kind::ExclamationEqualsEqualsToken),
    ("=>", Kind::EqualsGreaterThanToken),
    ("+", Kind::PlusToken),
    ("-", Kind::MinusToken),
    ("**", Kind::AsteriskAsteriskToken),
    ("*", Kind::AsteriskToken),
    ("/", Kind::SlashToken),
    ("%", Kind::PercentToken),
    ("++", Kind::PlusPlusToken),
    ("--", Kind::MinusMinusToken),
    ("<<", Kind::LessThanLessThanToken),
    ("</", Kind::LessThanSlashToken),
    (">>", Kind::GreaterThanGreaterThanToken),
    (">>>", Kind::GreaterThanGreaterThanGreaterThanToken),
    ("&", Kind::AmpersandToken),
    ("|", Kind::BarToken),
    ("^", Kind::CaretToken),
    ("!", Kind::ExclamationToken),
    ("~", Kind::TildeToken),
    ("&&", Kind::AmpersandAmpersandToken),
    ("||", Kind::BarBarToken),
    ("?", Kind::QuestionToken),
    ("??", Kind::QuestionQuestionToken),
    ("?.", Kind::QuestionDotToken),
    (":", Kind::ColonToken),
    ("=", Kind::EqualsToken),
    ("+=", Kind::PlusEqualsToken),
    ("-=", Kind::MinusEqualsToken),
    ("*=", Kind::AsteriskEqualsToken),
    ("**=", Kind::AsteriskAsteriskEqualsToken),
    ("/=", Kind::SlashEqualsToken),
    ("%=", Kind::PercentEqualsToken),
    ("<<=", Kind::LessThanLessThanEqualsToken),
    (">>=", Kind::GreaterThanGreaterThanEqualsToken),
    (">>>=", Kind::GreaterThanGreaterThanGreaterThanEqualsToken),
    ("&=", Kind::AmpersandEqualsToken),
    ("|=", Kind::BarEqualsToken),
    ("^=", Kind::CaretEqualsToken),
    ("||=", Kind::BarBarEqualsToken),
    ("&&=", Kind::AmpersandAmpersandEqualsToken),
    ("??=", Kind::QuestionQuestionEqualsToken),
    ("@", Kind::AtToken),
    ("#", Kind::HashToken),
    ("`", Kind::BacktickToken),
];

fn text_to_token() -> &'static FxHashMap<&'static str, Kind> {
    static M: OnceLock<FxHashMap<&'static str, Kind>> = OnceLock::new();
    M.get_or_init(|| {
        let mut m = FxHashMap::default();
        for &(text, kind) in TEXT_TO_PUNCTUATION {
            m.insert(text, kind);
        }
        for &(text, kind) in TEXT_TO_KEYWORD {
            m.insert(text, kind);
        }
        m
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScannerState {
    pub(crate) pos: i32,                             // Current position in text (and ending position of current token)
    pub(crate) full_start_pos: i32,                  // Starting position of current token including preceding whitespace
    pub(crate) token_start: i32,                     // Starting position of non-whitespace part of current token
    pub(crate) token: Kind,                          // Kind of current token
    pub(crate) token_value: &'static str,            // Parsed value of current token
    pub(crate) token_flags: TokenFlags,              // Flags for current token
    pub(crate) comment_directives: &'static [CommentDirective], // a snapshot, like Go's slice header: Rewind restores it
    pub(crate) skip_jsdoc_leading_asterisks: i32,    // Leading asterisks to skip when scanning types inside JSDoc. Should be 0 outside JSDoc
}

#[derive(Default)]
pub struct Scanner {
    pub(crate) text: &'static str,
    pub(crate) end: i32,
    language_variant: LanguageVariant,
    script_target: ScriptTarget,
    on_error: bool,
    errors: Vec<ScanError>,
    skip_trivia: bool,
    pub(crate) state: ScannerState,

    number_cache: FxHashMap<&'static str, &'static str>,
    hex_number_cache: FxHashMap<&'static str, &'static str>,
    hex_digit_cache: FxHashMap<&'static str, &'static str>,
}

pub(crate) const RUNE_SELF: i32 = 0x80;

/// `[A-Za-z0-9_$]` by byte.
static ASCII_IDENTIFIER_PART: [bool; 256] = {
    let mut t = [false; 256];
    let mut b = 0;
    while b < 128 {
        let c = b as u8;
        t[b] = c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
        b += 1;
    }
    t
};
pub(crate) const RUNE_ERROR: i32 = 0xFFFD;

/// Go `utf8.DecodeRuneInString` over raw bytes: `(RuneError, 0)` for empty input,
/// `(RuneError, 1)` for an invalid encoding.
#[inline]
pub(crate) fn decode_rune(b: &[u8]) -> (i32, usize) {
    let Some(&b0) = b.first() else {
        return (RUNE_ERROR, 0);
    };
    if b0 < 0x80 {
        return (b0 as i32, 1);
    }
    decode_rune_slow(b)
}

#[cold]
fn decode_rune_slow(b: &[u8]) -> (i32, usize) {
    let b0 = b[0] as u32;
    let cont = |i: usize, lo: u8, hi: u8| -> Option<u32> {
        let c = *b.get(i)?;
        if c < lo || c > hi {
            return None;
        }
        Some((c & 0x3F) as u32)
    };
    match b0 {
        0xC2..=0xDF => match cont(1, 0x80, 0xBF) {
            Some(c1) => ((((b0 & 0x1F) << 6) | c1) as i32, 2),
            None => (RUNE_ERROR, 1),
        },
        0xE0..=0xEF => {
            let (lo, hi) = match b0 {
                0xE0 => (0xA0, 0xBF),
                0xED => (0x80, 0x9F),
                _ => (0x80, 0xBF),
            };
            match (cont(1, lo, hi), cont(2, 0x80, 0xBF)) {
                (Some(c1), Some(c2)) => ((((b0 & 0x0F) << 12) | (c1 << 6) | c2) as i32, 3),
                _ => (RUNE_ERROR, 1),
            }
        }
        0xF0..=0xF4 => {
            let (lo, hi) = match b0 {
                0xF0 => (0x90, 0xBF),
                0xF4 => (0x80, 0x8F),
                _ => (0x80, 0xBF),
            };
            match (cont(1, lo, hi), cont(2, 0x80, 0xBF), cont(3, 0x80, 0xBF)) {
                (Some(c1), Some(c2), Some(c3)) => ((((b0 & 0x07) << 18) | (c1 << 12) | (c2 << 6) | c3) as i32, 4),
                _ => (RUNE_ERROR, 1),
            }
        }
        _ => (RUNE_ERROR, 1),
    }
}

/// Go `utf8.DecodeLastRuneInString`.
pub(crate) fn decode_last_rune(b: &[u8]) -> (i32, usize) {
    let end = b.len();
    if end == 0 {
        return (RUNE_ERROR, 0);
    }
    let last = b[end - 1];
    if last < 0x80 {
        return (last as i32, 1);
    }
    let lim = end.saturating_sub(4);
    let mut start = end - 1;
    while start > lim {
        if b[start] & 0xC0 != 0x80 {
            break;
        }
        start -= 1;
    }
    let (r, size) = decode_rune(&b[start..end]);
    if start + size != end {
        return (RUNE_ERROR, 1);
    }
    (r, size)
}

/// Go `string(r)`: invalid code points (surrogates, out of range) become U+FFFD.
#[inline]
pub(crate) fn rune_to_char(r: i32) -> char {
    char::from_u32(r as u32).unwrap_or('\u{FFFD}')
}

#[inline]
pub(crate) fn rune_string(r: i32) -> String {
    rune_to_char(r).to_string()
}

impl Scanner {
    fn default_scanner() -> Scanner {
        Scanner { skip_trivia: true, ..Default::default() }
    }

    pub fn new() -> Scanner {
        Scanner::default_scanner()
    }

    pub fn reset(&mut self) {
        let mut number_cache = std::mem::take(&mut self.number_cache);
        let mut hex_number_cache = std::mem::take(&mut self.hex_number_cache);
        let mut hex_digit_cache = std::mem::take(&mut self.hex_digit_cache);
        number_cache.clear();
        hex_number_cache.clear();
        hex_digit_cache.clear();
        *self = Scanner::default_scanner();
        self.number_cache = number_cache;
        self.hex_number_cache = hex_number_cache;
        self.hex_digit_cache = hex_digit_cache;
    }

    pub fn text(&self) -> &'static str {
        self.text
    }

    /// A number cache entry outlives a rolled-back parse (notes/mem-recycle.md): pin the arena unless both strings are
    /// slices of the source text (or empty), which no rewind can discard. Lazily parsed declaration-file member lists
    /// are discarded by a rewind too (notes/mem-lazy-dts-members.md), and most numbers are source slices.
    fn pin_unless_source(&self, key: &str, value: &str) {
        let in_text = |s: &str| {
            let (p, t) = (s.as_ptr() as usize, self.text.as_ptr() as usize);
            s.is_empty() || (p >= t && p + s.len() <= t + self.text.len())
        };
        if !in_text(key) || !in_text(value) {
            tsrs_core::arena_pin();
        }
    }

    pub fn token(&self) -> Kind {
        self.state.token
    }

    pub fn token_flags(&self) -> TokenFlags {
        self.state.token_flags
    }

    pub fn token_full_start(&self) -> i32 {
        self.state.full_start_pos
    }

    pub fn token_start(&self) -> i32 {
        self.state.token_start
    }

    pub fn token_end(&self) -> i32 {
        self.state.pos
    }

    pub fn token_text(&self) -> &'static str {
        &self.text[self.state.token_start as usize..self.state.pos as usize]
    }

    pub fn token_value(&self) -> &'static str {
        self.state.token_value
    }

    pub fn token_range(&self) -> TextRange {
        TextRange::new(self.state.token_start, self.state.pos)
    }

    pub fn comment_directives(&self) -> &'static [CommentDirective] {
        self.state.comment_directives
    }

    pub fn mark(&self) -> ScannerState {
        self.state
    }

    pub fn rewind(&mut self, state: ScannerState) {
        self.state = state;
    }

    /// Go: `onError != nil`. Enables buffering of scanner diagnostics (see the module docs).
    pub fn set_on_error(&mut self, enabled: bool) {
        self.on_error = enabled;
    }

    #[inline]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Drains the buffered scanner diagnostics in report order.
    pub fn take_errors(&mut self) -> Vec<ScanError> {
        std::mem::take(&mut self.errors)
    }

    pub fn reset_pos(&mut self, pos: i32) {
        if pos < 0 {
            panic!("Cannot reset token state to negative position");
        }
        self.state.pos = pos;
        self.state.full_start_pos = pos;
        self.state.token_start = pos;
    }

    pub fn reset_token_state(&mut self, pos: i32) {
        self.reset_pos(pos);
        self.state.token = Kind::Unknown;
        self.state.token_value = "";
        self.state.token_flags = TokenFlags::None;
    }

    pub fn set_skip_jsdoc_leading_asterisks(&mut self, skip: bool) {
        if skip {
            self.state.skip_jsdoc_leading_asterisks += 1;
        } else {
            self.state.skip_jsdoc_leading_asterisks += -1;
        }
    }

    pub fn set_skip_trivia(&mut self, skip: bool) {
        self.skip_trivia = skip;
    }

    pub fn has_unicode_escape(&self) -> bool {
        self.state.token_flags.intersects(TokenFlags::UnicodeEscape)
    }

    pub fn has_extended_unicode_escape(&self) -> bool {
        self.state.token_flags.intersects(TokenFlags::ExtendedUnicodeEscape)
    }

    pub fn has_preceding_line_break(&self) -> bool {
        self.state.token_flags.intersects(TokenFlags::PrecedingLineBreak)
    }

    pub fn has_preceding_jsdoc_comment(&self) -> bool {
        self.state.token_flags.intersects(TokenFlags::PrecedingJSDocComment)
    }

    pub fn has_preceding_jsdoc_leading_asterisks(&self) -> bool {
        self.state.token_flags.intersects(TokenFlags::PrecedingJSDocLeadingAsterisks)
    }

    pub fn has_preceding_jsdoc_with_deprecated_tag(&self) -> bool {
        self.state.token_flags.intersects(TokenFlags::PrecedingJSDocWithDeprecated)
    }

    pub fn has_preceding_jsdoc_with_see_or_link(&self) -> bool {
        self.state.token_flags.intersects(TokenFlags::PrecedingJSDocWithSeeOrLink)
    }

    // scanJSDocCommentForTags scans a JSDoc comment for @deprecated, @see, and @link tags,
    // setting the appropriate token flags. Called during scanning when a JSDoc comment is detected.
    fn scan_jsdoc_comment_for_tags(&mut self, comment_text: &str) {
        let mut comment_text = comment_text.as_bytes();
        loop {
            let Some(i) = memchr::memchr(b'@', comment_text) else {
                return;
            };
            comment_text = &comment_text[i + 1..];
            if !self.state.token_flags.intersects(TokenFlags::PrecedingJSDocWithDeprecated) && has_jsdoc_tag(comment_text, &["deprecated"]) {
                self.state.token_flags |= TokenFlags::PrecedingJSDocWithDeprecated;
            }
            if !self.state.token_flags.intersects(TokenFlags::PrecedingJSDocWithSeeOrLink)
                && has_jsdoc_tag(comment_text, &["see", "link", "linkcode", "linkplain"])
            {
                self.state.token_flags |= TokenFlags::PrecedingJSDocWithSeeOrLink;
            }
            if self.state.token_flags.contains(TokenFlags::PrecedingJSDocWithDeprecated | TokenFlags::PrecedingJSDocWithSeeOrLink) {
                return;
            }
        }
    }

    pub fn set_text(&mut self, text: &'static str) {
        self.text = text;
        self.end = text.len() as i32;
        self.state = ScannerState::default();
    }

    pub fn set_language_variant(&mut self, language_variant: LanguageVariant) {
        self.language_variant = language_variant;
    }

    pub fn set_script_target(&mut self, script_target: ScriptTarget) {
        self.script_target = script_target;
    }

    pub(crate) fn language_version(&self) -> ScriptTarget {
        if self.script_target == ScriptTarget::None {
            return ScriptTarget::Latest;
        }
        self.script_target
    }

    pub(crate) fn error(&mut self, diagnostic: &'static Message) {
        self.error_at(diagnostic, self.state.pos, 0, &[]);
    }

    pub(crate) fn error_at(&mut self, diagnostic: &'static Message, pos: i32, length: i32, args: &[&dyn Display]) {
        if self.on_error {
            self.errors.push(ScanError { message: diagnostic, start: pos, length, args: args.iter().map(|a| a.to_string()).collect() });
        }
    }

    // NOTE: even though this returns a rune, it only decodes the current byte.
    // It must be checked against utf8.RuneSelf to verify that a call to charAndSize
    // is not needed.
    #[inline]
    pub(crate) fn char(&self) -> i32 {
        if self.state.pos < self.end {
            return self.text.as_bytes()[self.state.pos as usize] as i32;
        }
        -1
    }

    // NOTE: this returns a rune, but only decodes the byte at the offset.
    #[inline]
    pub(crate) fn char_at(&self, offset: i32) -> i32 {
        let p = self.state.pos + offset;
        if p < self.end {
            return self.text.as_bytes()[p as usize] as i32;
        }
        -1
    }

    #[inline]
    pub(crate) fn char_and_size(&self) -> (i32, i32) {
        // Fast path: a single ASCII byte.
        if self.state.pos < self.end {
            let b = self.text.as_bytes()[self.state.pos as usize];
            if b < 0x80 {
                return (b as i32, 1);
            }
        }
        let (r, size) = decode_rune(&self.text.as_bytes()[(self.state.pos as usize).min(self.text.len())..]);
        (r, size as i32)
    }

    // scanASCIIWhile advances s.pos over the longest run of ASCII bytes for which
    // pred returns true. It stops at end-of-text, the first non-ASCII byte, or the
    // first byte where pred is false.
    #[inline]
    fn scan_ascii_while(&mut self, pred: impl Fn(u8) -> bool) {
        let text = &self.text.as_bytes()[self.state.pos as usize..self.end as usize];
        let mut i = 0;
        while i < text.len() {
            let b = text[i];
            if b >= 0x80 || !pred(b) {
                break;
            }
            i += 1;
        }
        self.state.pos += i as i32;
    }

    #[inline]
    fn slice(&self, start: i32, end: i32) -> &'static str {
        &self.text[start as usize..end as usize]
    }

    /// Go `a + b` for two token-value fragments: when `b` directly follows `a` in the source
    /// text the result is the covering text slice, otherwise it is arena-allocated.
    fn concat_value(&self, a: &'static str, b: &str) -> &'static str {
        if b.is_empty() {
            return a;
        }
        if a.is_empty() {
            return self.intern_value(b);
        }
        let base = self.text.as_ptr() as usize;
        let text_end = base + self.text.len();
        let a_start = a.as_ptr() as usize;
        let b_start = b.as_ptr() as usize;
        if a_start >= base && a_start + a.len() == b_start && b_start + b.len() <= text_end {
            let start = a_start - base;
            return &self.text[start..start + a.len() + b.len()];
        }
        let mut s = String::with_capacity(a.len() + b.len());
        s.push_str(a);
        s.push_str(b);
        alloc_str(&s)
    }

    /// Returns `s` as a `'static` value without copying if it already is a slice of the text.
    fn intern_value(&self, s: &str) -> &'static str {
        let base = self.text.as_ptr() as usize;
        let p = s.as_ptr() as usize;
        if p >= base && p + s.len() <= base + self.text.len() {
            let start = p - base;
            return &self.text[start..start + s.len()];
        }
        alloc_str(s)
    }

    pub fn scan(&mut self) -> Kind {
        self.state.full_start_pos = self.state.pos;
        self.state.token_flags = TokenFlags::None;
        'scan: loop {
            let ch = self.char();
            self.state.token_start = self.state.pos;

            match ch {
                0x09 | 0x0B | 0x0C | 0x20 => {
                    self.state.pos += 1;
                    if self.skip_trivia {
                        // The next iteration would skip any further single-line whitespace byte the same way.
                        self.scan_ascii_while(|b| b == b' ' || b == b'\t' || b == 0x0B || b == 0x0C);
                        continue 'scan;
                    }
                    loop {
                        let (ch, size) = self.char_and_size();
                        if !stringutil::is_white_space_single_line(ch) {
                            break;
                        }
                        self.state.pos += size;
                    }
                    self.state.token = Kind::WhitespaceTrivia;
                }
                0x0A | 0x0D => {
                    self.state.token_flags |= TokenFlags::PrecedingLineBreak;
                    if self.skip_trivia {
                        self.state.pos += 1;
                        self.scan_ascii_while(|b| b == b' ' || (b'\t'..=b'\r').contains(&b));
                        continue 'scan;
                    }
                    if ch == '\r' as i32 && self.char_at(1) == '\n' as i32 {
                        self.state.pos += 2;
                    } else {
                        self.state.pos += 1;
                    }
                    self.state.token = Kind::NewLineTrivia;
                }
                0x21 /* ! */ => {
                    if self.char_at(1) == '=' as i32 {
                        if self.char_at(2) == '=' as i32 {
                            self.state.pos += 3;
                            self.state.token = Kind::ExclamationEqualsEqualsToken;
                        } else {
                            self.state.pos += 2;
                            self.state.token = Kind::ExclamationEqualsToken;
                        }
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::ExclamationToken;
                    }
                }
                0x22 | 0x27 /* " ' */ => {
                    self.state.token_value = self.scan_string(false /*jsxAttributeString*/);
                    self.state.token = Kind::StringLiteral;
                }
                0x60 /* ` */ => {
                    self.state.token = self.scan_template_and_set_token_value(false /*shouldEmitInvalidEscapeError*/);
                }
                0x25 /* % */ => {
                    if self.char_at(1) == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::PercentEqualsToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::PercentToken;
                    }
                }
                0x26 /* & */ => {
                    let next = self.char_at(1);
                    if next == '&' as i32 {
                        if self.char_at(2) == '=' as i32 {
                            self.state.pos += 3;
                            self.state.token = Kind::AmpersandAmpersandEqualsToken;
                        } else {
                            self.state.pos += 2;
                            self.state.token = Kind::AmpersandAmpersandToken;
                        }
                    } else if next == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::AmpersandEqualsToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::AmpersandToken;
                    }
                }
                0x28 /* ( */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::OpenParenToken;
                }
                0x29 /* ) */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::CloseParenToken;
                }
                0x2A /* * */ => {
                    let next = self.char_at(1);
                    if next == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::AsteriskEqualsToken;
                    } else if next == '*' as i32 {
                        if self.char_at(2) == '=' as i32 {
                            self.state.pos += 3;
                            self.state.token = Kind::AsteriskAsteriskEqualsToken;
                        } else {
                            self.state.pos += 2;
                            self.state.token = Kind::AsteriskAsteriskToken;
                        }
                    } else {
                        self.state.pos += 1;
                        if self.state.skip_jsdoc_leading_asterisks != 0
                            && !self.state.token_flags.intersects(TokenFlags::PrecedingJSDocLeadingAsterisks)
                            && self.state.token_flags.intersects(TokenFlags::PrecedingLineBreak)
                        {
                            self.state.token_flags |= TokenFlags::PrecedingJSDocLeadingAsterisks;
                            continue 'scan;
                        }
                        self.state.token = Kind::AsteriskToken;
                    }
                }
                0x2B /* + */ => {
                    let next = self.char_at(1);
                    if next == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::PlusEqualsToken;
                    } else if next == '+' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::PlusPlusToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::PlusToken;
                    }
                }
                0x2C /* , */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::CommaToken;
                }
                0x2D /* - */ => {
                    let next = self.char_at(1);
                    if next == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::MinusEqualsToken;
                    } else if next == '-' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::MinusMinusToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::MinusToken;
                    }
                }
                0x2E /* . */ => {
                    let next = self.char_at(1);
                    if stringutil::is_digit(next) {
                        self.state.token = self.scan_number();
                    } else if next == '.' as i32 && self.char_at(2) == '.' as i32 {
                        self.state.pos += 3;
                        self.state.token = Kind::DotDotDotToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::DotToken;
                    }
                }
                0x2F /* / */ => {
                    // Single-line comment
                    if self.char_at(1) == '/' as i32 {
                        self.state.pos += 2;

                        loop {
                            self.scan_ascii_while(|b| b != b'\n' && b != b'\r');
                            let (ch1, size) = self.char_and_size();
                            if size == 0 || stringutil::is_line_break(ch1) {
                                break;
                            }
                            self.state.pos += size;
                        }

                        self.process_comment_directive(self.state.token_start, self.state.pos, false);

                        if self.skip_trivia {
                            continue 'scan;
                        }
                        self.state.token = Kind::SingleLineCommentTrivia;
                        return self.state.token;
                    }
                    // Multi-line comment
                    if self.char_at(1) == '*' as i32 {
                        self.state.pos += 2;
                        let is_jsdoc = self.char() == '*' as i32 && self.char_at(1) != '/' as i32;

                        let mut comment_closed = false;
                        let mut last_line_start = self.state.token_start;
                        loop {
                            self.scan_ascii_while(|b| b != b'*' && b != b'\n' && b != b'\r');
                            let (ch1, size) = self.char_and_size();
                            if size == 0 {
                                break;
                            }

                            if ch1 == '*' as i32 && self.char_at(1) == '/' as i32 {
                                self.state.pos += 2;
                                comment_closed = true;
                                break;
                            }

                            self.state.pos += size;

                            if stringutil::is_line_break(ch1) {
                                last_line_start = self.state.pos;
                                self.state.token_flags |= TokenFlags::PrecedingLineBreak;
                            }
                        }

                        if is_jsdoc {
                            self.state.token_flags |= TokenFlags::PrecedingJSDocComment;
                            self.scan_jsdoc_comment_for_tags(self.slice(self.state.token_start, self.state.pos));
                        }

                        self.process_comment_directive(last_line_start, self.state.pos, true);

                        if !comment_closed {
                            self.error(&diagnostics::Asterisk_Slash_expected);
                        }

                        if self.skip_trivia {
                            continue 'scan;
                        }

                        if !comment_closed {
                            self.state.token_flags |= TokenFlags::Unterminated;
                        }
                        self.state.token = Kind::MultiLineCommentTrivia;
                        return self.state.token;
                    }
                    if self.char_at(1) == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::SlashEqualsToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::SlashToken;
                    }
                }
                0x30..=0x39 /* 0-9 */ => 'digit: {
                    if ch == '0' as i32 {
                        if self.char_at(1) == 'X' as i32 || self.char_at(1) == 'x' as i32 {
                            let start = self.state.pos;
                            self.state.pos += 2;
                            let mut digits = self.scan_hex_digits(1, true, true);
                            if digits.is_empty() {
                                self.error(&diagnostics::Hexadecimal_digit_expected);
                                digits = "0";
                            }
                            if let Some(&cached_value) = self.hex_number_cache.get(digits) {
                                self.state.token_value = cached_value;
                            } else {
                                let raw_text = self.slice(start, self.state.pos);
                                if raw_text.starts_with("0x") && &raw_text[2..] == digits {
                                    self.state.token_value = raw_text;
                                } else {
                                    self.state.token_value = alloc_str(&format!("0x{digits}"));
                                }
                                self.pin_unless_source(digits, self.state.token_value);
                                self.hex_number_cache.insert(digits, self.state.token_value);
                            }
                            self.state.token_flags |= TokenFlags::HexSpecifier;
                            self.state.token = self.scan_big_int_suffix();
                            break 'digit;
                        }
                        if self.char_at(1) == 'B' as i32 || self.char_at(1) == 'b' as i32 {
                            self.state.pos += 2;
                            let mut digits = self.scan_binary_or_octal_digits(2);
                            if digits.is_empty() {
                                self.error(&diagnostics::Binary_digit_expected);
                                digits = "0".to_string();
                            }
                            self.state.token_value = alloc_str(&format!("0b{digits}"));
                            self.state.token_flags |= TokenFlags::BinarySpecifier;
                            self.state.token = self.scan_big_int_suffix();
                            break 'digit;
                        }
                        if self.char_at(1) == 'O' as i32 || self.char_at(1) == 'o' as i32 {
                            self.state.pos += 2;
                            let mut digits = self.scan_binary_or_octal_digits(8);
                            if digits.is_empty() {
                                self.error(&diagnostics::Octal_digit_expected);
                                digits = "0".to_string();
                            }
                            self.state.token_value = alloc_str(&format!("0o{digits}"));
                            self.state.token_flags |= TokenFlags::OctalSpecifier;
                            self.state.token = self.scan_big_int_suffix();
                            break 'digit;
                        }
                    }
                    self.state.token = self.scan_number();
                }
                0x3A /* : */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::ColonToken;
                }
                0x3B /* ; */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::SemicolonToken;
                }
                0x3C /* < */ => {
                    if self.char_at(1) == '<' as i32 && is_conflict_marker_trivia(self.text, self.state.pos) {
                        self.state.pos = self.scan_conflict_marker_trivia_reporting(self.state.pos);
                        if self.skip_trivia {
                            continue 'scan;
                        } else {
                            self.state.token = Kind::ConflictMarkerTrivia;
                            return self.state.token;
                        }
                    }
                    if self.char_at(1) == '<' as i32 {
                        if self.char_at(2) == '=' as i32 {
                            self.state.pos += 3;
                            self.state.token = Kind::LessThanLessThanEqualsToken;
                        } else {
                            self.state.pos += 2;
                            self.state.token = Kind::LessThanLessThanToken;
                        }
                    } else if self.char_at(1) == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::LessThanEqualsToken;
                    } else if self.language_variant == LanguageVariant::JSX && self.char_at(1) == '/' as i32 && self.char_at(2) != '*' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::LessThanSlashToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::LessThanToken;
                    }
                }
                0x3D /* = */ => {
                    if self.char_at(1) == '=' as i32 && is_conflict_marker_trivia(self.text, self.state.pos) {
                        self.state.pos = self.scan_conflict_marker_trivia_reporting(self.state.pos);
                        if self.skip_trivia {
                            continue 'scan;
                        } else {
                            self.state.token = Kind::ConflictMarkerTrivia;
                            return self.state.token;
                        }
                    }
                    if self.char_at(1) == '=' as i32 {
                        if self.char_at(2) == '=' as i32 {
                            self.state.pos += 3;
                            self.state.token = Kind::EqualsEqualsEqualsToken;
                        } else {
                            self.state.pos += 2;
                            self.state.token = Kind::EqualsEqualsToken;
                        }
                    } else if self.char_at(1) == '>' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::EqualsGreaterThanToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::EqualsToken;
                    }
                }
                0x3E /* > */ => {
                    if self.char_at(1) == '>' as i32 && is_conflict_marker_trivia(self.text, self.state.pos) {
                        self.state.pos = self.scan_conflict_marker_trivia_reporting(self.state.pos);
                        if self.skip_trivia {
                            continue 'scan;
                        } else {
                            self.state.token = Kind::ConflictMarkerTrivia;
                            return self.state.token;
                        }
                    }
                    self.state.pos += 1;
                    self.state.token = Kind::GreaterThanToken;
                }
                0x3F /* ? */ => {
                    if self.char_at(1) == '.' as i32 && !stringutil::is_digit(self.char_at(2)) {
                        self.state.pos += 2;
                        self.state.token = Kind::QuestionDotToken;
                    } else if self.char_at(1) == '?' as i32 {
                        if self.char_at(2) == '=' as i32 {
                            self.state.pos += 3;
                            self.state.token = Kind::QuestionQuestionEqualsToken;
                        } else {
                            self.state.pos += 2;
                            self.state.token = Kind::QuestionQuestionToken;
                        }
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::QuestionToken;
                    }
                }
                0x5B /* [ */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::OpenBracketToken;
                }
                0x5D /* ] */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::CloseBracketToken;
                }
                0x5E /* ^ */ => {
                    if self.char_at(1) == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::CaretEqualsToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::CaretToken;
                    }
                }
                0x7B /* { */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::OpenBraceToken;
                }
                0x7C /* | */ => {
                    if self.char_at(1) == '|' as i32 && is_conflict_marker_trivia(self.text, self.state.pos) {
                        self.state.pos = self.scan_conflict_marker_trivia_reporting(self.state.pos);
                        if self.skip_trivia {
                            continue 'scan;
                        } else {
                            self.state.token = Kind::ConflictMarkerTrivia;
                            return self.state.token;
                        }
                    }
                    if self.char_at(1) == '|' as i32 {
                        if self.char_at(2) == '=' as i32 {
                            self.state.pos += 3;
                            self.state.token = Kind::BarBarEqualsToken;
                        } else {
                            self.state.pos += 2;
                            self.state.token = Kind::BarBarToken;
                        }
                    } else if self.char_at(1) == '=' as i32 {
                        self.state.pos += 2;
                        self.state.token = Kind::BarEqualsToken;
                    } else {
                        self.state.pos += 1;
                        self.state.token = Kind::BarToken;
                    }
                }
                0x7D /* } */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::CloseBraceToken;
                }
                0x7E /* ~ */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::TildeToken;
                }
                0x40 /* @ */ => {
                    self.state.pos += 1;
                    self.state.token = Kind::AtToken;
                }
                0x5C /* \ */ => {
                    if self.scan_identifier(0, IdentifierVariant::Standard) {
                        self.state.token = get_identifier_token(self.state.token_value);
                    } else {
                        self.scan_invalid_character();
                    }
                }
                0x23 /* # */ => 'hash: {
                    if self.char_at(1) == '!' as i32 {
                        if self.state.pos == 0 {
                            self.state.pos += 2;
                            let (mut ch, mut size) = self.char_and_size();
                            while size > 0 && !stringutil::is_line_break(ch) {
                                self.state.pos += size;
                                (ch, size) = self.char_and_size();
                            }
                            continue 'scan;
                        }
                        self.error_at(&diagnostics::X_can_only_be_used_at_the_start_of_a_file, self.state.pos, 2, &[]);
                        self.state.pos += 2;
                        self.state.token = Kind::Unknown;
                        break 'hash;
                    }
                    if !self.scan_identifier(1, IdentifierVariant::Standard) {
                        self.error_at(&diagnostics::Invalid_character, self.state.pos - 1, 1, &[]);
                        self.state.token_value = "#";
                    }
                    self.state.token = Kind::PrivateIdentifier;
                }
                _ => 'default: {
                    if ch < 0 {
                        self.state.token = Kind::EndOfFile;
                        break 'default;
                    }
                    if self.scan_identifier(0, IdentifierVariant::Standard) {
                        self.state.token = get_identifier_token(self.state.token_value);
                        break 'default;
                    }
                    let (mut ch, mut size) = self.char_and_size();
                    if ch == RUNE_ERROR {
                        self.error_at(&diagnostics::File_appears_to_be_binary, 0, 0, &[]);
                        self.state.pos = self.text.len() as i32;
                        self.state.token = Kind::NonTextFileMarkerTrivia;
                        break 'default;
                    }
                    if stringutil::is_white_space_single_line(ch) {
                        self.state.pos += size;

                        // If we get here and it's not 0x0085 (nextLine), then we're handling non-ASCII whitespace.
                        // Handle skipTrivia like we do in the space case above.
                        if ch == 0x0085 || self.skip_trivia {
                            continue 'scan;
                        }

                        loop {
                            (ch, size) = self.char_and_size();
                            if !stringutil::is_white_space_single_line(ch) {
                                break;
                            }
                            self.state.pos += size;
                        }
                        self.state.token = Kind::WhitespaceTrivia;
                        return self.state.token;
                    }
                    if stringutil::is_line_break(ch) {
                        self.state.token_flags |= TokenFlags::PrecedingLineBreak;
                        self.state.pos += size;
                        continue 'scan;
                    }
                    self.scan_invalid_character();
                }
            }
            return self.state.token;
        }
    }

    fn scan_conflict_marker_trivia_reporting(&mut self, pos: i32) -> i32 {
        let text = self.text;
        scan_conflict_marker_trivia(text, pos, Some(&mut |diag, pos, length| self.error_at(diag, pos, length, &[])))
    }

    fn process_comment_directive(&mut self, start: i32, end: i32, multiline: bool) {
        let text = self.text.as_bytes();
        let end_u = end as usize;
        // Skip starting slashes and whitespace
        let mut pos = start as usize;
        if multiline {
            // Skip whitespace
            while pos < end_u && (text[pos] == b' ' || text[pos] == b'\t') {
                pos += 1;
            }
            // Skip combinations of / and *
            while pos < end_u && (text[pos] == b'/' || text[pos] == b'*') {
                pos += 1;
            }
        } else {
            // Skip opening //
            pos += 2;
            // Skip another / if present
            while pos < end_u && text[pos] == b'/' {
                pos += 1;
            }
        }
        // Skip whitespace
        while pos < end_u && (text[pos] == b' ' || text[pos] == b'\t') {
            pos += 1;
        }
        // Directive must start with '@'
        if !(pos < end_u && text[pos] == b'@') {
            return;
        }
        pos += 1;
        let rest = &text[pos..];
        let kind = if rest.starts_with(b"ts-expect-error") {
            CommentDirectiveKind::ExpectError
        } else if rest.starts_with(b"ts-ignore") {
            CommentDirectiveKind::Ignore
        } else {
            return;
        };
        // Directives are rare; copying keeps every saved state's snapshot intact (Go shares the backing array,
        // but never rewrites elements below a saved length).
        let mut directives = self.state.comment_directives.to_vec();
        directives.push(CommentDirective { loc: TextRange::new(start, end), kind });
        self.state.comment_directives = tsrs_core::alloc_vec(directives);
    }

    pub fn re_scan_less_than_token(&mut self) -> Kind {
        if self.state.token == Kind::LessThanLessThanToken {
            self.state.pos = self.state.token_start + 1;
            self.state.token = Kind::LessThanToken;
        }
        self.state.token
    }

    pub fn re_scan_greater_than_token(&mut self) -> Kind {
        if self.state.token == Kind::GreaterThanToken {
            self.re_scan_greater_than_token_inner();
        }
        self.state.token
    }

    fn re_scan_greater_than_token_inner(&mut self) {
        self.state.pos = self.state.token_start + 1;
        if self.char() == '>' as i32 {
            if self.char_at(1) == '>' as i32 {
                if self.char_at(2) == '=' as i32 {
                    self.state.pos += 3;
                    self.state.token = Kind::GreaterThanGreaterThanGreaterThanEqualsToken;
                } else {
                    self.state.pos += 2;
                    self.state.token = Kind::GreaterThanGreaterThanGreaterThanToken;
                }
            } else if self.char_at(1) == '=' as i32 {
                self.state.pos += 2;
                self.state.token = Kind::GreaterThanGreaterThanEqualsToken;
            } else {
                self.state.pos += 1;
                self.state.token = Kind::GreaterThanGreaterThanToken;
            }
        } else if self.char() == '=' as i32 {
            self.state.pos += 1;
            self.state.token = Kind::GreaterThanEqualsToken;
        }
    }

    pub fn re_scan_template_token(&mut self, is_tagged_template: bool) -> Kind {
        self.state.pos = self.state.token_start;
        self.state.token = self.scan_template_and_set_token_value(!is_tagged_template);
        self.state.token
    }

    pub fn re_scan_asterisk_equals_token(&mut self) -> Kind {
        if self.state.token != Kind::AsteriskEqualsToken {
            panic!("'ReScanAsteriskEqualsToken' should only be called on a '*='");
        }
        self.state.pos = self.state.token_start + 1;
        self.state.token = Kind::EqualsToken;
        self.state.token
    }

    /// Go `ReScanSlashToken(reportErrors ...bool)`; pass `false` for the no-argument form.
    pub fn re_scan_slash_token(&mut self, report_errors: bool) -> Kind {
        let should_report_errors = report_errors;
        if self.state.token == Kind::SlashToken || self.state.token == Kind::SlashEqualsToken {
            let text = self.text.as_bytes();
            let end = self.end;
            // Quickly get to the end of regex such that we know the flags
            let start_of_reg_exp_body = self.state.token_start + 1;
            let mut p = start_of_reg_exp_body;
            let mut in_escape = false;
            let mut named_capture_groups = false;
            // Although nested character classes are allowed in Unicode Sets mode,
            // an unescaped slash is nevertheless invalid even in a character class in any Unicode mode.
            // This is indicated by Section 12.9.5 Regular Expression Literals of the specification,
            // where nested character classes are not considered at all. (A `[` RegularExpressionClassChar
            // does nothing in a RegularExpressionClass, and a `]` always closes the class.)
            // Additionally, parsing nested character classes will misinterpret regexes like `/[[]/`
            // as unterminated, consuming characters beyond the slash. (This even applies to `/[[]/v`,
            // which should be parsed as a well-terminated regex with an incomplete character class.)
            // Thus we must not handle nested character classes in the first pass.
            let mut in_character_class = false;
            loop {
                // If we reach the end of a file, or hit a newline, then this is an unterminated
                // regex. Report error and return what we have so far.
                if p >= end {
                    self.state.token_flags |= TokenFlags::Unterminated;
                    break;
                }
                let ch = text[p as usize] as i32;
                if stringutil::is_line_break(ch) {
                    self.state.token_flags |= TokenFlags::Unterminated;
                    break;
                } else if in_escape {
                    // Parsing an escape character;
                    // reset the flag and just advance to the next char.
                    in_escape = false;
                } else if ch == '/' as i32 && !in_character_class {
                    // A slash within a character class is permissible,
                    // but in general it signals the end of the regexp literal.
                    break;
                } else if ch == '[' as i32 {
                    in_character_class = true;
                } else if ch == '\\' as i32 {
                    in_escape = true;
                } else if ch == ']' as i32 {
                    in_character_class = false;
                } else if !in_character_class
                    && ch == '(' as i32
                    && p + 1 < end
                    && text[(p + 1) as usize] == b'?'
                    && p + 2 < end
                    && text[(p + 2) as usize] == b'<'
                    && (p + 3 >= end || (text[(p + 3) as usize] != b'=' && text[(p + 3) as usize] != b'!'))
                {
                    named_capture_groups = true;
                }
                p += 1;
            }

            let end_of_reg_exp_body = p;
            if self.state.token_flags.intersects(TokenFlags::Unterminated) {
                // Search for the nearest unbalanced bracket for better recovery. Since the expression is
                // invalid anyways, we take nested square brackets into consideration for the best guess.
                p = start_of_reg_exp_body;
                in_escape = false;
                let mut character_class_depth = 0;
                let mut in_decimal_quantifier = false;
                let mut group_depth = 0;
                while p < end_of_reg_exp_body {
                    let ch = text[p as usize];
                    if in_escape {
                        in_escape = false;
                    } else if ch == b'\\' {
                        in_escape = true;
                    } else if ch == b'[' {
                        character_class_depth += 1;
                    } else if ch == b']' && character_class_depth != 0 {
                        character_class_depth -= 1;
                    } else if character_class_depth == 0 {
                        if ch == b'{' {
                            in_decimal_quantifier = true;
                        } else if ch == b'}' && in_decimal_quantifier {
                            in_decimal_quantifier = false;
                        } else if !in_decimal_quantifier {
                            if ch == b'(' {
                                group_depth += 1;
                            } else if ch == b')' && group_depth != 0 {
                                group_depth -= 1;
                            } else if ch == b')' || ch == b']' || ch == b'}' {
                                // We encountered an unbalanced bracket outside a character class. Treat this position as the end of regex.
                                break;
                            }
                        }
                    }
                    p += 1;
                }
                // Whitespaces and semicolons at the end are not likely to be part of the regex
                while p > start_of_reg_exp_body {
                    let (ch, size) = decode_last_rune(&text[..p as usize]);
                    if stringutil::is_white_space_like(ch) || ch == ';' as i32 {
                        p -= size as i32;
                    } else {
                        break;
                    }
                }
                self.error_at(&diagnostics::Unterminated_regular_expression_literal, self.state.token_start, p - self.state.token_start, &[]);
            } else {
                // Consume the slash character
                p += 1;
                let mut reg_exp_flags = RegularExpressionFlags::None;
                while p < end {
                    let (ch, size) = decode_rune(&text[p as usize..]);
                    let size = size as i32;
                    if ch == RUNE_ERROR || !is_identifier_part(ch) {
                        break;
                    }
                    if should_report_errors {
                        match char_code_to_reg_exp_flag(ch) {
                            None => self.error_at(&diagnostics::Unknown_regular_expression_flag, p, size, &[]),
                            Some(flag) if reg_exp_flags.intersects(flag) => {
                                self.error_at(&diagnostics::Duplicate_regular_expression_flag, p, size, &[]);
                            }
                            Some(flag) if (reg_exp_flags | flag).contains(RegularExpressionFlags::AnyUnicodeMode) => {
                                self.error_at(
                                    &diagnostics::The_Unicode_u_flag_and_the_Unicode_Sets_v_flag_cannot_be_set_simultaneously,
                                    p,
                                    size,
                                    &[],
                                );
                            }
                            Some(flag) => {
                                reg_exp_flags |= flag;
                                self.check_regular_expression_flag_availability(flag, p, size);
                            }
                        }
                    }
                    p += size;
                }
                if should_report_errors {
                    self.state.pos = start_of_reg_exp_body;
                    let save_end = self.end;
                    let save_token_pos = self.state.token_start;
                    let save_token_flags = self.state.token_flags;
                    self.end = end_of_reg_exp_body;
                    let scanner = std::mem::take(self);
                    let mut parser = RegExpParser::new(
                        scanner,
                        end_of_reg_exp_body,
                        reg_exp_flags.intersects(RegularExpressionFlags::AnyUnicodeMode),
                        reg_exp_flags.intersects(RegularExpressionFlags::UnicodeSets),
                        true,
                        named_capture_groups,
                    );
                    parser.run();
                    *self = parser.into_scanner();
                    self.end = save_end;
                    self.state.pos = p;
                    self.state.token_start = save_token_pos;
                    self.state.token_flags = save_token_flags;
                } else {
                    self.state.pos = p;
                }
            }

            self.state.pos = p;
            self.state.token_value = self.slice(self.state.token_start, self.state.pos);
            self.state.token = Kind::RegularExpressionLiteral;
        }
        self.state.token
    }

    pub fn re_scan_jsx_token(&mut self, allow_multiline_jsx_text: bool) -> Kind {
        self.state.pos = self.state.full_start_pos;
        self.state.token_start = self.state.full_start_pos;
        self.state.token = self.scan_jsx_token_ex(allow_multiline_jsx_text);
        self.state.token
    }

    pub fn re_scan_hash_token(&mut self) -> Kind {
        if self.state.token == Kind::PrivateIdentifier {
            self.state.pos = self.state.token_start + 1;
            self.state.token = Kind::HashToken;
        }
        self.state.token
    }

    pub fn re_scan_question_token(&mut self) -> Kind {
        if self.state.token != Kind::QuestionQuestionToken {
            panic!("'reScanQuestionToken' should only be called on a '??'");
        }
        self.state.pos = self.state.token_start + 1;
        self.state.token = Kind::QuestionToken;
        self.state.token
    }

    pub fn scan_jsx_token(&mut self) -> Kind {
        self.scan_jsx_token_ex(true /*allowMultilineJsxText*/)
    }

    pub fn scan_jsx_token_ex(&mut self, allow_multiline_jsx_text: bool) -> Kind {
        self.state.full_start_pos = self.state.pos;
        self.state.token_start = self.state.pos;
        let ch = self.char();
        if ch < 0 {
            self.state.token = Kind::EndOfFile;
        } else if ch == '<' as i32 {
            if self.char_at(1) == '/' as i32 {
                self.state.pos += 2;
                self.state.token = Kind::LessThanSlashToken;
            } else {
                self.state.pos += 1;
                self.state.token = Kind::LessThanToken;
            }
        } else if ch == '{' as i32 {
            self.state.pos += 1;
            self.state.token = Kind::OpenBraceToken;
        } else {
            // First non-whitespace character on this line.
            let mut first_non_whitespace = 0;
            // These initial values are special because the first line is:
            // firstNonWhitespace = 0 to indicate that we want leading whitespace
            loop {
                let (ch, size) = self.char_and_size();
                if size == 0 || ch == '{' as i32 {
                    break;
                }
                if ch == '<' as i32 {
                    if is_conflict_marker_trivia(self.text, self.state.pos) {
                        self.state.pos = self.scan_conflict_marker_trivia_reporting(self.state.pos);
                        self.state.token = Kind::ConflictMarkerTrivia;
                        return self.state.token;
                    }
                    break;
                }
                if ch == '>' as i32 {
                    self.error_at(&diagnostics::Unexpected_token_Did_you_mean_or_gt, self.state.pos, 1, &[]);
                } else if ch == '}' as i32 {
                    self.error_at(&diagnostics::Unexpected_token_Did_you_mean_or_rbrace, self.state.pos, 1, &[]);
                }
                // FirstNonWhitespace is 0, then we only see whitespaces so far. If we see a linebreak, we want to ignore that whitespaces.
                // i.e (- : whitespace)
                //      <div>----
                //      </div> becomes <div></div>
                //
                //      <div>----</div> becomes <div>----</div>
                if stringutil::is_line_break(ch) && first_non_whitespace == 0 {
                    first_non_whitespace = -1;
                } else if !allow_multiline_jsx_text && stringutil::is_line_break(ch) && first_non_whitespace > 0 {
                    // Stop JsxText on each line during formatting. This allows the formatter to
                    // indent each line correctly.
                    break;
                } else if !stringutil::is_white_space_like(ch) {
                    first_non_whitespace = self.state.pos;
                }
                self.state.pos += size;
            }
            self.state.token_value = self.slice(self.state.full_start_pos, self.state.pos);
            self.state.token = Kind::JsxText;
            if first_non_whitespace == -1 {
                self.state.token = Kind::JsxTextAllWhiteSpaces;
            }
        }
        self.state.token
    }

    // Scans a JSX identifier; these differ from normal identifiers in that they allow dashes
    pub fn scan_jsx_identifier(&mut self) -> Kind {
        if token_is_identifier_or_keyword(self.state.token) {
            // An identifier or keyword has already been parsed - check for a `-` or a single instance of `:` and then append it and
            // everything after it to the token
            // Do note that this means that `scanJsxIdentifier` effectively _mutates_ the visible token without advancing to a new token
            // Any caller should be expecting this behavior and should only read the pos or token value after calling it.
            // Here scanIdentifierParts is reused to ensure Unicode escapes are handled.
            let parts = self.scan_identifier_parts(IdentifierVariant::JSX);
            self.state.token_value = self.concat_value(self.state.token_value, parts);
            self.state.token = get_identifier_token(self.state.token_value);
        }
        self.state.token
    }

    pub fn scan_jsx_attribute_value(&mut self) -> Kind {
        self.state.full_start_pos = self.state.pos;
        // Skip whitespace between '=' and the value so tokenStart lands on the
        // opening quote, not on trivia.
        let (mut ch, mut size) = self.char_and_size();
        while size > 0 && stringutil::is_white_space_like(ch) {
            self.state.pos += size;
            (ch, size) = self.char_and_size();
        }
        self.state.token_start = self.state.pos;
        match self.char() {
            0x22 | 0x27 => {
                self.state.token_value = self.scan_string(true /*jsxAttributeString*/);
                self.state.token = Kind::StringLiteral;
                self.state.token
            }
            // If this scans anything other than `{`, it's a parse error.
            _ => self.scan(),
        }
    }

    pub fn re_scan_jsx_attribute_value(&mut self) -> Kind {
        self.state.pos = self.state.full_start_pos;
        self.state.token_start = self.state.full_start_pos;
        self.scan_jsx_attribute_value()
    }

    /** In addition to the usual JSDoc ast.Kinds, can also return ast.KindJSDocCommentTextToken */
    pub fn scan_jsdoc_comment_text_token(&mut self, in_backticks: bool) -> Kind {
        self.state.full_start_pos = self.state.pos;
        self.state.token_flags = TokenFlags::None;
        if self.state.pos as usize >= self.text.len() {
            self.state.token = Kind::EndOfFile;
            return self.state.token;
        }
        self.state.token_start = self.state.pos;
        let text = self.text.as_bytes();
        let (mut ch, mut size) = self.char_and_size();
        while (self.state.pos as usize) < self.text.len() && !stringutil::is_line_break(ch) && ch != '`' as i32 {
            if !in_backticks {
                if ch == '{' as i32 {
                    break;
                } else if ch == '@' as i32 && self.state.pos >= 0 {
                    // @ doesn't start a new tag inside ``, and elsewhere, only after whitespace and before identifier
                    let (previous, _) = decode_last_rune(&text[..self.state.pos as usize]);
                    if stringutil::is_white_space_single_line(previous) {
                        let (next, _) = decode_rune(&text[(self.state.pos + size) as usize..]);
                        if is_identifier_start(next) {
                            break;
                        }
                    }
                }
            }
            self.state.pos += size;
            (ch, size) = self.char_and_size();
        }
        if self.state.pos == self.state.token_start {
            return self.scan_jsdoc_token();
        }
        self.state.token_value = self.slice(self.state.token_start, self.state.pos);
        self.state.token = Kind::JSDocCommentTextToken;
        self.state.token
    }

    // Peek at the character at the current scanner position (expected to be right after '@')
    // and return true if a JSDoc tag can follow. Identifier starts indicate a tag name.
    // Whitespace, newlines, and EOF are also accepted to support incomplete tags for code completion.
    pub fn can_follow_jsdoc_at(&self) -> bool {
        if self.state.pos as usize >= self.text.len() {
            return true;
        }
        let (ch, _) = decode_rune(&self.text.as_bytes()[self.state.pos as usize..]);
        is_identifier_start(ch) || stringutil::is_white_space_single_line(ch) || stringutil::is_line_break(ch)
    }

    pub fn scan_jsdoc_token(&mut self) -> Kind {
        self.state.full_start_pos = self.state.pos;
        self.state.token_flags = TokenFlags::None;
        if self.state.pos as usize >= self.text.len() {
            self.state.token = Kind::EndOfFile;
            return self.state.token;
        }

        self.state.token_start = self.state.pos;
        let (ch, size) = self.char_and_size();
        self.state.pos += size;
        let token = match ch {
            0x09 | 0x0B | 0x0C | 0x20 => {
                let (mut ch2, mut size2) = self.char_and_size();
                while size2 > 0 && stringutil::is_white_space_single_line(ch2) {
                    self.state.pos += size2;
                    (ch2, size2) = self.char_and_size();
                }
                Some(Kind::WhitespaceTrivia)
            }
            0x40 /* @ */ => Some(Kind::AtToken),
            0x0D | 0x0A => {
                if ch == '\r' as i32 && self.char() == '\n' as i32 {
                    self.state.pos += 1;
                }
                self.state.token_flags |= TokenFlags::PrecedingLineBreak;
                Some(Kind::NewLineTrivia)
            }
            0x2A /* * */ => Some(Kind::AsteriskToken),
            0x7B /* { */ => Some(Kind::OpenBraceToken),
            0x7D /* } */ => Some(Kind::CloseBraceToken),
            0x5B /* [ */ => Some(Kind::OpenBracketToken),
            0x5D /* ] */ => Some(Kind::CloseBracketToken),
            0x28 /* ( */ => Some(Kind::OpenParenToken),
            0x29 /* ) */ => Some(Kind::CloseParenToken),
            0x3C /* < */ => Some(Kind::LessThanToken),
            0x3E /* > */ => Some(Kind::GreaterThanToken),
            0x3D /* = */ => Some(Kind::EqualsToken),
            0x2C /* , */ => Some(Kind::CommaToken),
            0x2E /* . */ => Some(Kind::DotToken),
            0x60 /* ` */ => Some(Kind::BacktickToken),
            0x23 /* # */ => Some(Kind::HashToken),
            _ => None,
        };
        if let Some(token) = token {
            self.state.token = token;
            return self.state.token;
        }

        self.state.pos = self.state.token_start;
        if self.scan_identifier(0, IdentifierVariant::JSX) {
            self.state.token = get_identifier_token(self.state.token_value);
            return self.state.token;
        }
        self.state.pos = self.state.token_start + size;
        self.state.token = Kind::Unknown;
        self.state.token
    }

    pub(crate) fn scan_identifier(&mut self, prefix_length: i32, variant: IdentifierVariant) -> bool {
        let start = self.state.pos;
        self.state.pos += prefix_length;
        let identifier_start = self.state.pos;
        let ch = self.char();
        // Fast path for simple ASCII identifiers
        if variant != IdentifierVariant::JSX && (stringutil::is_ascii_letter(ch) || ch == '_' as i32 || ch == '$' as i32) {
            self.state.pos += 1;
            self.scan_ascii_while(|b| ASCII_IDENTIFIER_PART[b as usize]);
            let ch = self.char();
            if ch < RUNE_SELF && ch != '\\' as i32 {
                self.state.token_value = self.slice(start, self.state.pos);
                return true;
            }
            self.state.pos = identifier_start;
        }
        let (mut ch, mut size) = self.char_and_size();
        if is_identifier_start(ch) {
            let language_variant = if variant == IdentifierVariant::JSX { LanguageVariant::JSX } else { LanguageVariant::Standard };
            loop {
                self.state.pos += size;
                (ch, size) = self.char_and_size();
                if !is_identifier_part_ex(ch, language_variant) {
                    break;
                }
            }
            self.state.token_value = self.slice(start, self.state.pos);
            if ch == '\\' as i32 {
                let parts = self.scan_identifier_parts(variant);
                self.state.token_value = self.concat_value(self.state.token_value, parts);
            }
            return true;
        }
        if ch == '\\' as i32 {
            if let Some(escaped) =
                self.scan_identifier_escape(|ch| is_identifier_start(ch), variant == IdentifierVariant::RegExpGroupName)
            {
                let mut value = String::new();
                value.push_str(self.slice(start, identifier_start));
                value.push(rune_to_char(escaped));
                value.push_str(self.scan_identifier_parts(variant));
                self.state.token_value = alloc_str(&value);
                return true;
            }
        }
        false
    }

    fn scan_identifier_parts(&mut self, variant: IdentifierVariant) -> &'static str {
        let mut sb: Option<String> = None;
        let mut start = self.state.pos;
        let language_variant = if variant == IdentifierVariant::JSX { LanguageVariant::JSX } else { LanguageVariant::Standard };
        loop {
            let (ch, size) = self.char_and_size();
            if is_identifier_part_ex(ch, language_variant) {
                self.state.pos += size;
                continue;
            }
            if ch == '\\' as i32 {
                let escape_start = self.state.pos;
                if let Some(escaped) = self.scan_identifier_escape(
                    |ch| is_identifier_part_ex(ch, language_variant),
                    variant == IdentifierVariant::RegExpGroupName,
                ) {
                    let sb = sb.get_or_insert_with(String::new);
                    sb.push_str(self.slice(start, escape_start));
                    sb.push(rune_to_char(escaped));
                    start = self.state.pos;
                    continue;
                }
            }
            break;
        }
        match sb {
            None => self.slice(start, self.state.pos),
            Some(mut sb) => {
                sb.push_str(self.slice(start, self.state.pos));
                alloc_str(&sb)
            }
        }
    }

    fn scan_identifier_escape(&mut self, is_valid: impl Fn(i32) -> bool, allow_surrogate_pair_escape: bool) -> Option<i32> {
        let escaped = self.peek_unicode_escape();
        if escaped >= 0 && is_valid(escaped) {
            return Some(self.scan_unicode_escape(true));
        }
        if allow_surrogate_pair_escape && self.char_at(2) != '{' as i32 && stringutil::is_high_surrogate(escaped) {
            // Unlike normal identifiers, group names in regular expressions, whether in Unicode mode or not,
            // accept \u HexLeadSurrogate \u HexTrailSurrogate as part of RegExpIdentifierName.
            // See https://github.com/tc39/ecma262/pull/1869 for the change.
            let saved_pos = self.state.pos;
            let saved_token_flags = self.state.token_flags;
            self.scan_unicode_escape(false);
            // scanLowSurrogateEscape also accepts the braced form used in string literals,
            // but RegExpIdentifierName does not allow it.
            if self.char_at(2) != '{' as i32 {
                if let Some(code_point) = self.scan_low_surrogate_escape(escaped) {
                    if is_valid(code_point) {
                        return Some(code_point);
                    }
                }
            }
            self.state.pos = saved_pos;
            self.state.token_flags = saved_token_flags;
        }
        None
    }

    fn scan_string(&mut self, jsx_attribute_string: bool) -> &'static str {
        let quote = self.char();
        if quote == '\'' as i32 {
            self.state.token_flags |= TokenFlags::SingleQuote;
        }
        self.state.pos += 1;
        // Fast path for simple strings without escape sequences.
        let rest = &self.text.as_bytes()[self.state.pos as usize..];
        if let Some(str_len) = memchr::memchr(quote as u8, rest) {
            if str_len == 0 {
                self.state.pos += 1;
                return "";
            }
            let str_bytes = &rest[..str_len];
            if jsx_attribute_string || memchr::memchr3(b'\\', b'\r', b'\n', str_bytes).is_none() {
                let s = self.slice(self.state.pos, self.state.pos + str_len as i32);
                self.state.pos += str_len as i32 + 1;
                return s;
            }
        }
        let mut sb = String::new();
        let mut start = self.state.pos;
        loop {
            let ch = self.char();
            if ch < 0 {
                sb.push_str(self.slice(start, self.state.pos));
                self.state.token_flags |= TokenFlags::Unterminated;
                self.error(&diagnostics::Unterminated_string_literal);
                break;
            }
            if ch == quote {
                sb.push_str(self.slice(start, self.state.pos));
                self.state.pos += 1;
                break;
            }
            if ch == '\\' as i32 && !jsx_attribute_string {
                sb.push_str(self.slice(start, self.state.pos));
                let escaped = self.scan_escape_sequence(EscapeSequenceScanningFlags::String | EscapeSequenceScanningFlags::ReportErrors);
                sb.push_str(&escaped);
                start = self.state.pos;
                continue;
            }
            if (ch == '\n' as i32 || ch == '\r' as i32) && !jsx_attribute_string {
                sb.push_str(self.slice(start, self.state.pos));
                self.state.token_flags |= TokenFlags::Unterminated;
                self.error(&diagnostics::Unterminated_string_literal);
                break;
            }
            self.state.pos += 1;
        }
        self.intern_value(&sb)
    }

    fn scan_template_and_set_token_value(&mut self, should_emit_invalid_escape_error: bool) -> Kind {
        let started_with_backtick = self.char() == '`' as i32;
        self.state.pos += 1;
        let mut start = self.state.pos;
        // Go collects the parts in a slice and joins them; a single untouched part is a text slice.
        let mut parts: Option<String> = None;
        let token;
        loop {
            self.scan_ascii_while(|b| b != b'`' && b != b'$' && b != b'\\' && b != b'\r');
            let ch = self.char();
            if ch < 0 || ch == '`' as i32 {
                let part_end = self.state.pos;
                if ch == '`' as i32 {
                    self.state.pos += 1;
                } else {
                    self.state.token_flags |= TokenFlags::Unterminated;
                    self.error(&diagnostics::Unterminated_template_literal);
                }
                self.finish_template_parts(&mut parts, start, part_end);
                token = if started_with_backtick { Kind::NoSubstitutionTemplateLiteral } else { Kind::TemplateTail };
                break;
            }
            if ch == '$' as i32 && self.char_at(1) == '{' as i32 {
                let part_end = self.state.pos;
                self.state.pos += 2;
                self.finish_template_parts(&mut parts, start, part_end);
                token = if started_with_backtick { Kind::TemplateHead } else { Kind::TemplateMiddle };
                break;
            }
            if ch == '\\' as i32 {
                let buf = parts.get_or_insert_with(String::new);
                buf.push_str(self.slice(start, self.state.pos));
                let flags = EscapeSequenceScanningFlags::String
                    | if should_emit_invalid_escape_error { EscapeSequenceScanningFlags::ReportErrors } else { EscapeSequenceScanningFlags::empty() };
                let escaped = self.scan_escape_sequence(flags);
                parts.as_mut().unwrap().push_str(&escaped);
                start = self.state.pos;
                continue;
            }
            // Speculated ECMAScript 6 Spec 11.8.6.1:
            // <CR><LF> and <CR> LineTerminatorSequences are normalized to <LF> for Template Values
            if ch == '\r' as i32 {
                let buf = parts.get_or_insert_with(String::new);
                buf.push_str(self.slice(start, self.state.pos));
                self.state.pos += 1;
                if self.char() == '\n' as i32 {
                    self.state.pos += 1;
                }
                parts.as_mut().unwrap().push('\n');
                start = self.state.pos;
                continue;
            }
            self.state.pos += 1;
        }
        token
    }

    fn finish_template_parts(&mut self, parts: &mut Option<String>, start: i32, end: i32) {
        self.state.token_value = match parts.take() {
            None => self.slice(start, end),
            Some(mut buf) => {
                buf.push_str(self.slice(start, end));
                alloc_str(&buf)
            }
        };
    }

    pub(crate) fn scan_escape_sequence(&mut self, flags: EscapeSequenceScanningFlags) -> Cow<'static, str> {
        let start = self.state.pos;
        self.state.pos += 1;
        let ch = self.char();
        if ch < 0 {
            self.error(&diagnostics::Unexpected_end_of_text);
            return Cow::Borrowed("");
        }
        self.state.pos += 1;
        match ch {
            0x30..=0x37 /* 0-7 */ => {
                if ch == '0' as i32 {
                    // Although '0' preceding any digit is treated as LegacyOctalEscapeSequence,
                    // '\08' should separately be interpreted as '\0' + '8'.
                    if !stringutil::is_digit(self.char()) {
                        return Cow::Borrowed("\x00");
                    }
                }
                if ch <= '3' as i32 {
                    // '\01', '\011'
                    // '\1', '\17', '\177'
                    if stringutil::is_octal_digit(self.char()) {
                        self.state.pos += 1;
                    }
                }
                // '\17', '\177'
                // '\4', '\47' but not '\477'
                if stringutil::is_octal_digit(self.char()) {
                    self.state.pos += 1;
                }
                // '\47'
                self.state.token_flags |= TokenFlags::ContainsInvalidEscape;
                if flags.intersects(EscapeSequenceScanningFlags::ReportInvalidEscapeErrors) {
                    let code = i64::from_str_radix(self.slice(start + 1, self.state.pos), 8).unwrap_or(0) as i32;
                    let arg = format!("\\x{:02x}", code);
                    if flags.intersects(EscapeSequenceScanningFlags::RegularExpression)
                        && !flags.intersects(EscapeSequenceScanningFlags::AtomEscape)
                        && ch != '0' as i32
                    {
                        self.error_at(
                            &diagnostics::Octal_escape_sequences_and_backreferences_are_not_allowed_in_a_character_class_If_this_was_intended_as_an_escape_sequence_use_the_syntax_0_instead,
                            start,
                            self.state.pos - start,
                            &[&arg],
                        );
                    } else {
                        self.error_at(&diagnostics::Octal_escape_sequences_are_not_allowed_Use_the_syntax_0, start, self.state.pos - start, &[&arg]);
                    }
                    return Cow::Owned(rune_string(code));
                }
                Cow::Borrowed(self.slice(start, self.state.pos))
            }
            0x38 | 0x39 /* 8 9 */ => {
                // the invalid '\8' and '\9'
                self.state.token_flags |= TokenFlags::ContainsInvalidEscape;
                if flags.intersects(EscapeSequenceScanningFlags::ReportInvalidEscapeErrors) {
                    if flags.intersects(EscapeSequenceScanningFlags::RegularExpression) && !flags.intersects(EscapeSequenceScanningFlags::AtomEscape) {
                        self.error_at(
                            &diagnostics::Decimal_escape_sequences_and_backreferences_are_not_allowed_in_a_character_class,
                            start,
                            self.state.pos - start,
                            &[],
                        );
                    } else {
                        let text = self.slice(start, self.state.pos);
                        self.error_at(&diagnostics::Escape_sequence_0_is_not_allowed, start, self.state.pos - start, &[&text]);
                    }
                    return Cow::Owned(rune_string(ch));
                }
                Cow::Borrowed(self.slice(start, self.state.pos))
            }
            0x62 /* b */ => Cow::Borrowed("\u{8}"),
            0x74 /* t */ => Cow::Borrowed("\t"),
            0x6E /* n */ => Cow::Borrowed("\n"),
            0x76 /* v */ => Cow::Borrowed("\u{b}"),
            0x66 /* f */ => Cow::Borrowed("\u{c}"),
            0x72 /* r */ => Cow::Borrowed("\r"),
            0x27 /* ' */ => Cow::Borrowed("'"),
            0x22 /* " */ => Cow::Borrowed("\""),
            0x75 /* u */ => {
                // '\uDDDD' and '\u{DDDDDD}'
                let extended = self.char() == '{' as i32;
                self.state.pos -= 2;
                let code_point = self.scan_unicode_escape(flags.intersects(EscapeSequenceScanningFlags::ReportInvalidEscapeErrors));
                if extended {
                    if !flags.intersects(EscapeSequenceScanningFlags::AllowExtendedUnicodeEscape) {
                        self.state.token_flags |= TokenFlags::ContainsInvalidEscape;
                        if flags.intersects(EscapeSequenceScanningFlags::ReportInvalidEscapeErrors) {
                            self.error_at(
                                &diagnostics::Unicode_escape_sequences_are_only_available_when_the_Unicode_u_flag_or_the_Unicode_Sets_v_flag_is_set,
                                start,
                                self.state.pos - start,
                                &[],
                            );
                        }
                    }
                    if code_point < 0 {
                        return Cow::Borrowed(self.slice(start, self.state.pos));
                    }
                    // In string literals, a high surrogate \u{...} followed by a low
                    // surrogate escape forms a single code point, exactly as adjacent
                    // UTF-16 code units would in a JavaScript string.
                    if !flags.intersects(EscapeSequenceScanningFlags::RegularExpression) && stringutil::is_high_surrogate(code_point) {
                        if let Some(combined) = self.scan_low_surrogate_escape(code_point) {
                            return Cow::Owned(rune_string(combined));
                        }
                    }
                    return Cow::Owned(stringutil::encode_js_string_rune(code_point));
                }
                if code_point < 0 {
                    return Cow::Borrowed(self.slice(start, self.state.pos));
                } else if stringutil::is_high_surrogate(code_point) {
                    if !flags.intersects(EscapeSequenceScanningFlags::RegularExpression) {
                        // Combine \uHigh followed by any low surrogate escape (\uLow or
                        // \u{Low}) into a single code point in string literals, matching
                        // how adjacent UTF-16 code units pair in a JavaScript string.
                        if let Some(combined) = self.scan_low_surrogate_escape(code_point) {
                            return Cow::Owned(rune_string(combined));
                        }
                    } else if flags.intersects(EscapeSequenceScanningFlags::AnyUnicodeMode)
                        && self.char() == '\\' as i32
                        && self.char_at(1) == 'u' as i32
                        && self.char_at(2) != '{' as i32
                    {
                        // In regex AnyUnicodeMode, combine \uHigh\uLow so scanClassRanges
                        // can compare the pair numerically. In non-unicode regex mode they
                        // are separate atoms, and extended \u{...} escapes never combine.
                        let saved_pos = self.state.pos;
                        let next_code_point = self.scan_unicode_escape(flags.intersects(EscapeSequenceScanningFlags::ReportInvalidEscapeErrors));
                        if stringutil::is_low_surrogate(next_code_point) {
                            return Cow::Owned(rune_string(stringutil::surrogate_pair_to_code_point(code_point, next_code_point)));
                        }
                        self.state.pos = saved_pos;
                    }
                }
                // Lone surrogate: encode as CESU-8 so it survives losslessly. In a
                // non-unicode regex this also lets scanClassRanges compare it numerically.
                Cow::Owned(stringutil::encode_js_string_rune(code_point))
            }
            0x78 /* x */ => {
                // '\xDD'
                while self.state.pos < start + 4 {
                    if !stringutil::is_hex_digit(self.char()) {
                        self.state.token_flags |= TokenFlags::ContainsInvalidEscape;
                        if flags.intersects(EscapeSequenceScanningFlags::ReportInvalidEscapeErrors) {
                            self.error(&diagnostics::Hexadecimal_digit_expected);
                        }
                        return Cow::Borrowed(self.slice(start, self.state.pos));
                    }
                    self.state.pos += 1;
                }
                self.state.token_flags |= TokenFlags::HexEscape;
                let escaped_value = i64::from_str_radix(self.slice(start + 2, self.state.pos), 16).unwrap_or(0) as i32;
                Cow::Owned(rune_string(escaped_value))
            }
            // when encountering a LineContinuation (i.e. a backslash and a line terminator sequence),
            // the line terminator is interpreted to be "the empty code unit sequence".
            0x0D /* \r */ => {
                if self.char() == '\n' as i32 {
                    self.state.pos += 1;
                }
                Cow::Borrowed("")
            }
            0x0A /* \n */ => Cow::Borrowed(""),
            _ => {
                let mut ch = ch;
                // ch was read as a single byte; for multi-byte UTF-8 characters,
                // we need to decode the full rune and advance past all its bytes.
                if ch >= RUNE_SELF {
                    self.state.pos -= 1; // back up past the single-byte advance
                    let (r, size) = decode_rune(&self.text.as_bytes()[self.state.pos as usize..]);
                    ch = r;
                    self.state.pos += size as i32;
                }
                // LineContinuation: a backslash followed by a line terminator is "the empty code unit sequence".
                if ch == 0x2028 || ch == 0x2029 {
                    return Cow::Borrowed("");
                }
                if flags.intersects(EscapeSequenceScanningFlags::AnyUnicodeMode)
                    || flags.intersects(EscapeSequenceScanningFlags::RegularExpression)
                        && !flags.intersects(EscapeSequenceScanningFlags::AnnexB)
                        && is_identifier_part(ch)
                {
                    self.error_at(&diagnostics::This_character_cannot_be_escaped_in_a_regular_expression, start, self.state.pos - start, &[]);
                }
                if ch < RUNE_SELF {
                    // Single ASCII characters are always slices of the text.
                    return Cow::Borrowed(self.slice(self.state.pos - 1, self.state.pos));
                }
                Cow::Owned(rune_string(ch))
            }
        }
    }

    // Known to be at \u
    pub(crate) fn scan_unicode_escape(&mut self, should_emit_invalid_escape_error: bool) -> i32 {
        self.state.pos += 2;
        let start = self.state.pos;
        let extended = self.char() == '{' as i32;
        let hex_digits;
        if extended {
            self.state.pos += 1;
            hex_digits = self.scan_hex_digits(1, true, false);
        } else {
            self.state.token_flags |= TokenFlags::UnicodeEscape;
            hex_digits = self.scan_hex_digits(4, false, false);
        }
        if hex_digits.is_empty() {
            self.state.token_flags |= TokenFlags::ContainsInvalidEscape;
            if should_emit_invalid_escape_error {
                self.error(&diagnostics::Hexadecimal_digit_expected);
            }
            return -1;
        }
        let hex_value = parse_int_32(hex_digits, 16);
        if extended {
            let mut is_invalid_extended_escape = false;
            if hex_value > 0x10FFFF {
                if should_emit_invalid_escape_error {
                    self.error_at(
                        &diagnostics::An_extended_Unicode_escape_value_must_be_between_0x0_and_0x10FFFF_inclusive,
                        start + 1,
                        self.state.pos - start - 1,
                        &[],
                    );
                }
                is_invalid_extended_escape = true;
            }
            if self.state.pos >= self.end {
                if should_emit_invalid_escape_error {
                    self.error(&diagnostics::Unexpected_end_of_text);
                }
                is_invalid_extended_escape = true;
            } else if self.char() == '}' as i32 {
                self.state.pos += 1;
            } else {
                if should_emit_invalid_escape_error {
                    self.error(&diagnostics::Unterminated_Unicode_escape_sequence);
                }
                is_invalid_extended_escape = true;
            }
            if is_invalid_extended_escape {
                self.state.token_flags |= TokenFlags::ContainsInvalidEscape;
                return -1;
            }
            self.state.token_flags |= TokenFlags::ExtendedUnicodeEscape;
        }
        hex_value as i32
    }

    // scanLowSurrogateEscape attempts to consume a low-surrogate Unicode escape
    // (either '\uLow' or '\u{Low}') immediately following an already-scanned high
    // surrogate and combine them into a single supplementary code point. This
    // mirrors how adjacent UTF-16 code units form a surrogate pair in a JavaScript
    // string, regardless of which escape syntax produced each half. On success it
    // returns the combined code point; otherwise it restores the scanner
    // position and returns None.
    fn scan_low_surrogate_escape(&mut self, high: i32) -> Option<i32> {
        if self.char() != '\\' as i32 || self.char_at(1) != 'u' as i32 {
            return None;
        }
        let saved_pos = self.state.pos;
        let saved_token_flags = self.state.token_flags;
        // Speculatively scan the escape with diagnostics suppressed: if it isn't a
        // low surrogate we rewind below, and the caller re-scans the same escape and
        // reports any error then, so reporting here would duplicate diagnostics.
        let low = self.scan_unicode_escape(false);
        if stringutil::is_low_surrogate(low) {
            return Some(stringutil::surrogate_pair_to_code_point(high, low));
        }
        self.state.pos = saved_pos;
        self.state.token_flags = saved_token_flags;
        None
    }

    // Current character is known to be a backslash. Check for Unicode escape of the form '\uXXXX'
    // or '\u{XXXXXX}' and return code point value if valid Unicode escape is found. Otherwise return -1.
    fn peek_unicode_escape(&mut self) -> i32 {
        if self.char_at(1) == 'u' as i32 {
            let save_pos = self.state.pos;
            let save_token_flags = self.state.token_flags;
            let code_point = self.scan_unicode_escape(false);
            self.state.pos = save_pos;
            self.state.token_flags = save_token_flags;
            return code_point;
        }
        -1
    }

    fn scan_number(&mut self) -> Kind {
        let mut start = self.state.pos;
        let fixed_part: Cow<'static, str>;
        if self.char() == '0' as i32 {
            self.state.pos += 1;
            if self.char() == '_' as i32 {
                self.state.token_flags |= TokenFlags::ContainsSeparator | TokenFlags::ContainsInvalidSeparator;
                self.error_at(&diagnostics::Numeric_separators_are_not_allowed_here, self.state.pos, 1, &[]);
                self.state.pos = start;
                fixed_part = self.scan_number_fragment();
            } else {
                let (digits, is_octal) = self.scan_digits();
                if digits.is_empty() {
                    fixed_part = Cow::Borrowed("0");
                } else if !is_octal {
                    self.state.token_flags |= TokenFlags::ContainsLeadingZero;
                    fixed_part = Cow::Borrowed(digits);
                } else {
                    let val = parse_int_64(digits, 8);
                    self.state.token_value = alloc_str(&val.to_string());
                    self.state.token_flags |= TokenFlags::Octal;
                    let with_minus = self.state.token == Kind::MinusToken;
                    let literal = format!("{}0o{:o}", if with_minus { "-" } else { "" }, val);
                    if with_minus {
                        start -= 1;
                    }
                    self.error_at(&diagnostics::Octal_literals_are_not_allowed_Use_the_syntax_0, start, self.state.pos - start, &[&literal]);
                    return Kind::NumericLiteral;
                }
            }
        } else {
            fixed_part = self.scan_number_fragment();
        }
        let fixed_part_end = self.state.pos;
        let mut fractional_part: Cow<'static, str> = Cow::Borrowed("");
        let mut exponent_preamble = "";
        let mut exponent_part: Cow<'static, str> = Cow::Borrowed("");
        if self.char() == '.' as i32 {
            self.state.pos += 1;
            fractional_part = self.scan_number_fragment();
        }
        let mut end = self.state.pos;
        if self.char() == 'E' as i32 || self.char() == 'e' as i32 {
            self.state.pos += 1;
            self.state.token_flags |= TokenFlags::Scientific;
            if self.char() == '+' as i32 || self.char() == '-' as i32 {
                self.state.pos += 1;
            }
            let start_numeric_part = self.state.pos;
            exponent_part = self.scan_number_fragment();
            if exponent_part.is_empty() {
                self.error(&diagnostics::Digit_expected);
            } else {
                exponent_preamble = self.slice(end, start_numeric_part);
                end = self.state.pos;
            }
        }
        if self.state.token_flags.intersects(TokenFlags::ContainsSeparator) {
            let mut value = String::from(&*fixed_part);
            if !fractional_part.is_empty() {
                value.push('.');
                value.push_str(&fractional_part);
            }
            if !exponent_part.is_empty() {
                value.push_str(exponent_preamble);
                value.push_str(&exponent_part);
            }
            self.state.token_value = self.intern_value(&value);
        } else {
            self.state.token_value = self.slice(start, end);
        }
        if self.state.token_flags.intersects(TokenFlags::ContainsLeadingZero) {
            self.error_at(&diagnostics::Decimals_with_leading_zeros_are_not_allowed, start, self.state.pos - start, &[]);
            self.state.token_value = self.canonical_number(self.state.token_value);
            return Kind::NumericLiteral;
        }
        let result;
        if fixed_part_end == self.state.pos {
            result = self.scan_big_int_suffix();
        } else {
            self.state.token_value = self.canonical_number(self.state.token_value);
            result = Kind::NumericLiteral;
        }
        let (ch, _) = self.char_and_size();
        if is_identifier_start(ch) {
            let id_start = self.state.pos;
            let id = self.scan_identifier_parts(IdentifierVariant::Standard);
            if result != Kind::BigIntLiteral && id.len() == 1 && self.text.as_bytes()[id_start as usize] == b'n' {
                if self.state.token_flags.intersects(TokenFlags::Scientific) {
                    self.error_at(&diagnostics::A_bigint_literal_cannot_use_exponential_notation, start, self.state.pos - start, &[]);
                    return result;
                }
                if fixed_part_end < id_start {
                    self.error_at(&diagnostics::A_bigint_literal_must_be_an_integer, start, self.state.pos - start, &[]);
                    return result;
                }
            }
            self.error_at(
                &diagnostics::An_identifier_or_keyword_cannot_immediately_follow_a_numeric_literal,
                id_start,
                self.state.pos - id_start,
                &[],
            );
            self.state.pos = id_start;
        }
        result
    }

    /// Go `jsnum.FromString(v).String()`, reusing `v` when it is already canonical.
    fn canonical_number(&self, v: &'static str) -> &'static str {
        let s = jsnum::from_string(v).string();
        if s == v {
            return v;
        }
        alloc_str(&s)
    }

    fn scan_number_fragment(&mut self) -> Cow<'static, str> {
        let mut start = self.state.pos;
        let mut allow_separator = false;
        let mut is_previous_token_separator = false;
        let mut result = String::new();
        loop {
            let before = self.state.pos;
            self.scan_ascii_while(|b| b.is_ascii_digit());
            if self.state.pos > before {
                allow_separator = true;
                is_previous_token_separator = false;
            }
            let ch = self.char();
            if ch == '_' as i32 {
                self.state.token_flags |= TokenFlags::ContainsSeparator;
                if allow_separator {
                    allow_separator = false;
                    is_previous_token_separator = true;
                    result.push_str(self.slice(start, self.state.pos));
                } else {
                    self.state.token_flags |= TokenFlags::ContainsInvalidSeparator;
                    if is_previous_token_separator {
                        self.error_at(&diagnostics::Multiple_consecutive_numeric_separators_are_not_permitted, self.state.pos, 1, &[]);
                    } else {
                        self.error_at(&diagnostics::Numeric_separators_are_not_allowed_here, self.state.pos, 1, &[]);
                    }
                }
                self.state.pos += 1;
                start = self.state.pos;
                continue;
            }
            break;
        }
        if is_previous_token_separator {
            self.state.token_flags |= TokenFlags::ContainsInvalidSeparator;
            self.error_at(&diagnostics::Numeric_separators_are_not_allowed_here, self.state.pos - 1, 1, &[]);
        }
        if result.is_empty() {
            return Cow::Borrowed(self.slice(start, self.state.pos));
        }
        result.push_str(self.slice(start, self.state.pos));
        Cow::Owned(result)
    }

    fn scan_digits(&mut self) -> (&'static str, bool) {
        let start = self.state.pos;
        let mut is_octal = true;
        while stringutil::is_digit(self.char()) {
            if !stringutil::is_octal_digit(self.char()) {
                is_octal = false;
            }
            self.state.pos += 1;
        }
        (self.slice(start, self.state.pos), is_octal)
    }

    pub(crate) fn scan_hex_digits(&mut self, min_count: i32, scan_as_many_as_possible: bool, can_have_separators: bool) -> &'static str {
        let mut digit_count = 0;
        let start = self.state.pos;
        let mut allow_separator = false;
        let mut is_previous_token_separator = false;
        while digit_count < min_count || scan_as_many_as_possible {
            let ch = self.char();
            if stringutil::is_hex_digit(ch) {
                allow_separator = can_have_separators;
                is_previous_token_separator = false;
                digit_count += 1;
            } else if can_have_separators && ch == '_' as i32 {
                self.state.token_flags |= TokenFlags::ContainsSeparator;
                if allow_separator {
                    allow_separator = false;
                    is_previous_token_separator = true;
                } else if is_previous_token_separator {
                    self.error_at(&diagnostics::Multiple_consecutive_numeric_separators_are_not_permitted, self.state.pos, 1, &[]);
                } else {
                    self.error_at(&diagnostics::Numeric_separators_are_not_allowed_here, self.state.pos, 1, &[]);
                }
            } else {
                break;
            }
            self.state.pos += 1;
        }
        if is_previous_token_separator {
            self.error_at(&diagnostics::Numeric_separators_are_not_allowed_here, self.state.pos - 1, 1, &[]);
        }
        if digit_count < min_count {
            return "";
        }
        let digits = self.slice(start, self.state.pos);
        if let Some(&cached) = self.hex_digit_cache.get(digits) {
            cached
        } else {
            let original = digits;
            let mut result = Cow::Borrowed(digits);
            if self.state.token_flags.intersects(TokenFlags::ContainsSeparator) {
                result = Cow::Owned(result.replace('_', ""));
            }
            // standardize hex literals to lowercase
            if result.bytes().any(|b| b.is_ascii_uppercase()) {
                result = Cow::Owned(result.to_ascii_lowercase());
            }
            let digits = match result {
                Cow::Borrowed(s) => s,
                Cow::Owned(s) => alloc_str(&s),
            };
            self.pin_unless_source(original, digits);
            self.hex_digit_cache.insert(original, digits);
            digits
        }
    }

    fn scan_binary_or_octal_digits(&mut self, base: i32) -> String {
        let mut sb = String::new();
        let mut allow_separator = false;
        let mut is_previous_token_separator = false;
        loop {
            let ch = self.char();
            if stringutil::is_digit(ch) && ch - ('0' as i32) < base {
                sb.push(ch as u8 as char);
                allow_separator = true;
                is_previous_token_separator = false;
            } else if ch == '_' as i32 {
                self.state.token_flags |= TokenFlags::ContainsSeparator;
                if allow_separator {
                    allow_separator = false;
                    is_previous_token_separator = true;
                } else if is_previous_token_separator {
                    self.error_at(&diagnostics::Multiple_consecutive_numeric_separators_are_not_permitted, self.state.pos, 1, &[]);
                } else {
                    self.error_at(&diagnostics::Numeric_separators_are_not_allowed_here, self.state.pos, 1, &[]);
                }
            } else {
                break;
            }
            self.state.pos += 1;
        }
        if is_previous_token_separator {
            self.error_at(&diagnostics::Numeric_separators_are_not_allowed_here, self.state.pos - 1, 1, &[]);
        }
        sb
    }

    fn scan_big_int_suffix(&mut self) -> Kind {
        if self.char() == 'n' as i32 {
            // Go: tokenValue += "n"; when the value is a text slice directly followed by the `n`
            // this is again a text slice.
            let n = self.slice(self.state.pos, self.state.pos + 1);
            self.state.token_value = self.concat_value(self.state.token_value, n);
            if self.state.token_flags.intersects(TokenFlags::BinaryOrOctalSpecifier) {
                self.state.token_value = alloc_str(&(jsnum::parse_pseudo_big_int(self.state.token_value) + "n"));
            }
            self.state.pos += 1;
            return Kind::BigIntLiteral;
        }
        if let Some(&cached) = self.number_cache.get(self.state.token_value) {
            self.state.token_value = cached;
        } else {
            let token_value = self.canonical_number(self.state.token_value);
            self.pin_unless_source(self.state.token_value, token_value);
            self.number_cache.insert(self.state.token_value, token_value);
            self.state.token_value = token_value;
        }
        Kind::NumericLiteral
    }

    fn scan_invalid_character(&mut self) {
        let (_, size) = self.char_and_size();
        self.error_at(&diagnostics::Invalid_character, self.state.pos, size, &[]);
        self.state.pos += size;
        self.state.token = Kind::Unknown;
    }
}

/// Go `strconv.ParseInt(s, base, 32)` ignoring the error (clamped on overflow).
fn parse_int_32(s: &str, base: u32) -> i64 {
    match i64::from_str_radix(s, base) {
        Ok(v) => v.clamp(i32::MIN as i64, i32::MAX as i64),
        Err(_) => {
            if !s.is_empty() && s.bytes().all(|b| (b as char).is_digit(base)) {
                i32::MAX as i64
            } else {
                0
            }
        }
    }
}

/// Go `strconv.ParseInt(s, base, 64)` ignoring the error (clamped on overflow).
fn parse_int_64(s: &str, base: u32) -> i64 {
    match i64::from_str_radix(s, base) {
        Ok(v) => v,
        Err(_) => {
            if !s.is_empty() && s.bytes().all(|b| (b as char).is_digit(base)) {
                i64::MAX
            } else {
                0
            }
        }
    }
}

// hasJSDocTag reports whether text starts with one of the given tag names followed
// by a valid JSDoc tag terminator (whitespace, '}', '*', or end-of-string).
fn has_jsdoc_tag(text: &[u8], tags: &[&str]) -> bool {
    for tag in tags {
        let tag = tag.as_bytes();
        if !text.starts_with(tag) {
            continue;
        }
        if text.len() == tag.len() {
            return true;
        }
        let ch = text[tag.len()];
        if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' || ch == b'}' || ch == b'*' {
            return true;
        }
    }
    false
}

#[inline]
pub fn get_identifier_token(str: &str) -> Kind {
    let b = str.as_bytes();
    if b.len() >= 2 && b.len() <= 12 && b[0] >= b'a' && b[0] <= b'z' {
        let keyword = text_to_keyword(str);
        if keyword != Kind::Unknown {
            return keyword;
        }
    }
    Kind::Identifier
}

pub fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    for (i, ch) in s.char_indices() {
        let ch = ch as i32;
        if i == 0 && !is_identifier_start(ch) || i != 0 && !is_identifier_part(ch) {
            return false;
        }
    }
    true
}

// Section 6.1.4
pub(crate) fn is_word_character(ch: i32) -> bool {
    stringutil::is_ascii_letter(ch) || stringutil::is_digit(ch) || ch == '_' as i32
}

#[inline]
pub fn is_identifier_start(ch: i32) -> bool {
    stringutil::is_ascii_letter(ch) || ch == '_' as i32 || ch == '$' as i32 || ch >= RUNE_SELF && stringutil::is_unicode_identifier_start(ch)
}

#[inline]
pub fn is_identifier_part(ch: i32) -> bool {
    is_identifier_part_ex(ch, LanguageVariant::Standard)
}

#[inline]
pub fn is_identifier_part_ex(ch: i32, language_variant: LanguageVariant) -> bool {
    is_word_character(ch)
        || ch == '$' as i32
        || ch >= RUNE_SELF && stringutil::is_unicode_identifier_part(ch)
        || language_variant == LanguageVariant::JSX && ch == '-' as i32 // ":" is part of JSXNamespacedName, but not JSXIdentifier.
}

fn token_to_text() -> &'static [&'static str] {
    static T: OnceLock<Vec<&'static str>> = OnceLock::new();
    T.get_or_init(|| {
        // Go fills this from the textToToken map; no two texts share a kind, so the tables give the same result.
        let mut result = vec![""; Kind::Count as usize];
        for &(text, kind) in TEXT_TO_PUNCTUATION.iter().chain(TEXT_TO_KEYWORD) {
            result[kind as usize] = text;
        }
        result
    })
}

pub fn token_to_string(token: Kind) -> &'static str {
    token_to_text()[token as usize]
}

pub fn string_to_token(s: &str) -> Kind {
    match text_to_token().get(s) {
        Some(&kind) => kind,
        None => Kind::Unknown,
    }
}

pub fn get_viable_keyword_suggestions() -> Vec<&'static str> {
    let mut result = Vec::with_capacity(TEXT_TO_KEYWORD.len());
    for &(text, _) in TEXT_TO_KEYWORD {
        if text.len() > 2 {
            result.push(text);
        }
    }
    result
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SkipTriviaOptions {
    pub stop_after_line_break: bool,
    pub stop_at_comments: bool,
    pub in_jsdoc: bool,
}

pub fn skip_trivia(text: &str, pos: i32) -> i32 {
    skip_trivia_ex(text, pos, None)
}

pub fn skip_trivia_ex(text: &str, pos: i32, options: Option<&SkipTriviaOptions>) -> i32 {
    if ast::position_is_synthesized(pos) {
        return pos;
    }
    let default_options = SkipTriviaOptions::default();
    let options = options.unwrap_or(&default_options);

    let bytes = text.as_bytes();
    let text_len = bytes.len() as i32;
    let mut pos = pos;
    let mut can_consume_star = false;
    // Keep in sync with couldStartTrivia
    loop {
        if pos >= text_len {
            return pos;
        }
        let (ch, size) = decode_rune(&bytes[pos as usize..]);
        match ch {
            0x0D | 0x0A => {
                if ch == '\r' as i32 && pos + 1 < text_len && bytes[(pos + 1) as usize] == b'\n' {
                    pos += 1;
                }
                pos += 1;
                if options.stop_after_line_break {
                    return pos;
                }
                can_consume_star = options.in_jsdoc;
                continue;
            }
            0x09 | 0x0B | 0x0C | 0x20 => {
                pos += 1;
                continue;
            }
            0x2F /* / */ => {
                if !options.stop_at_comments && pos + 1 < text_len {
                    if bytes[(pos + 1) as usize] == b'/' {
                        pos += 2;
                        while pos < text_len {
                            let (ch, size) = decode_rune(&bytes[pos as usize..]);
                            if stringutil::is_line_break(ch) {
                                break;
                            }
                            pos += size as i32;
                        }
                        can_consume_star = false;
                        continue;
                    }
                    if bytes[(pos + 1) as usize] == b'*' {
                        pos += 2;
                        while pos < text_len {
                            if bytes[pos as usize] == b'*' && (pos + 1 < text_len) && bytes[(pos + 1) as usize] == b'/' {
                                pos += 2;
                                break;
                            }
                            let (_, size) = decode_rune(&bytes[pos as usize..]);
                            pos += size as i32;
                        }
                        can_consume_star = false;
                        continue;
                    }
                }
            }
            0x3C | 0x7C | 0x3D | 0x3E /* < | = > */ => {
                if is_conflict_marker_trivia(text, pos) {
                    pos = scan_conflict_marker_trivia(text, pos, None);
                    can_consume_star = false;
                    continue;
                }
            }
            0x23 /* # */ => {
                if pos == 0 && is_shebang_trivia(text, pos) {
                    pos = scan_shebang_trivia(text, pos);
                    can_consume_star = false;
                    continue;
                }
            }
            0x2A /* * */ => {
                if can_consume_star {
                    pos += 1;
                    can_consume_star = false;
                    continue;
                }
            }
            _ => {
                if ch > MAX_ASCII_CHARACTER as i32 && stringutil::is_white_space_like(ch) {
                    pos += size as i32;
                    continue;
                }
            }
        }
        return pos;
    }
}

// All conflict markers consist of the same character repeated seven times.  If it is
// a <<<<<<< or >>>>>>> marker then it is also followed by a space.
const MERGE_CONFLICT_MARKER_LENGTH: i32 = "<<<<<<<".len() as i32;
const MAX_ASCII_CHARACTER: u8 = 127;

pub(crate) fn is_conflict_marker_trivia(text: &str, pos: i32) -> bool {
    if pos < 0 {
        panic!("pos < 0");
    }
    let bytes = text.as_bytes();
    let len = bytes.len() as i32;

    // Fast reject: a conflict marker is the same byte repeated seven times. If the
    // second byte differs (the overwhelmingly common case for `<`, `>`, `=`, `|`
    // tokens), it cannot be a marker, so skip the line-start check entirely.
    if pos + 1 >= len || bytes[(pos + 1) as usize] != bytes[pos as usize] {
        return false;
    }

    // Conflict markers must be at the start of a line.
    let mut at_line_start = pos == 0 || stringutil::is_line_break(bytes[(pos - 1) as usize] as i32);
    if !at_line_start && pos >= 2 {
        let (prev, _) = decode_last_rune(&bytes[..(pos - 2) as usize]);
        at_line_start = stringutil::is_line_break(prev);
    }
    if at_line_start {
        let ch = bytes[pos as usize];

        if (pos + MERGE_CONFLICT_MARKER_LENGTH) < len {
            for i in 0..MERGE_CONFLICT_MARKER_LENGTH {
                if bytes[(pos + i) as usize] != ch {
                    return false;
                }
            }

            return ch == b'=' || bytes[(pos + MERGE_CONFLICT_MARKER_LENGTH) as usize] == b' ';
        }
    }

    false
}

pub(crate) fn scan_conflict_marker_trivia(text: &str, pos: i32, report_error: Option<&mut dyn FnMut(&'static Message, i32, i32)>) -> i32 {
    if let Some(report_error) = report_error {
        report_error(&diagnostics::Merge_conflict_marker_encountered, pos, MERGE_CONFLICT_MARKER_LENGTH);
    }
    let bytes = text.as_bytes();
    let mut pos = pos;
    let (mut ch, mut size) = decode_rune(&bytes[pos as usize..]);
    let length = bytes.len() as i32;

    if ch == '<' as i32 || ch == '>' as i32 {
        while pos < length && !stringutil::is_line_break(ch) {
            pos += size as i32;
            (ch, size) = decode_rune(&bytes[pos as usize..]);
        }
    } else {
        if ch != '|' as i32 && ch != '=' as i32 {
            panic!("Assertion failed: ch must be either '|' or '='");
        }
        // Consume everything from the start of a ||||||| or ======= marker to the start
        // of the next ======= or >>>>>>> marker.
        while pos < length {
            let current_char = bytes[pos as usize];
            if (current_char == b'=' || current_char == b'>') && current_char as i32 != ch && is_conflict_marker_trivia(text, pos) {
                break;
            }

            pos += 1;
        }
    }

    pos
}

pub(crate) fn is_shebang_trivia(text: &str, pos: i32) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    if pos != 0 {
        panic!("Shebangs check must only be done at the start of the file");
    }
    bytes[0] == b'#' && bytes[1] == b'!'
}

pub(crate) fn scan_shebang_trivia(text: &str, pos: i32) -> i32 {
    let bytes = text.as_bytes();
    let mut pos = pos + 2;
    while (pos as usize) < bytes.len() {
        let (ch, size) = decode_rune(&bytes[pos as usize..]);
        if stringutil::is_line_break(ch) {
            break;
        }
        pos += size as i32;
    }
    pos
}

pub fn get_shebang(text: &str) -> &str {
    if !is_shebang_trivia(text, 0) {
        return "";
    }

    let end = scan_shebang_trivia(text, 0);
    &text[..end as usize]
}

pub fn get_scanner_for_source_file(source_file: P<SourceFile>, pos: i32) -> Scanner {
    let mut s = Scanner::new();
    s.text = source_file.text();
    s.state.pos = pos;
    s.end = s.text.len() as i32;
    s.language_variant = source_file.language_variant();
    s.scan();
    s
}

pub fn scan_token_at_position(source_file: P<SourceFile>, pos: i32) -> Kind {
    let s = get_scanner_for_source_file(source_file, pos);
    s.state.token
}

pub fn get_range_of_token_at_position(source_file: P<SourceFile>, pos: i32) -> TextRange {
    let s = get_scanner_for_source_file(source_file, pos);
    TextRange::new(s.state.token_start, s.state.pos)
}

pub fn get_token_pos_of_node(node: P<Node>, source_file: P<SourceFile>, include_jsdoc: bool) -> i32 {
    // With nodes that have no width (i.e. 'Missing' nodes), we actually *don't*
    // want to skip trivia because this will launch us forward to the next token.
    if ast::node_is_missing(Some(node)) {
        return node.pos();
    }
    if ast::is_jsdoc_node(node) || node.kind() == Kind::JsxText {
        // JsxText cannot actually contain comments, even though the scanner will think it sees comments
        return skip_trivia_ex(source_file.text(), node.pos(), Some(&SkipTriviaOptions { stop_at_comments: true, ..Default::default() }));
    }
    if include_jsdoc {
        let jsdoc = node.jsdoc(Some(source_file.get()));
        if !jsdoc.is_empty() {
            return get_token_pos_of_node(jsdoc[0], source_file, false /*includeJSDoc*/);
        }
    }
    skip_trivia_ex(
        source_file.text(),
        node.pos(),
        Some(&SkipTriviaOptions { in_jsdoc: node.flags().intersects(NodeFlags::JSDoc), ..Default::default() }),
    )
}

fn get_error_range_for_arrow_function(source_file: P<SourceFile>, node: P<Node>) -> TextRange {
    let pos = skip_trivia(source_file.text(), node.pos());
    let body = node.body();
    if let Some(body) = body {
        if body.kind() == Kind::Block {
            let start_line = get_ecma_line_of_position(&*source_file, body.pos());
            let end_line = get_ecma_line_of_position(&*source_file, body.end());
            if start_line < end_line {
                // The arrow function spans multiple lines, make the error span be the first line, inclusive.
                return TextRange::new(pos, get_ecma_end_line_position(source_file, start_line) + 1);
            }
        }
    }
    TextRange::new(pos, node.end())
}

fn find_originating_jsdoc_satisfies_tag(source_file: P<SourceFile>, node: P<Node>) -> Option<P<Node>> {
    let target_type = node.as_satisfies_expression().type_;
    if !target_type.flags().intersects(NodeFlags::Reparsed) {
        return None;
    }
    let mut current = node.parent();
    while let Some(cur) = current {
        current = cur.parent();
        if !cur.flags().intersects(NodeFlags::HasJSDoc) {
            continue;
        }
        let mut first_satisfies_tag: Option<P<Node>> = None;
        for &js_doc in cur.eager_jsdoc(Some(source_file.get())) {
            if let Some(tags) = js_doc.as_jsdoc().tags {
                for &tag in tags.nodes() {
                    if !ast::is_jsdoc_satisfies_tag(tag) {
                        continue;
                    }
                    if first_satisfies_tag.is_none() {
                        first_satisfies_tag = Some(tag);
                    }
                    let type_expr = tag.as_jsdoc_satisfies_tag().type_expression;
                    if let Some(t) = type_expr.type_node() {
                        if t.loc() == target_type.loc() {
                            return Some(tag);
                        }
                    }
                }
            }
        }
        return first_satisfies_tag;
    }
    None
}

pub fn get_error_range_for_node(source_file: P<SourceFile>, node: P<Node>) -> TextRange {
    let mut error_node = Some(node);
    match node.kind() {
        Kind::SourceFile => {
            let pos = skip_trivia(source_file.text(), 0);
            if pos as usize == source_file.text().len() {
                return TextRange::new(0, 0);
            }
            return get_range_of_token_at_position(source_file, pos);
        }
        // This list is a work in progress. Add missing node kinds to improve their error spans
        Kind::FunctionDeclaration
        | Kind::MethodDeclaration
        | Kind::VariableDeclaration
        | Kind::BindingElement
        | Kind::ClassDeclaration
        | Kind::InterfaceDeclaration
        | Kind::ModuleDeclaration
        | Kind::EnumDeclaration
        | Kind::EnumMember
        | Kind::FunctionExpression
        | Kind::GetAccessor
        | Kind::SetAccessor
        | Kind::TypeAliasDeclaration
        | Kind::JSTypeAliasDeclaration
        | Kind::PropertyDeclaration
        | Kind::PropertySignature
        | Kind::NamespaceImport => {
            if (node.kind() == Kind::FunctionDeclaration || node.kind() == Kind::MethodDeclaration)
                && node.flags().intersects(NodeFlags::Reparsed)
            {
                error_node = Some(node);
            } else {
                error_node = ast::get_name_of_declaration(Some(node));
            }
        }
        Kind::ClassExpression => {
            error_node = node.name();
        }
        Kind::ArrowFunction => {
            return get_error_range_for_arrow_function(source_file, node);
        }
        Kind::CaseClause | Kind::DefaultClause => {
            let start = skip_trivia(source_file.text(), node.pos());
            let mut end = node.end();
            let statements = node.statements();
            if !statements.is_empty() {
                end = statements[0].pos();
            }
            return TextRange::new(start, end);
        }
        Kind::ReturnStatement | Kind::YieldExpression => {
            let pos = skip_trivia(source_file.text(), node.pos());
            return get_range_of_token_at_position(source_file, pos);
        }
        Kind::SatisfiesExpression => {
            if let Some(js_doc_satisfies_tag) = find_originating_jsdoc_satisfies_tag(source_file, node) {
                let pos = skip_trivia(source_file.text(), js_doc_satisfies_tag.tag_name().pos());
                return get_range_of_token_at_position(source_file, pos);
            }
            let pos = skip_trivia(source_file.text(), node.as_satisfies_expression().expression.end());
            return get_range_of_token_at_position(source_file, pos);
        }
        Kind::Constructor => {
            if node.flags().intersects(NodeFlags::Reparsed) {
                error_node = Some(node);
            } else {
                let mut scanner = get_scanner_for_source_file(source_file, node.pos());
                let start = scanner.token_start();
                while scanner.token() != Kind::ConstructorKeyword && scanner.token() != Kind::StringLiteral && scanner.token() != Kind::EndOfFile {
                    scanner.scan();
                }
                return TextRange::new(start, scanner.token_end());
            }
        }
        _ => {}
    }
    let Some(error_node) = error_node else {
        // If we don't have a better node, then just set the error on the first token of
        // construct.
        return get_range_of_token_at_position(source_file, node.pos());
    };
    let mut pos = error_node.pos();
    if !ast::node_is_missing(Some(error_node)) && !ast::is_jsx_text(error_node) {
        pos = skip_trivia(source_file.text(), pos);
    }
    TextRange::new(pos, error_node.end())
}

pub fn compute_line_of_position(line_starts: &[TextPos], pos: i32) -> i32 {
    let mut low: i32 = 0;
    let mut high: i32 = line_starts.len() as i32 - 1;
    while low <= high {
        let middle = low + ((high - low) >> 1);
        let value = line_starts[middle as usize];
        if value < pos {
            low = middle + 1;
        } else if value > pos {
            high = middle - 1;
        } else {
            return middle;
        }
    }
    low - 1
}

pub fn get_ecma_line_starts<S: SourceFileLike + ?Sized>(source_file: &S) -> &[TextPos] {
    source_file.ecma_line_map()
}

pub fn get_ecma_line_of_position(source_file: &(impl SourceFileLike + ?Sized), pos: i32) -> i32 {
    let line_map = get_ecma_line_starts(source_file);
    compute_line_of_position(line_map, pos)
}

// GetECMALineAndUTF16CharacterOfPosition returns the 0-based line number and the
// UTF-16 code unit offset from the start of that line for the given byte position.
// Uses ECMAScript line separators (LF, CR, CRLF, LS, PS).
pub fn get_ecma_line_and_utf16_character_of_position(source_file: &(impl SourceFileLike + ?Sized), pos: i32) -> (i32, UTF16Offset) {
    let line_map = get_ecma_line_starts(source_file);
    let line = compute_line_of_position(line_map, pos);
    let character = tsrs_core::utf16_len(&source_file.text()[line_map[line as usize] as usize..pos as usize]);
    (line, character)
}

// GetECMALineAndByteOffsetOfPosition returns the 0-based line number and the
// raw UTF-8 byte offset from the start of that line for the given byte position.
// Uses ECMAScript line separators (LF, CR, CRLF, LS, PS).
// Unlike GetECMALineAndUTF16CharacterOfPosition, the offset is in bytes, not UTF-16 code units.
pub fn get_ecma_line_and_byte_offset_of_position(source_file: &(impl SourceFileLike + ?Sized), pos: i32) -> (i32, i32) {
    let line_map = get_ecma_line_starts(source_file);
    let line = compute_line_of_position(line_map, pos);
    let byte_offset = pos - line_map[line as usize];
    (line, byte_offset)
}

pub fn get_ecma_end_line_position(source_file: P<SourceFile>, line: i32) -> i32 {
    let text = source_file.text().as_bytes();
    let mut pos = get_ecma_line_starts(&*source_file)[line as usize];
    loop {
        let (ch, size) = decode_rune(&text[pos as usize..]);
        if size == 0 || stringutil::is_line_break(ch) {
            return pos - 1;
        }
        pos += size as i32;
    }
}

// GetECMAPositionOfLineAndUTF16Character converts a 0-based line number and UTF-16
// code unit character offset back to an absolute byte position in the source text.
// Uses ECMAScript line separators.
pub fn get_ecma_position_of_line_and_utf16_character(source_file: &(impl SourceFileLike + ?Sized), line: i32, character: UTF16Offset) -> i32 {
    let line_starts = get_ecma_line_starts(source_file);
    compute_position_of_line_and_utf16_character(line_starts, line, character, source_file.text(), false)
}

// GetECMAPositionOfLineAndByteOffset converts a 0-based line number and byte offset
// from line start back to an absolute byte position in the source text.
// Uses ECMAScript line separators.
pub fn get_ecma_position_of_line_and_byte_offset(source_file: &(impl SourceFileLike + ?Sized), line: i32, byte_offset: i32) -> i32 {
    compute_position_of_line_and_byte_offset(get_ecma_line_starts(source_file), line, byte_offset)
}

// ComputePositionOfLineAndByteOffset computes a byte position from a line and
// raw byte offset from the line start. This is a simple addition with validation.
pub fn compute_position_of_line_and_byte_offset(line_starts: &[TextPos], line: i32, byte_offset: i32) -> i32 {
    if line < 0 || line as usize >= line_starts.len() {
        panic!("Bad line number. Line: {}, lineStarts.length: {}.", line, line_starts.len());
    }
    line_starts[line as usize] + byte_offset
}

// ComputePositionOfLineAndUTF16Character converts a line and UTF-16 character offset
// back to a byte position. The character parameter is measured in UTF-16 code units.
// It scans from the line start to correctly handle multi-byte characters.
// When allowEdits is true, out-of-range values are clamped instead of panicking.
pub fn compute_position_of_line_and_utf16_character(
    line_starts: &[TextPos],
    line: i32,
    character: UTF16Offset,
    text: &str,
    allow_edits: bool,
) -> i32 {
    let mut line = line;
    if line < 0 || line as usize >= line_starts.len() {
        if allow_edits {
            // Clamp line to nearest allowable value
            if line < 0 {
                line = 0;
            } else if line as usize >= line_starts.len() {
                line = line_starts.len() as i32 - 1;
            }
        } else {
            panic!("Bad line number. Line: {}, lineStarts.length: {}.", line, line_starts.len());
        }
    }

    let line_start = line_starts[line as usize];
    let text_len = text.len() as i32;

    if character > 0 {
        // UTF-16 character offset: scan from line start counting UTF-16 code units.
        let mut line_end = text_len;
        if ((line + 1) as usize) < line_starts.len() {
            line_end = line_starts[(line + 1) as usize];
        }
        let mut utf16_count: UTF16Offset = 0;
        let mut pos = line_start;
        while pos < line_end {
            if utf16_count >= character {
                break;
            }
            let (r, size) = decode_rune(&text.as_bytes()[pos as usize..]);
            utf16_count += if r >= 0x10000 { 2 } else { 1 };
            pos += size as i32;
        }
        if !allow_edits {
            if pos == line_end && utf16_count < character {
                panic!("Bad UTF-16 character offset. Line: {}, character: {}.", line, character);
            }
            assert!(pos <= text_len);
            return pos;
        }
        if pos > text_len {
            return text_len;
        }
        return pos;
    }

    // Character is 0: line start position.
    let res = line_start;

    if allow_edits {
        if res > text_len {
            return text_len;
        }
        return res;
    }
    assert!(res <= text_len); // Allow single character overflow for trailing newline
    res
}

/// Go `(*ast.NodeFactory).NewCommentRange`; the factory argument of `GetLeadingCommentRanges` /
/// `GetTrailingCommentRanges` only served this constructor and is dropped.
fn new_comment_range(kind: Kind, pos: i32, end: i32, has_trailing_new_line: bool) -> CommentRange {
    CommentRange { text_range: TextRange::new(pos, end), kind, has_trailing_new_line }
}

pub fn get_leading_comment_ranges(text: &'static str, pos: i32) -> CommentRangeIter {
    iterate_comment_ranges(text, pos, false)
}

pub fn get_trailing_comment_ranges(text: &'static str, pos: i32) -> CommentRangeIter {
    iterate_comment_ranges(text, pos, true)
}

/*
Returns an iterator over each comment range following the provided position.
Single-line comment ranges include the leading double-slash characters but not the ending
line break. Multi-line comment ranges include the leading slash-asterisk and trailing
asterisk-slash characters.
*/
fn iterate_comment_ranges(text: &'static str, pos: i32, trailing: bool) -> CommentRangeIter {
    let mut pos = pos;
    let mut collecting = trailing;
    if pos == 0 {
        collecting = true;
        if is_shebang_trivia(text, pos) {
            pos = scan_shebang_trivia(text, pos);
        }
    }
    CommentRangeIter {
        text,
        pos,
        trailing,
        collecting,
        pending_pos: 0,
        pending_end: 0,
        pending_kind: Kind::Unknown,
        pending_has_trailing_new_line: false,
        has_pending_comment_range: false,
        done: false,
    }
}

/// The Go `iter.Seq[ast.CommentRange]` returned by `GetLeadingCommentRanges` /
/// `GetTrailingCommentRanges`, as an explicit state machine.
pub struct CommentRangeIter {
    text: &'static str,
    pos: i32,
    trailing: bool,
    collecting: bool,
    pending_pos: i32,
    pending_end: i32,
    pending_kind: Kind,
    pending_has_trailing_new_line: bool,
    has_pending_comment_range: bool,
    done: bool,
}

impl Iterator for CommentRangeIter {
    type Item = CommentRange;

    fn next(&mut self) -> Option<CommentRange> {
        if self.done {
            return None;
        }
        let text = self.text.as_bytes();
        let len = text.len() as i32;
        'scan: while self.pos >= 0 && self.pos < len {
            let (ch, size) = decode_rune(&text[self.pos as usize..]);
            match ch {
                0x0D | 0x0A => {
                    if ch == '\r' as i32 && self.pos + 1 < len && text[(self.pos + 1) as usize] == b'\n' {
                        self.pos += 1;
                    }
                    self.pos += 1;
                    if self.trailing {
                        break 'scan;
                    }

                    self.collecting = true;
                    if self.has_pending_comment_range {
                        self.pending_has_trailing_new_line = true;
                    }

                    continue;
                }
                0x09 | 0x0B | 0x0C | 0x20 => {
                    self.pos += 1;
                    continue;
                }
                0x2F /* / */ => {
                    let next_char = if self.pos + 1 < len { text[(self.pos + 1) as usize] } else { 0 };
                    let mut has_trailing_new_line = false;
                    if next_char == b'/' || next_char == b'*' {
                        let kind = if next_char == b'/' { Kind::SingleLineCommentTrivia } else { Kind::MultiLineCommentTrivia };

                        let start_pos = self.pos;
                        self.pos += 2;
                        if next_char == b'/' {
                            while self.pos < len {
                                let (c, s) = decode_rune(&text[self.pos as usize..]);
                                if stringutil::is_line_break(c) {
                                    has_trailing_new_line = true;
                                    break;
                                }
                                self.pos += s as i32;
                            }
                        } else if let Some(i) = memchr::memmem::find(&text[self.pos as usize..], b"*/") {
                            self.pos += i as i32 + 2;
                        } else {
                            self.pos = len;
                        }

                        if self.collecting {
                            let mut result = None;
                            if self.has_pending_comment_range {
                                result = Some(new_comment_range(
                                    self.pending_kind,
                                    self.pending_pos,
                                    self.pending_end,
                                    self.pending_has_trailing_new_line,
                                ));
                            }

                            self.pending_pos = start_pos;
                            self.pending_end = self.pos;
                            self.pending_kind = kind;
                            self.pending_has_trailing_new_line = has_trailing_new_line;
                            self.has_pending_comment_range = true;
                            if result.is_some() {
                                return result;
                            }
                        }

                        continue;
                    }
                    break 'scan;
                }
                _ => {
                    if ch > 0x7F && stringutil::is_white_space_like(ch) {
                        if self.has_pending_comment_range && stringutil::is_line_break(ch) {
                            self.pending_has_trailing_new_line = true;
                        }
                        self.pos += size as i32;
                        continue;
                    }
                    break 'scan;
                }
            }
        }

        self.done = true;
        if self.has_pending_comment_range {
            return Some(new_comment_range(self.pending_kind, self.pending_pos, self.pending_end, self.pending_has_trailing_new_line));
        }
        None
    }
}
