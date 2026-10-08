//! tsrs-only: member lists of unchecked declaration files parsed and bound on first use
//! (notes/mem-lazy-dts-members.md).
//!
//! In a CLI run with `skipLibCheck`, the parser parses the member list of an interface, class or type literal in a
//! declaration file as usual, and if the list is plain (no diagnostic, no `this`, no import type, no body, decorator,
//! `async` or initializer, no eager JSDoc, no comment directive, and the arena can discard it), it throws the nodes
//! away and stores a `LazyNodeList` instead: the owner node, where the list starts and the parser context there. The
//! `NodeList` of the owner points at the record (a long-form `ThinSlice` whose slice is `LAZY_HEAD`). The binder
//! skips such a list and records its own state at that point (`LazyBindContext`) on the record and on the owner's
//! symbol (`Symbol::set_lazy_list`). The first reader of the list's nodes (`NodeList::nodes`) or of the owner symbol's
//! `members` / `exports` parses the list again from the recorded position (the parser hook) and binds it with the
//! recorded binder state (the binder hook) on its own thread, publishes the nodes, and every later reader gets the
//! same slice. A second declaration merged into the owner symbol in the same file forces the pending list of the
//! first while the file is bound, so symbol tables are filled in source order (binder.rs `force_pending_lazy_list`).
//!
//! States: `PENDING` (parsed lazily, the binder has not reached it: readers see no members, which only the parser
//! and binder of the file can observe), `DEFERRED` (the binder skipped it: the next reader parses and binds it),
//! `UNBOUND` (the file was bound without reaching it: the next reader only parses it), `FORCING`, `DONE`.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use tsrs_core::{OwnedCell, P};

use crate::ast::{Node, SourceFile};
use crate::flow::FlowNode;
use crate::nodeflags::NodeFlags;

pub const PENDING: u8 = 0;
pub const DEFERRED: u8 = 1;
pub const UNBOUND: u8 = 2;
pub const FORCING: u8 = 3;
pub const DONE: u8 = 4;

/// `tsrs_parser::ParsingContext::ClassMembers as u8` (checked there).
pub const CLASS_MEMBERS: u8 = 5;

/// The slice a lazy `NodeList`'s long-form `ThinSlice` reads (`LazyNodeList::head`); its data pointer tells a lazy
/// list from a long one.
static LAZY_HEAD_DATA: [P<Node>; 0] = [];

#[inline]
pub(crate) fn is_lazy_head(s: &'static [P<Node>]) -> bool {
    std::ptr::eq(s.as_ptr(), LAZY_HEAD_DATA.as_ptr())
}

/// The binder state at the point where the binder skipped a lazy member list (`tsrs_binder`).
#[derive(Clone, Copy)]
pub struct LazyBindContext {
    pub container: Option<P<Node>>,
    pub this_container: Option<P<Node>>,
    pub block_scope_container: Option<P<Node>>,
    pub last_container: Option<P<Node>>,
    pub current_flow: Option<P<FlowNode>>,
    pub unreachable_flow: P<FlowNode>,
}

#[repr(C)]
pub struct LazyNodeList {
    /// First, so that a `&LazyNodeList` and the `&'static [P<Node>]` its `NodeList` reads have the same address.
    head: &'static [P<Node>],
    state: AtomicU8,
    /// The forcing thread (`thread_token`), so its own reads during the force return the list being bound.
    forcer: AtomicU64,
    /// Written once by the forcing thread before `DONE` (Release); read after an Acquire load of `DONE`, or by the
    /// forcing thread itself.
    nodes: OwnedCell<&'static [P<Node>]>,
    /// Set right after the parser creates the owner node (the list is parsed first).
    owner: OwnedCell<Option<P<Node>>>,
    /// Where the parser resumes: the end of the `{` token.
    pub open_end: i32,
    /// `pos` / `end` of the list (the list's `loc`).
    pub pos: i32,
    pub end: i32,
    /// The parsing context of the list (`tsrs_parser`'s `ParsingContext`), the parser's context flags and the
    /// enclosing parsing contexts at the list.
    pub parsing_context: u8,
    pub context_flags: NodeFlags,
    pub parsing_contexts: u32,
    /// Set by the binder before `DEFERRED` (single-threaded, before the program is shared).
    bind: OwnedCell<Option<P<LazyBindContext>>>,
}

