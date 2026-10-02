// fsevents_darwin_ffi.go: macOS CoreFoundation / CoreServices / libdispatch FFI.
//
// Go reaches these C functions without cgo (`//go:cgo_import_dynamic` + assembly trampolines) and delivers the
// FSEvents callback through an assembly function that copies the payload into a pipe read by a Go goroutine,
// because Go code cannot run on a foreign (GCD) thread without cgo. Rust calls the same C API directly through
// `extern "C"` declarations (the frameworks are linked with `#[link(kind = "framework")]`), and the callback is
// an `extern "C" fn` that runs the event classification (`fs_events_callback`) on the stream's serial GCD queue
// thread itself; the pipe and event-loop goroutine are not needed. Teardown keeps Go's order (stop, invalidate,
// wait for the queue to drain, release), which still guarantees no callback runs after `teardown_stream`.
//
// All `unsafe` of the FSEvents backend is confined to this file: the C declarations, the wrappers below (each
// documents the invariant it relies on) and the callback.

use std::ffi::{c_char, c_void, CString};
use std::sync::Arc;

use crate::fsevents_darwin::{fs_events_callback, fseventsWatchSnapshot};

type CFTypeRef = *const c_void;
type CFIndex = isize;
type Boolean = u8;
type FSEventStreamRef = *mut c_void;
type dispatch_queue_t = *mut c_void;

// FSEventStreamContext mirrors the C struct of the same name.
#[repr(C)]
struct FSEventStreamContext {
    version: CFIndex,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

type FSEventStreamCallback = extern "C" fn(stream: FSEventStreamRef, info: *mut c_void, num_events: usize, event_paths: *mut c_void, event_flags: *const u32, event_ids: *const u64);

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithCString(alloc: CFTypeRef, c_str: *const c_char, encoding: u32) -> CFTypeRef;
    fn CFArrayCreate(alloc: CFTypeRef, values: *const CFTypeRef, num_values: CFIndex, callbacks: *const c_void) -> CFTypeRef;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, idx: CFIndex) -> CFTypeRef;
    fn CFStringCreateMutableCopy(alloc: CFTypeRef, max_length: CFIndex, string: CFTypeRef) -> CFTypeRef;
    fn CFStringNormalize(string: CFTypeRef, the_form: CFIndex);
    fn CFStringFold(string: CFTypeRef, the_flags: usize, the_locale: CFTypeRef);
    fn CFStringGetLength(string: CFTypeRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut c_char, buffer_size: CFIndex, encoding: u32) -> Boolean;
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn FSEventStreamCreate(
        allocator: CFTypeRef,
        callback: FSEventStreamCallback,
        context: *const FSEventStreamContext,
        paths_to_watch: CFTypeRef,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, q: dispatch_queue_t);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> Boolean;
    fn FSEventStreamFlushSync(stream: FSEventStreamRef);
    fn FSEventsGetCurrentEventId() -> u64;
    fn FSEventStreamStop(stream: FSEventStreamRef);
    fn FSEventStreamInvalidate(stream: FSEventStreamRef);
    fn FSEventStreamRelease(stream: FSEventStreamRef);
}

// libSystem (always linked).
extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> dispatch_queue_t;
    fn dispatch_release(object: *mut c_void);
    fn dispatch_sync_f(queue: dispatch_queue_t, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

pub(crate) const cfStringEncodingUTF8: u32 = 0x08000100;
const cfStringNormalizationFormC: CFIndex = 2; // kCFStringNormalizationFormC
// kFSEventStreamCreateFlagUseCFTypes (0x1) | kFSEventStreamCreateFlagFileEvents (0x10); Go hardcodes it in the
// assembly trampolines.
const fsEventStreamCreateFlags: u32 = 0x11;

// An owned CoreFoundation object reference (released on drop).
pub(crate) struct CFRef(CFTypeRef);

impl Drop for CFRef {
    fn drop(&mut self) {
        // SAFETY: `CFRef` is only constructed from a non-NULL object returned by a CF "Create"/"Copy" function,
        // which the caller owns exactly once.
        unsafe { CFRelease(self.0) }
    }
}

// Go `cfStringCreate(0, cstr, UTF8)`; None when CFStringCreateWithCString returns NULL (or `s` contains NUL).
pub(crate) fn cf_string_create(s: &str) -> Option<CFRef> {
    let cstr = CString::new(s).ok()?;
    // SAFETY: `cstr` is NUL-terminated and outlives the call; NULL allocator = default.
    let r = unsafe { CFStringCreateWithCString(std::ptr::null(), cstr.as_ptr(), cfStringEncodingUTF8) };
    (!r.is_null()).then_some(CFRef(r))
}

// Go `cfArrayCreate(0, values, n, 0)`: NULL callbacks, so the array does not retain its values; the caller keeps
// `values` alive while the array is used (Go's `defer cfRelease` order).
pub(crate) fn cf_array_create(values: &[CFRef]) -> Option<CFRef> {
    let ptrs: Vec<CFTypeRef> = values.iter().map(|v| v.0).collect();
    // SAFETY: `ptrs` holds `ptrs.len()` valid CF objects for the duration of the call.
    let r = unsafe { CFArrayCreate(std::ptr::null(), ptrs.as_ptr(), ptrs.len() as CFIndex, std::ptr::null()) };
    (!r.is_null()).then_some(CFRef(r))
}

fn cf_string_create_mutable_copy(src: CFTypeRef) -> Option<CFRef> {
    // SAFETY: `src` is a valid CFString (callers pass objects they own or borrow for the call).
    let r = unsafe { CFStringCreateMutableCopy(std::ptr::null(), 0, src) };
    (!r.is_null()).then_some(CFRef(r))
}

fn cf_string_normalize(mut_str: &CFRef, form: CFIndex) {
    // SAFETY: `mut_str` is a CFMutableString created by `cf_string_create_mutable_copy`.
    unsafe { CFStringNormalize(mut_str.0, form) }
}

pub(crate) const nativePathFolding: bool = true;

// fsevents_darwin_ffi.go:206
// foldNativePath is a comparison form, never a displayed or opened path.
// Case folding expands sharp s and ligatures without making diacritics,
// dotless i, circled letters, or character widths interchangeable.
pub(crate) fn fold_native_path(s: &str) -> String {
    if is_ascii(s) {
        return s.to_ascii_lowercase();
    }
    if s.contains('\0') {
        return String::new();
    }
    let Some(src) = cf_string_create(s) else {
        panic!("fswatch: cannot create CFString for path folding");
    };
    let Some(mut_) = cf_string_create_mutable_copy(src.0) else {
        panic!("fswatch: cannot copy CFString for path folding");
    };
    // Normalize before folding as well: a decomposed capital I with dot
    // must have the same comparison form as precomposed dotted capital I.
    cf_string_normalize(&mut_, cfStringNormalizationFormC);
    const cfCompareCaseInsensitive: usize = 1;
    // SAFETY: `mut_` is a CFMutableString; NULL locale = canonical folding.
    unsafe { CFStringFold(mut_.0, cfCompareCaseInsensitive, std::ptr::null()) };
    cf_string_normalize(&mut_, cfStringNormalizationFormC);
    let folded = cf_string_to_go(mut_.0);
    if folded.is_empty() {
        panic!("fswatch: cannot extract folded CFString");
    }
    folded
}

// fsevents_darwin_ffi.go:271
// isASCII reports whether every byte in s is below 0x80. Pure-ASCII paths
// are identical in every Unicode normalization form, so we can skip the
// CoreFoundation round-trip entirely, which is the overwhelming common case.
fn is_ascii(s: &str) -> bool {
    s.is_ascii()
}

// fsevents_darwin_ffi.go:284
// cfStringToNFC returns the CFString at src as a NFC-normalized Go string.
// If normalization fails, it falls back to the unnormalized UTF-8 contents.
// Returns "" only if both the normalized and unnormalized conversions fail
// (e.g. src is not a CFString, or allocation fails).
pub(crate) fn cf_string_to_nfc(src: CFTypeRef) -> String {
    if src.is_null() {
        return String::new();
    }
    let s = cf_string_normalized_to_go(src);
    if !s.is_empty() {
        return s;
    }
    cf_string_to_go(src)
}

// fsevents_darwin_ffi.go:296
// cfStringNormalizedToGo returns the CFString at src as a NFC-normalized Go
// string, or "" on any failure.
fn cf_string_normalized_to_go(src: CFTypeRef) -> String {
    let Some(mut_) = cf_string_create_mutable_copy(src) else {
        return String::new();
    };
    cf_string_normalize(&mut_, cfStringNormalizationFormC);
    cf_string_to_go(mut_.0)
}

// fsevents_darwin_ffi.go:309
// cfStringToGo extracts the UTF-8 contents of the CFString at src as a Go
// string, or "" on failure.
fn cf_string_to_go(src: CFTypeRef) -> String {
    // SAFETY: `src` is a valid CFString for the duration of these calls; `buf` has `buf_size` bytes.
    unsafe {
        let length = CFStringGetLength(src);
        let buf_size = CFStringGetMaximumSizeForEncoding(length, cfStringEncodingUTF8) + 1;
        let mut buf = vec![0u8; buf_size as usize];
        if CFStringGetCString(src, buf.as_mut_ptr() as *mut c_char, buf_size, cfStringEncodingUTF8) == 0 {
            return String::new();
        }
        // CFStringGetCString writes a NUL terminator; trim it.
        let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        buf.truncate(n);
        String::from_utf8(buf).unwrap_or_default()
    }
}

// fsevents_darwin_ffi.go:330
// normalizeNFC returns s in Unicode NFC (canonical composed) form. ASCII
// inputs are returned unchanged. Non-ASCII inputs go through CoreFoundation;
// if any step fails (e.g. invalid UTF-8 from a corrupt path), the original
// string is returned so the caller still sees *something* rather than nothing.
pub(crate) fn normalize_nfc(s: &str) -> String {
    if is_ascii(s) {
        return s.to_string();
    }
    let Some(src) = cf_string_create(s) else {
        return s.to_string();
    };
    let normalized = cf_string_to_nfc(src.0);
    if normalized.is_empty() {
        return s.to_string();
    }
    normalized
}

// Go `fsEventsGetCurrentEventID`.
pub(crate) fn fs_events_get_current_event_id() -> u64 {
    // SAFETY: no arguments, no preconditions.
    unsafe { FSEventsGetCurrentEventId() }
}

// The borrowed arguments of one FSEvents callback (Go's retained/copied `fsEventsCallbackPayload`; here the
// callback runs synchronously, so nothing is retained or freed).
pub(crate) struct fsEventsCallbackPayload {
    pub(crate) num_events: usize,
    paths: CFTypeRef,
    pub(crate) flags: *const u32,
    pub(crate) ids: *const u64,
}

impl fsEventsCallbackPayload {
    pub(crate) fn is_empty(&self) -> bool {
        self.paths.is_null() || self.flags.is_null() || self.ids.is_null()
    }

    pub(crate) fn flag(&self, i: usize) -> u32 {
        // SAFETY: FSEvents passes `num_events` flags; callers index below `num_events`.
        unsafe { *self.flags.add(i) }
    }

    pub(crate) fn id(&self, i: usize) -> u64 {
        // SAFETY: as `flag`.
        unsafe { *self.ids.add(i) }
    }

    // Go `cfStringToNFC(cfArrayGetValueAtIndex(paths, i))`.
    pub(crate) fn path_nfc(&self, i: usize) -> String {
        // SAFETY: with kFSEventStreamCreateFlagUseCFTypes `paths` is a CFArray of `num_events` CFStrings, valid for
        // the duration of the callback.
        let path_ref = unsafe { CFArrayGetValueAtIndex(self.paths, i as CFIndex) };
        cf_string_to_nfc(path_ref)
    }
}

// streamCallback is the per-stream state the C callback receives as FSEventStreamContext.info.
pub(crate) struct streamCallback {
    queue: dispatch_queue_t, // per-stream serial dispatch queue
    pub(crate) watches: Vec<fseventsWatchSnapshot>,
}

// SAFETY: the dispatch queue handle is a thread-safe libdispatch object; `watches` is immutable after creation.
unsafe impl Send for streamCallback {}
unsafe impl Sync for streamCallback {}

// fsevents_darwin_ffi.go:536
// newStreamCallback allocates a streamCallback with its own per-stream serial dispatch queue. The per-stream
// serial queue serializes this stream's callbacks and prevents cross-stream head-of-line blocking that a
// process-wide serial queue would cause.
pub(crate) fn new_stream_callback(watches: &[fseventsWatchSnapshot]) -> Option<Box<streamCallback>> {
    let label = b"typescript.fswatch.fsevents.stream\0";
    // SAFETY: `label` is NUL-terminated; NULL attr = serial queue.
    let queue = unsafe { dispatch_queue_create(label.as_ptr() as *const c_char, std::ptr::null()) };
    if queue.is_null() {
        return None;
    }
    Some(Box::new(streamCallback { queue, watches: watches.to_vec() }))
}

extern "C" fn dispatch_noop(_: *mut c_void) {}

impl streamCallback {
    // fsevents_darwin_ffi.go:565
    pub(crate) fn wait_dispatch_queue(&self) {
        if !self.queue.is_null() {
            // SAFETY: `queue` is a live queue owned by this callback; the work function does nothing.
            unsafe { dispatch_sync_f(self.queue, std::ptr::null_mut(), dispatch_noop) }
        }
    }

    // fsevents_darwin_ffi.go:572
    // close releases resources.
    pub(crate) fn close(&mut self) {
        if !self.queue.is_null() {
            // SAFETY: the queue was created by `new_stream_callback` and is released exactly once.
            unsafe { dispatch_release(self.queue) };
            self.queue = std::ptr::null_mut();
        }
    }
}

extern "C" fn fs_events_callback_c(_stream: FSEventStreamRef, info: *mut c_void, num_events: usize, event_paths: *mut c_void, event_flags: *const u32, event_ids: *const u64) {
    // SAFETY: `info` is the `streamCallback` passed to FSEventStreamCreate; it stays alive until
    // `teardown_stream` has waited for the stream's queue, so no callback outlives it.
    let cb = unsafe { &*(info as *const streamCallback) };
    let payload = fsEventsCallbackPayload { num_events, paths: event_paths as CFTypeRef, flags: event_flags, ids: event_ids };
    // A panic must not unwind into CoreServices.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fs_events_callback(cb, &payload)));
}

// An FSEventStream handle (Go keeps it as an atomic uintptr).
pub(crate) struct fsEventStream(FSEventStreamRef);

// SAFETY: FSEventStream functions may be called from any thread once the stream is scheduled on a dispatch queue.
unsafe impl Send for fsEventStream {}
unsafe impl Sync for fsEventStream {}

pub(crate) enum streamStartError {
    CFStringCreateNull,
    CFArrayCreateNull,
    StreamCreateNull,
    StreamStartFailed,
}

// The FFI part of fsevents_darwin.go `startStream`: create the CF path array and the stream, schedule it on the
// callback's queue, start and flush it. On failure the callback is closed (Go: `cb.close()`).
pub(crate) fn create_and_start_stream(paths: &[String], cb: &mut Box<streamCallback>) -> Result<fsEventStream, streamStartError> {
    let mut cf_strings = Vec::with_capacity(paths.len());
    for path in paths {
        match cf_string_create(path) {
            Some(s) => cf_strings.push(s),
            None => return Err(streamStartError::CFStringCreateNull),
        }
    }
    let Some(paths_to_watch) = cf_array_create(&cf_strings) else {
        return Err(streamStartError::CFArrayCreateNull);
    };

    let info: *mut streamCallback = &mut **cb;
    let ctx = FSEventStreamContext { version: 0, info: info as *mut c_void, retain: std::ptr::null(), release: std::ptr::null(), copy_description: std::ptr::null() };
    // kFSEventStreamEventIdSinceNow == ((FSEventStreamEventId)0xFFFFFFFFFFFFFFFFULL)
    const eventIDSinceNow: u64 = 0xFFFFFFFFFFFFFFFF;
    // SAFETY: `ctx` is copied by FSEventStreamCreate; `paths_to_watch` is a valid CFArray of CFStrings; `info`
    // points into the boxed callback, whose address is stable until the stream is torn down.
    let stream = unsafe { FSEventStreamCreate(std::ptr::null(), fs_events_callback_c, &ctx, paths_to_watch.0, eventIDSinceNow, 0.001, fsEventStreamCreateFlags) };
    drop(paths_to_watch);
    drop(cf_strings);
    if stream.is_null() {
        cb.close();
        return Err(streamStartError::StreamCreateNull);
    }

    // SAFETY: `stream` was just created; `cb.queue` is a live serial queue.
    unsafe {
        FSEventStreamSetDispatchQueue(stream, cb.queue);
        if FSEventStreamStart(stream) == 0 {
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            cb.close();
            return Err(streamStartError::StreamStartFailed);
        }
        FSEventStreamFlushSync(stream);
    }
    Ok(fsEventStream(stream))
}

// fsevents_darwin.go:350
// teardownStream performs the full FSEventStream cleanup. Stop and Invalidate
// prevent new callbacks, waitDispatchQueue waits for callbacks already queued
// on the stream's serial dispatch queue.
pub(crate) fn teardown_stream(stream: fsEventStream, cb: Option<&mut Box<streamCallback>>) {
    // SAFETY: `stream` is a started stream that is torn down exactly once (the caller swapped it out).
    unsafe {
        FSEventStreamStop(stream.0);
        match cb {
            Some(cb) => {
                FSEventStreamInvalidate(stream.0);
                cb.wait_dispatch_queue();
                cb.close();
            }
            None => FSEventStreamInvalidate(stream.0),
        }
        FSEventStreamRelease(stream.0);
    }
}