// SAFETY: shared between checker threads under the state protocol above: `nodes` is written once by the forcing
// thread before the Release store of `DONE` and read after an Acquire load of it; `owner` and `bind` are written by
// the file's parser and binder before the program is shared and only read afterwards.
unsafe impl Sync for LazyNodeList {}
// SAFETY: as above.
unsafe impl Send for LazyNodeList {}

/// Parses a lazy list again (the parser hook): returns its members, parents set to `owner`.
pub type ParseHook = fn(&LazyNodeList, P<SourceFile>) -> &'static [P<Node>];
/// Binds the members of a lazy list with its recorded binder state (the binder hook).
pub type BindHook = fn(&LazyNodeList, P<SourceFile>, &'static [P<Node>]);

static PARSE_HOOK: OnceLock<ParseHook> = OnceLock::new();
static BIND_HOOK: OnceLock<BindHook> = OnceLock::new();

pub fn set_parse_hook(hook: ParseHook) {
    let _ = PARSE_HOOK.set(hook);
}

pub fn set_bind_hook(hook: BindHook) {
    let _ = BIND_HOOK.set(hook);
}

static WAIT: (Mutex<()>, Condvar) = (Mutex::new(()), Condvar::new());

/// `TSRS_LAZY_DTS=stats`: lists made lazy, deferred by the binder, parsed again (all, and while their file was bound).
pub static STATS: [AtomicU64; 5] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

pub fn note(i: usize) {
    STATS[i].fetch_add(1, Ordering::Relaxed);
}

/// One line for `TSRS_LAZY_DTS=stats`.
pub fn stats_line() -> String {
    let v: Vec<u64> = STATS.iter().map(|c| c.load(Ordering::Relaxed)).collect();
    format!("lazy-dts: {} lists lazy, {} deferred by the binder, {} never reached by it, {} parsed again ({} while their file was bound)\n", v[0], v[1], v[4], v[2], v[3])
}

fn thread_token() -> u64 {
    thread_local! {
        static TOKEN: u8 = const { 0 };
    }
    TOKEN.with(|t| t as *const u8 as u64)
}

impl LazyNodeList {
    pub fn new(
        open_end: i32,
        pos: i32,
        end: i32,
        parsing_context: u8,
        context_flags: NodeFlags,
        parsing_contexts: u32,
    ) -> LazyNodeList {
        LazyNodeList {
            head: &LAZY_HEAD_DATA,
            state: AtomicU8::new(PENDING),
            forcer: AtomicU64::new(0),
            nodes: OwnedCell::new(&[]),
            owner: OwnedCell::new(None),
            open_end,
            pos,
            end,
            parsing_context,
            context_flags,
            parsing_contexts,
            bind: OwnedCell::new(None),
        }
    }

    pub(crate) fn head_ref(&'static self) -> &'static &'static [P<Node>] {
        &self.head
    }

    /// # Safety
    /// `head` is the `head` field of a live `LazyNodeList` (a lazy `NodeList`'s long-form slice).
    pub(crate) unsafe fn from_head(head: &'static &'static [P<Node>]) -> &'static LazyNodeList {
        // SAFETY: `head` is the first field of a `repr(C)` `LazyNodeList` (this function's contract).
        unsafe { &*(head as *const &'static [P<Node>] as *const LazyNodeList) }
    }

    /// Whether binding the list adds to the owner symbol's `exports` (static members of a class) as well as to its
    /// `members`. `tsrs_parser`'s `ParsingContext::ClassMembers` is 5.
    #[inline]
    pub fn fills_exports(&self) -> bool {
        self.parsing_context == CLASS_MEMBERS
    }

    #[inline]
    pub fn state(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }

    /// The owner is set once, right after the parser created the owner node (parse time, one thread).
    pub fn set_owner(&self, owner: P<Node>) {
        self.owner.set(Some(owner));
    }

    pub fn owner(&self) -> P<Node> {
        self.owner.get().expect("a lazy list's owner")
    }

    /// Binder: the list was skipped with this state; the next reader parses and binds it.
    pub fn defer(&self, ctx: P<LazyBindContext>) {
        note(1);
        self.bind.set(Some(ctx));
        self.state.store(DEFERRED, Ordering::Release);
    }

    /// Binder: the file was bound without reaching the list; the next reader only parses it.
    pub fn mark_unbound(&self) {
        if self.state.load(Ordering::Relaxed) == PENDING {
            note(4);
            self.state.store(UNBOUND, Ordering::Release);
        }
    }

    pub fn bind_context(&self) -> Option<P<LazyBindContext>> {
        self.bind.get()
    }

    /// Binder (file binding, unusual context): parse the list now; the caller binds it like an eager list.
    pub fn parse_now(&self, file: P<SourceFile>) -> &'static [P<Node>] {
        debug_assert_eq!(self.state.load(Ordering::Relaxed), PENDING);
        note(2);
        note(3);
        let nodes = (PARSE_HOOK.get().expect("lazy list parse hook"))(self, file);
        self.nodes.set(nodes);
        self.state.store(DONE, Ordering::Release);
        nodes
    }

    /// The members, parsed (and bound) on first use. `PENDING`: none yet (only the file's parser and binder read it
    /// in that state).
    #[cold]
    #[inline(never)]
    pub fn force_nodes(&self) -> &'static [P<Node>] {
        loop {
            match self.state.load(Ordering::Acquire) {
                DONE => return self.nodes.get(),
                PENDING => return &[],
                s @ (DEFERRED | UNBOUND) => {
                    if self.state.compare_exchange(s, FORCING, Ordering::Acquire, Ordering::Acquire).is_ok() {
                        self.forcer.store(thread_token(), Ordering::Relaxed);
                        return self.force(s == DEFERRED);
                    }
                }
                _ => {
                    // Relaxed: the forcer stores its token right after winning the exchange, on its own thread; any
                    // other value means another thread, which this one must wait for.
                    if self.forcer.load(Ordering::Relaxed) == thread_token() {
                        return self.nodes.get();
                    }
                    let (lock, cv) = &WAIT;
                    let mut g = lock.lock().unwrap();
                    while self.state.load(Ordering::Acquire) == FORCING {
                        g = cv.wait(g).unwrap();
                    }
                }
            }
        }
    }

    fn force(&self, bind: bool) -> &'static [P<Node>] {
        note(2);
        let file = crate::utilities_1::get_source_file_of_node(Some(self.owner())).expect("a lazy list's file");
        // A checker may be inside a scratch region or another allocation scope: the tree is shared and permanent.
        let _escape = tsrs_core::arena::escape_scratch();
        let _arena = tsrs_core::arena::enter_thread_arena();
        let nodes = (PARSE_HOOK.get().expect("lazy list parse hook"))(self, file);
        self.nodes.set(nodes);
        if bind {
            (BIND_HOOK.get().expect("lazy list bind hook"))(self, file, nodes);
        }
        self.state.store(DONE, Ordering::Release);
        let (lock, cv) = &WAIT;
        drop(lock.lock().unwrap());
        cv.notify_all();
        nodes
    }

    /// Forces the list if a reader could see it now (used by `Symbol::members` / `exports`).
    #[inline]
    pub fn ensure(&self) {
        if self.state.load(Ordering::Acquire) != DONE {
            self.force_nodes();
        }
    }

    /// Binder (while the file is bound): a declaration merges into the owner symbol, so the deferred list is parsed
    /// and bound by `bind` now, before the merge.
    pub fn force_inline(&self, file: P<SourceFile>, bind: &mut dyn FnMut(&'static [P<Node>])) {
        if self.state.compare_exchange(DEFERRED, FORCING, Ordering::Acquire, Ordering::Acquire).is_err() {
            return;
        }
        self.forcer.store(thread_token(), Ordering::Relaxed);
        note(2);
        note(3);
        let nodes = (PARSE_HOOK.get().expect("lazy list parse hook"))(self, file);
        self.nodes.set(nodes);
        bind(nodes);
        self.state.store(DONE, Ordering::Release);
    }
}
