//! tagma-ffi: C ABI over `tagma-core` (Phase 3, PLAN.md §8, task C2).
//!
//! Data-in/data-out over an opaque handle (ARCHITECTURE.md API constraint):
//! strings in, id arrays out (newline-joined), index state inside the core.
//! Findings (SPEC.md §13/§14, [`tagma_validate`] and [`tagma_check`]) come
//! out the same way: one JSON object per line. All strings are UTF-8. A thread-local holds the last error message,
//! readable via [`tagma_last_error`].
//!
//! # Handle safety
//!
//! An index is named by an opaque [`TagmaIndex`] integer, not by a pointer;
//! a standalone rule set ([`tagma_rules_from_tags`]) by a [`TagmaRules`]
//! integer, with the same guarantees.
//! Every handle is validated on entry, so a freed handle, a handle whose
//! slot has since been reissued, and a value this library never issued are
//! all *detected* and reported through [`tagma_last_error`] instead of
//! being undefined behaviour. See [`handle`] for the representation, the
//! concurrency guarantee, and how slots are reclaimed.
//!
//! # Panic safety
//!
//! A Rust panic must never reach an `extern "C"` boundary. Historically it
//! was undefined behaviour; since Rust 1.81 the compiler inserts an
//! abort-on-unwind shim, so today it instead kills the host process. Either
//! way it is unacceptable in a library: a caller has no way to recover, and
//! a panic reachable from caller-controlled input is a denial of service.
//!
//! Every entry point in this crate is therefore wrapped in
//! [`std::panic::catch_unwind`] (see [`guard`]): a panic that reaches the
//! ABI boundary is converted into that function's ordinary failure value
//! (`-1`, or `NULL` for the string-returning ones) with the panic message
//! recorded for [`tagma_last_error`]. No entry point unwinds, and none
//! aborts on a caller mistake.
//!
//! `catch_unwind` is the *backstop*, not the primary path: caller-controlled
//! invalidity (null pointers, non-UTF-8 bytes, interior NULs in a result,
//! malformed queries) is detected explicitly and returned as an error
//! without ever panicking. A caught panic always means a bug in tagma or in
//! a client extension.
//!
//! This holds only where panics unwind. Rust's `wasm32-unknown-unknown`
//! target uses `panic = "abort"`, where `catch_unwind` cannot intercept
//! anything; the corresponding guarantee for `crates/tagma-wasm` is
//! "never panic", not "catch panics". Building this crate under a
//! `panic = "abort"` profile likewise degrades the guarantee to an abort —
//! defined behaviour, but not a recoverable error.
#![deny(missing_docs)]

use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, UnwindSafe};

use tagma_core::token::split_unquoted_whitespace;
use tagma_core::{Change, Finding, Index, Rules, Tag};

pub mod handle;

pub use handle::{TagmaIndex, TagmaRules};

thread_local! {
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_last_error(e: String) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(e));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Reads the last error message recorded on this thread, cloning it out of
/// the `RefCell` before the borrow ends.
///
/// Cloning rather than formatting in place matters: `to_c_string` records an
/// error of its own on an interior NUL, so rendering the message while the
/// slot is still borrowed would be a `borrow_mut`-inside-`borrow` and panic
/// with `BorrowMutError`.
fn last_error() -> Option<String> {
    LAST_ERROR.with(|slot| slot.borrow().clone())
}

/// Renders a caught panic payload as a message. `catch_unwind` yields the
/// value passed to `panic!`, which is a `&str` or `String` for every panic
/// the standard library and `panic!` macro raise; anything else (a custom
/// `panic_any`) is reported without its content.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Runs `f` under [`catch_unwind`], returning `on_panic` and recording the
/// panic message for [`tagma_last_error`] if it unwinds.
///
/// This is the single mechanism keeping panics from crossing the C ABI. It
/// deliberately reuses the crate's existing thread-local error channel
/// rather than introducing a second one, so a caught panic is observed by
/// callers exactly like any other failure — check the return value, then
/// read [`tagma_last_error`].
fn guard<T>(what: &str, on_panic: T, f: impl FnOnce() -> T + UnwindSafe) -> T {
    match catch_unwind(f) {
        Ok(v) => v,
        Err(payload) => {
            // The panic unwound past whatever borrow it was holding, so the
            // thread-local is free to write again here.
            set_last_error(format!(
                "ffi: panic in {what}: {}",
                panic_message(&*payload)
            ));
            on_panic
        }
    }
}

/// Converts a `String` to a heap-allocated, caller-owned C string. Returns
/// `NULL` (and records an error) if `s` contains an interior NUL byte.
fn to_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => {
            set_last_error("ffi: result contains an interior NUL byte".to_string());
            std::ptr::null_mut()
        }
    }
}

/// Borrows `ptr` as a UTF-8 `&str`, recording an error and returning `None`
/// if `ptr` is null or not valid UTF-8.
///
/// # Safety
///
/// `ptr`, if non-null, must point to a valid NUL-terminated C string.
unsafe fn borrow_str<'a>(ptr: *const c_char, what: &str) -> Option<&'a str> {
    if ptr.is_null() {
        set_last_error(format!("ffi: {what} is null"));
        return None;
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Some(s),
        Err(_) => {
            set_last_error(format!("ffi: {what} is not valid UTF-8"));
            None
        }
    }
}

/// Runs `f` against the index named by `handle`, recording the handle
/// diagnostic and returning `on_error` if the handle is not a live one this
/// library issued.
fn with_handle<T>(handle: TagmaIndex, on_error: T, f: impl FnOnce(&mut Index) -> T) -> T {
    match handle::with_index(handle, f) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(e.message("index"));
            on_error
        }
    }
}

/// Creates a new, empty index and returns an opaque owning handle to it.
/// The caller must eventually pass the handle to [`tagma_index_free`].
/// Returns `0` if construction fails (see [`tagma_last_error`]); `0` is
/// never a valid handle. Never unwinds.
#[no_mangle]
pub extern "C" fn tagma_index_new() -> TagmaIndex {
    guard("tagma_index_new", 0, || {
        match handle::allocate(Index::new()) {
            Ok(h) => {
                clear_last_error();
                h
            }
            Err(e) => {
                set_last_error(e.message("index"));
                0
            }
        }
    })
}

/// Frees the index named by a handle previously returned by
/// [`tagma_index_new`]. Returns `0` on success and `-1` if the handle is
/// not a live tagma handle — already freed, never issued, or garbage — with
/// the reason in [`tagma_last_error`]. A `0` handle is a no-op returning
/// `0`, mirroring `free(NULL)`.
///
/// After a successful call the handle is dead: every later use of it,
/// including another free, is a defined error rather than undefined
/// behaviour, and it will never be reissued for a future index.
///
/// Never unwinds: a panic while dropping the index is caught and recorded
/// for [`tagma_last_error`].
#[no_mangle]
pub extern "C" fn tagma_index_free(handle: TagmaIndex) -> c_int {
    guard("tagma_index_free", -1, || {
        if handle == 0 {
            return 0;
        }
        match handle::release(handle) {
            Ok(()) => {
                clear_last_error();
                0
            }
            Err(e) => {
                set_last_error(e.message("index"));
                -1
            }
        }
    })
}

/// Parses and adds a `<id> <tag> <tag>...` line to the index (same line
/// format as `tagma-cli query`'s stdin). Returns `0` on success, `-1` on
/// error (see [`tagma_last_error`]).
///
/// # Safety
///
/// `line`, if non-null, must point to a valid NUL-terminated UTF-8 C
/// string. `handle` needs no precondition: an invalid one is a reported
/// error.
#[no_mangle]
pub unsafe extern "C" fn tagma_index_add(handle: TagmaIndex, line: *const c_char) -> c_int {
    guard("tagma_index_add", -1, || {
        let Some(line) = (unsafe { borrow_str(line, "line") }) else {
            return -1;
        };
        with_handle(handle, -1, |idx| match idx.add_line(line) {
            Ok(()) => {
                clear_last_error();
                0
            }
            Err(e) => {
                set_last_error(e);
                -1
            }
        })
    })
}

/// Compiles `q` (infix) and evaluates it against the index, returning a
/// newly allocated, newline-joined, sorted list of matching ids (empty
/// string for no matches). Returns `NULL` on error (see
/// [`tagma_last_error`]); free the result with [`tagma_str_free`].
///
/// # Safety
///
/// `q`, if non-null, must point to a valid NUL-terminated UTF-8 C string.
/// `handle` needs no precondition: an invalid one is a reported error.
#[no_mangle]
pub unsafe extern "C" fn tagma_query(handle: TagmaIndex, q: *const c_char) -> *mut c_char {
    guard("tagma_query", std::ptr::null_mut(), || {
        let Some(q) = (unsafe { borrow_str(q, "query") }) else {
            return std::ptr::null_mut();
        };
        with_handle(handle, std::ptr::null_mut(), |idx| match idx.query(q) {
            Ok(mut ids) => {
                clear_last_error();
                ids.sort();
                to_c_string(ids.join("\n"))
            }
            Err(e) => {
                set_last_error(e);
                std::ptr::null_mut()
            }
        })
    })
}

/// Evaluates an already-compiled postfix query against the index; same
/// return convention as [`tagma_query`].
///
/// # Safety
///
/// `q`, if non-null, must point to a valid NUL-terminated UTF-8 C string.
/// `handle` needs no precondition: an invalid one is a reported error.
#[no_mangle]
pub unsafe extern "C" fn tagma_query_postfix(handle: TagmaIndex, q: *const c_char) -> *mut c_char {
    guard("tagma_query_postfix", std::ptr::null_mut(), || {
        let Some(q) = (unsafe { borrow_str(q, "query") }) else {
            return std::ptr::null_mut();
        };
        with_handle(handle, std::ptr::null_mut(), |idx| {
            match idx.query_postfix(q) {
                Ok(mut ids) => {
                    clear_last_error();
                    ids.sort();
                    to_c_string(ids.join("\n"))
                }
                Err(e) => {
                    set_last_error(e);
                    std::ptr::null_mut()
                }
            }
        })
    })
}

/// Compiles an infix query `q` to its canonical postfix form. Returns
/// `NULL` on error (see [`tagma_last_error`]); free the result with
/// [`tagma_str_free`].
///
/// # Safety
///
/// `q`, if non-null, must point to a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn tagma_compile(q: *const c_char) -> *mut c_char {
    guard("tagma_compile", std::ptr::null_mut(), || {
        let Some(q) = (unsafe { borrow_str(q, "query") }) else {
            return std::ptr::null_mut();
        };
        match tagma_core::infix::compile(q) {
            Ok(postfix) => {
                clear_last_error();
                to_c_string(postfix)
            }
            Err(e) => {
                set_last_error(e);
                std::ptr::null_mut()
            }
        }
    })
}

/// Parses a whitespace-separated tag list, split on unquoted whitespace as
/// an [`tagma_index_add`] line is (SPEC.md §2); `""` is the empty list.
fn parse_tag_list(list: &str) -> Result<Vec<Tag>, String> {
    split_unquoted_whitespace(list)?
        .into_iter()
        .map(Tag::parse)
        .collect()
}

/// Renders findings one JSON object per line (empty string for none):
/// `{"code":…,"tag":…,"args":[[name,value],…],"origin":…}`, keys in
/// SPEC.md §13's order, `tag` in its canonical spelling (SPEC.md §2) —
/// the same lines `tagma validate`/`tagma check` print.
fn findings_jsonl(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|f| {
            let args: Vec<String> = f
                .args
                .iter()
                .map(|(name, value)| format!("[{},{}]", json_str(name), json_str(value)))
                .collect();
            format!(
                "{{\"code\":{},\"tag\":{},\"args\":[{}],\"origin\":{}}}",
                json_str(f.code.as_str()),
                json_str(&f.tag.to_string()),
                args.join(","),
                json_str(f.origin.as_str()),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `s` as a JSON string literal (RFC 8259 §7): `"` and `\` escaped, every
/// control character as `\u00XX` — so a finding never spans two lines.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Validates a change to item `id` (SPEC.md §13): the findings for the tag
/// set the item would carry after removing every tag in `remove` and then
/// adding every tag in `add` (both whitespace-separated tag lists, `""` for
/// none), judged against the index's `tagma.*` config as it stands. Pure:
/// the index is not modified. An unknown `id` starts from no tags.
///
/// Returns a newly allocated string of findings, one JSON object per line
/// (`{"code","tag","args":[[name,value],...],"origin"}`, `origin` being
/// `"introduced"` or `"existing"`), sorted as SPEC.md §13 orders them; the
/// empty string means none. Returns `NULL` on error — an invalid tag in
/// `add` or `remove`, a null or non-UTF-8 argument, an invalid handle (see
/// [`tagma_last_error`]). Free the result with [`tagma_str_free`].
///
/// # Safety
///
/// `id`, `add` and `remove`, if non-null, must each point to a valid
/// NUL-terminated C string. `handle` needs no precondition: an invalid one
/// is a reported error.
#[no_mangle]
pub unsafe extern "C" fn tagma_validate(
    handle: TagmaIndex,
    id: *const c_char,
    add: *const c_char,
    remove: *const c_char,
) -> *mut c_char {
    guard("tagma_validate", std::ptr::null_mut(), || {
        let (Some(id), Some(add), Some(remove)) = (
            unsafe { borrow_str(id, "id") },
            unsafe { borrow_str(add, "add") },
            unsafe { borrow_str(remove, "remove") },
        ) else {
            return std::ptr::null_mut();
        };
        let Some(change) = parse_change(add, remove) else {
            return std::ptr::null_mut();
        };
        with_handle(handle, std::ptr::null_mut(), |idx| {
            clear_last_error();
            to_c_string(findings_jsonl(&idx.validate(id, &change)))
        })
    })
}

/// Checks the index's own `tagma.*` config (SPEC.md §14). Returns a newly
/// allocated string of findings in [`tagma_validate`]'s form (every
/// `origin` `"existing"`; the empty string for none), or `NULL` on an
/// invalid handle (see [`tagma_last_error`]). Free the result with
/// [`tagma_str_free`]. Never unwinds.
#[no_mangle]
pub extern "C" fn tagma_check(handle: TagmaIndex) -> *mut c_char {
    guard("tagma_check", std::ptr::null_mut(), || {
        with_handle(handle, std::ptr::null_mut(), |idx| {
            clear_last_error();
            to_c_string(findings_jsonl(&idx.check()))
        })
    })
}

/// Parses a validate change from its two tag lists, recording an error
/// naming the failing list.
fn parse_change(add: &str, remove: &str) -> Option<Change> {
    match (parse_tag_list(add), parse_tag_list(remove)) {
        (Ok(add), Ok(remove)) => Some(Change { add, remove }),
        (Err(e), _) => {
            set_last_error(format!("add: {e}"));
            None
        }
        (_, Err(e)) => {
            set_last_error(format!("remove: {e}"));
            None
        }
    }
}

/// Runs `f` against the rule set named by `handle`, recording the handle
/// diagnostic and returning `on_error` if the handle is not a live rules
/// handle this library issued.
fn with_rules<T>(handle: TagmaRules, on_error: T, f: impl FnOnce(&mut Rules) -> T) -> T {
    match handle::RULES.with(handle, f) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(e.message("rules"));
            on_error
        }
    }
}

/// Derives a standalone rule set (SPEC.md §10–§12, with §8 arity and §9
/// type) from the `tagma.*` config tags among `tags`, a whitespace-separated
/// tag list split as a [`tagma_index_add`] line is (every other tag is
/// ignored; `""` is no config) — for a caller that holds its config itself
/// rather than in an index. Returns an opaque owning handle, to be freed
/// with [`tagma_rules_free`], or `0` on error (an invalid tag, a null or
/// non-UTF-8 argument; see [`tagma_last_error`]). `0` is never a valid
/// handle. No comparators are registered: the C ABI takes no callbacks.
///
/// # Safety
///
/// `tags`, if non-null, must point to a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tagma_rules_from_tags(tags: *const c_char) -> TagmaRules {
    guard("tagma_rules_from_tags", 0, || {
        let Some(tags) = (unsafe { borrow_str(tags, "tags") }) else {
            return 0;
        };
        let tags = match parse_tag_list(tags) {
            Ok(tags) => tags,
            Err(e) => {
                set_last_error(e);
                return 0;
            }
        };
        match handle::RULES.allocate(Rules::from_tags(&tags)) {
            Ok(h) => {
                clear_last_error();
                h
            }
            Err(e) => {
                set_last_error(e.message("rules"));
                0
            }
        }
    })
}

/// Frees the rule set named by a handle from [`tagma_rules_from_tags`],
/// exactly as [`tagma_index_free`] frees an index: `0` on success, `-1` if
/// the handle is not a live rules handle (an index handle included), `0`
/// handle a no-op returning `0`. Never unwinds.
#[no_mangle]
pub extern "C" fn tagma_rules_free(handle: TagmaRules) -> c_int {
    guard("tagma_rules_free", -1, || {
        if handle == 0 {
            return 0;
        }
        match handle::RULES.release(handle) {
            Ok(()) => {
                clear_last_error();
                0
            }
            Err(e) => {
                set_last_error(e.message("rules"));
                -1
            }
        }
    })
}

/// Validates a change against a standalone rule set (SPEC.md §13): the
/// findings for the tag set `current` (a whitespace-separated tag list,
/// the item's tags as the caller holds them) would become after removing
/// every tag in `remove` and then adding every tag in `add`. Same lists,
/// result and errors as [`tagma_validate`]; an invalid tag in `current`
/// is reported with the prefix `current: `. Free the result with
/// [`tagma_str_free`].
///
/// # Safety
///
/// `current`, `add` and `remove`, if non-null, must each point to a valid
/// NUL-terminated C string. `handle` needs no precondition.
#[no_mangle]
pub unsafe extern "C" fn tagma_rules_validate(
    handle: TagmaRules,
    current: *const c_char,
    add: *const c_char,
    remove: *const c_char,
) -> *mut c_char {
    guard("tagma_rules_validate", std::ptr::null_mut(), || {
        let (Some(current), Some(add), Some(remove)) = (
            unsafe { borrow_str(current, "current") },
            unsafe { borrow_str(add, "add") },
            unsafe { borrow_str(remove, "remove") },
        ) else {
            return std::ptr::null_mut();
        };
        let current = match parse_tag_list(current) {
            Ok(tags) => tags,
            Err(e) => {
                set_last_error(format!("current: {e}"));
                return std::ptr::null_mut();
            }
        };
        let Some(change) = parse_change(add, remove) else {
            return std::ptr::null_mut();
        };
        with_rules(handle, std::ptr::null_mut(), |rules| {
            clear_last_error();
            to_c_string(findings_jsonl(&rules.validate(&current, &change)))
        })
    })
}

/// Checks a standalone rule set's config (SPEC.md §14); same result and
/// errors as [`tagma_check`]. Free the result with [`tagma_str_free`].
/// Never unwinds.
#[no_mangle]
pub extern "C" fn tagma_rules_check(handle: TagmaRules) -> *mut c_char {
    guard("tagma_rules_check", std::ptr::null_mut(), || {
        with_rules(handle, std::ptr::null_mut(), |rules| {
            clear_last_error();
            to_c_string(findings_jsonl(&rules.check()))
        })
    })
}

/// Returns a newly allocated copy of the last error message recorded on
/// this thread, or `NULL` if there is none. Free the result with
/// [`tagma_str_free`]. Never unwinds.
///
/// A caught panic from any other entry point is reported through this same
/// channel, prefixed `ffi: panic in <function>:`.
#[no_mangle]
pub extern "C" fn tagma_last_error() -> *mut c_char {
    guard("tagma_last_error", std::ptr::null_mut(), || {
        match last_error() {
            // Rendered outside the `RefCell` borrow: `to_c_string` records
            // an error of its own on failure, which would otherwise be a
            // `borrow_mut` while this borrow is still live.
            Some(e) => to_c_string(e),
            None => std::ptr::null_mut(),
        }
    })
}

/// Frees a string previously returned by [`tagma_query`],
/// [`tagma_query_postfix`], [`tagma_compile`], [`tagma_validate`],
/// [`tagma_check`], [`tagma_rules_validate`], [`tagma_rules_check`], or
/// [`tagma_last_error`]. A null pointer is a no-op.
///
/// # Safety
///
/// `s`, if non-null, must be a still-live pointer previously returned by
/// one of this crate's string-returning functions, not already freed, and
/// not used again after this call.
///
/// Never unwinds.
#[no_mangle]
pub unsafe extern "C" fn tagma_str_free(s: *mut c_char) {
    guard("tagma_str_free", (), || {
        if s.is_null() {
            return;
        }
        drop(unsafe { CString::from_raw(s) });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_cstring(s: &str) -> CString {
        CString::new(s).expect("test literal must not contain an interior NUL")
    }

    /// A C string whose bytes are valid for the ABI (NUL-terminated, no
    /// interior NUL) but are *not* valid UTF-8 — the caller-controlled input
    /// that must produce a defined error rather than a panic.
    fn invalid_utf8_cstring() -> CString {
        CString::new(vec![b'a', 0xff, 0xfe, b'b']).expect("no interior NUL")
    }

    unsafe fn from_c(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null(), "expected non-null string");
        let s = unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .expect("tagma always returns UTF-8")
            .to_string();
        unsafe { tagma_str_free(ptr) };
        s
    }

    /// Drains and returns the thread-local error, asserting one was set.
    unsafe fn take_error() -> String {
        let err = tagma_last_error();
        assert!(!err.is_null(), "expected an error to have been recorded");
        let msg = unsafe { from_c(err) };
        assert!(!msg.is_empty(), "error message must not be empty");
        clear_last_error();
        msg
    }

    #[test]
    fn round_trip_add_and_query() {
        unsafe {
            let handle = tagma_index_new();
            let lines = [
                "a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done",
                "b range=tbd lang=en prio:urgent due=2026-08-01",
                "c urgent=false score=-3 note",
            ];
            for line in lines {
                let cline = to_cstring(line);
                assert_eq!(tagma_index_add(handle, cline.as_ptr()), 0);
            }

            let q = to_cstring("urgent and not status=done");
            let result = tagma_query(handle, q.as_ptr());
            assert_eq!(from_c(result), "c");

            tagma_index_free(handle);
        }
    }

    #[test]
    fn query_no_matches_is_empty_string() {
        unsafe {
            let handle = tagma_index_new();
            let cline = to_cstring("a urgent");
            assert_eq!(tagma_index_add(handle, cline.as_ptr()), 0);

            let q = to_cstring("range>5");
            let result = tagma_query(handle, q.as_ptr());
            assert_eq!(from_c(result), "");

            tagma_index_free(handle);
        }
    }

    #[test]
    fn query_postfix_round_trip() {
        unsafe {
            let handle = tagma_index_new();
            let cline = to_cstring("a urgent status=done");
            assert_eq!(tagma_index_add(handle, cline.as_ptr()), 0);

            let q = to_cstring("urgent/status=done/not/and");
            let result = tagma_query_postfix(handle, q.as_ptr());
            assert_eq!(from_c(result), "");

            tagma_index_free(handle);
        }
    }

    /// Infix is a gateway (SPEC.md §5): `tagma_query(q)` returns exactly
    /// `tagma_query_postfix(tagma_compile(q))`.
    #[test]
    fn infix_query_is_query_postfix_of_compile() {
        let table = [
            "urgent",
            "urgent and range>4",
            "lang=en or note",
            "not status=done",
            "(urgent or note) and not lang=fr",
            "*:urgent",
            "lang=en lang=fr",
            "urgent AND NOT note",
            "nothing",
        ];
        unsafe {
            let handle = tagma_index_new();
            for line in [
                "a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done",
                "b range=tbd lang=en prio:urgent due=2026-08-01",
                "c urgent=false score=-3 note",
            ] {
                let cline = to_cstring(line);
                assert_eq!(tagma_index_add(handle, cline.as_ptr()), 0);
            }
            let mut distinct = std::collections::BTreeSet::new();
            for q in table {
                let cq = to_cstring(q);
                let postfix = to_cstring(&from_c(tagma_compile(cq.as_ptr())));
                let via_postfix = from_c(tagma_query_postfix(handle, postfix.as_ptr()));
                let via_infix = from_c(tagma_query(handle, cq.as_ptr()));
                assert_eq!(via_infix, via_postfix, "query({q:?})");
                distinct.insert(via_infix);
            }
            assert!(
                distinct.len() > 3,
                "table must discriminate between results"
            );
            tagma_index_free(handle);
        }
    }

    #[test]
    fn compile_round_trip() {
        unsafe {
            let q = to_cstring("a or b and c");
            let result = tagma_compile(q.as_ptr());
            assert_eq!(from_c(result), "a/b/c/and/or");
        }
    }

    #[test]
    fn compile_error_sets_last_error() {
        unsafe {
            let q = to_cstring("a and");
            let result = tagma_compile(q.as_ptr());
            assert!(result.is_null());

            let err = tagma_last_error();
            assert!(!err.is_null());
            let msg = from_c(err);
            assert!(!msg.is_empty());
        }
    }

    #[test]
    fn add_error_sets_last_error_and_returns_minus_one() {
        unsafe {
            let handle = tagma_index_new();
            let cline = to_cstring("a =5");
            assert_eq!(tagma_index_add(handle, cline.as_ptr()), -1);

            let err = tagma_last_error();
            assert!(!err.is_null());
            from_c(err);

            tagma_index_free(handle);
        }
    }

    #[test]
    fn null_pointer_inputs_fail_safely() {
        unsafe {
            let handle = tagma_index_new();

            assert_eq!(tagma_index_add(handle, std::ptr::null()), -1);

            assert!(tagma_query(handle, std::ptr::null()).is_null());
            assert!(tagma_query_postfix(handle, std::ptr::null()).is_null());
            assert!(tagma_compile(std::ptr::null()).is_null());

            let q = to_cstring("urgent");
            assert!(tagma_query(0, q.as_ptr()).is_null());
            assert_eq!(tagma_index_add(0, q.as_ptr()), -1);

            assert_eq!(tagma_index_free(handle), 0);
            assert_eq!(tagma_index_free(0), 0, "a zero handle frees as a no-op");
            tagma_str_free(std::ptr::null_mut());
        }
    }

    // --- panic safety (task tasty-snub) ------------------------------------

    /// Non-UTF-8 bytes are caller-controlled input, so they must take the
    /// explicit error path — not merely be netted by `catch_unwind`. The
    /// recorded message therefore must be the UTF-8 diagnostic, never a
    /// panic report.
    #[test]
    fn invalid_utf8_inputs_return_defined_errors_without_panicking() {
        unsafe {
            let handle = tagma_index_new();
            assert_ne!(handle, 0);
            let bad = invalid_utf8_cstring();

            assert_eq!(tagma_index_add(handle, bad.as_ptr()), -1);
            assert!(take_error().contains("not valid UTF-8"));

            assert!(tagma_query(handle, bad.as_ptr()).is_null());
            assert!(take_error().contains("not valid UTF-8"));

            assert!(tagma_query_postfix(handle, bad.as_ptr()).is_null());
            assert!(take_error().contains("not valid UTF-8"));

            assert!(tagma_compile(bad.as_ptr()).is_null());
            assert!(take_error().contains("not valid UTF-8"));

            tagma_index_free(handle);
        }
    }

    /// An interior NUL cannot arrive *through* the ABI (a C string ends at
    /// the first NUL), so the only interior-NUL hazard is on the way out:
    /// `to_c_string` must report it rather than unwrap a `CString::new`
    /// failure.
    #[test]
    fn interior_nul_result_returns_null_and_records_an_error() {
        unsafe {
            clear_last_error();
            let ptr = to_c_string("has\0a nul".to_string());
            assert!(ptr.is_null());
            assert!(take_error().contains("interior NUL"));
        }
    }

    /// Regression: `tagma_last_error` used to render the message *inside*
    /// the `RefCell` borrow, so a stored message containing an interior NUL
    /// made `to_c_string` call `borrow_mut` under a live `borrow` and panic
    /// with `BorrowMutError` — a panic raised by the very function callers
    /// use to diagnose failures.
    #[test]
    fn last_error_with_interior_nul_does_not_double_borrow() {
        unsafe {
            set_last_error("boom\0tail".to_string());
            let ptr = tagma_last_error();
            assert!(ptr.is_null(), "unrenderable message reports as NULL");
            // The slot is writable, i.e. no borrow leaked and nothing panicked.
            assert!(take_error().contains("interior NUL"));
        }
    }

    /// The backstop itself: a panic inside the guarded body becomes the
    /// function's ordinary failure value plus a recorded message, never an
    /// unwind across the ABI.
    #[test]
    fn guard_converts_a_panic_into_the_error_channel() {
        unsafe {
            clear_last_error();
            let v: c_int = guard("unit_under_test", -1, || panic!("deliberate boom"));
            assert_eq!(v, -1);
            let msg = take_error();
            assert!(msg.contains("panic in unit_under_test"), "got {msg}");
            assert!(msg.contains("deliberate boom"), "got {msg}");
        }
    }

    #[test]
    fn guard_reports_a_non_string_panic_payload() {
        unsafe {
            clear_last_error();
            let p: *mut c_char = guard("payload_test", std::ptr::null_mut(), || {
                std::panic::panic_any(42u8)
            });
            assert!(p.is_null());
            assert!(take_error().contains("unknown panic payload"));
        }
    }

    /// An `extern "C"` function shaped exactly like a real entry point whose
    /// body panics. Production entry points have no reachable panic today
    /// (that is the point of the explicit error paths above), so this stands
    /// in for a future one — notably a client-registered `tagma.type`
    /// comparator (SPEC.md §9) that violates its MUST-NOT-panic contract.
    extern "C" fn panicking_entry_point() -> c_int {
        guard("panicking_entry_point", -1, || {
            let empty: Vec<c_int> = Vec::new();
            empty[7]
        })
    }

    #[test]
    fn a_panicking_entry_point_returns_an_error_instead_of_unwinding() {
        unsafe {
            clear_last_error();
            assert_eq!(panicking_entry_point(), -1);
            assert!(take_error().contains("panic in panicking_entry_point"));
        }
    }

    /// Every entry point must survive a null handle *and* a null string in
    /// the same call, including the free functions.
    #[test]
    fn null_handle_and_null_string_together_are_defined() {
        unsafe {
            assert_eq!(tagma_index_add(0, std::ptr::null()), -1);
            assert!(tagma_query(0, std::ptr::null()).is_null());
            assert!(tagma_query_postfix(0, std::ptr::null()).is_null());
            assert_eq!(tagma_index_free(0), 0);
            tagma_str_free(std::ptr::null_mut());
        }
    }

    // --- handle safety (task kooky-snub) -----------------------------------

    /// Freeing twice is a reported error, not a double free.
    #[test]
    fn double_free_returns_a_defined_error() {
        unsafe {
            let handle = tagma_index_new();
            assert_eq!(tagma_index_free(handle), 0);
            clear_last_error();
            assert_eq!(tagma_index_free(handle), -1);
            assert!(take_error().contains("freed index"));
        }
    }

    /// Using a handle after freeing it is a reported error on every entry
    /// point that takes one.
    #[test]
    fn use_after_free_returns_a_defined_error() {
        unsafe {
            let handle = tagma_index_new();
            assert_eq!(tagma_index_free(handle), 0);

            let line = to_cstring("a urgent");
            assert_eq!(tagma_index_add(handle, line.as_ptr()), -1);
            assert!(take_error().contains("freed index"));

            let q = to_cstring("urgent");
            assert!(tagma_query(handle, q.as_ptr()).is_null());
            assert!(take_error().contains("freed index"));

            assert!(tagma_query_postfix(handle, q.as_ptr()).is_null());
            assert!(take_error().contains("freed index"));
        }
    }

    /// Values this library never issued are rejected, including plausible
    /// small integers and a real heap address — the shape of the old ABI's
    /// `void *` handles.
    #[test]
    fn a_foreign_handle_is_rejected() {
        unsafe {
            let boxed = Box::new(0u64);
            let as_pointer = (&*boxed as *const u64) as TagmaIndex;
            for garbage in [1, 42, 0xDEAD_BEEF, u64::MAX, as_pointer] {
                clear_last_error();
                assert_eq!(tagma_index_free(garbage), -1, "handle {garbage:#x}");
                let msg = take_error();
                assert!(msg.contains("not issued by tagma"), "got {msg}");
            }
        }
    }

    /// The generation case: after a free the slot is reused, and the stale
    /// handle must not reach the index that took its place.
    #[test]
    fn a_stale_handle_is_not_confused_with_a_reused_slot() {
        unsafe {
            let first = tagma_index_new();
            assert_eq!(tagma_index_free(first), 0);
            let second = tagma_index_new();
            assert_ne!(first, second, "a reused slot must issue a fresh handle");

            let line = to_cstring("a urgent");
            assert_eq!(tagma_index_add(second, line.as_ptr()), 0);

            assert_eq!(tagma_index_add(first, line.as_ptr()), -1);
            assert!(take_error().contains("freed index"));
            assert_eq!(tagma_index_free(first), -1);
            assert!(take_error().contains("freed index"));

            // The live index is untouched by the stale handle's traffic.
            let q = to_cstring("urgent");
            assert_eq!(from_c(tagma_query(second, q.as_ptr())), "a");
            assert_eq!(tagma_index_free(second), 0);
        }
    }

    /// One handle, many threads: the per-index lock serializes the writes
    /// and every call sees a valid handle.
    #[test]
    fn one_handle_is_safe_to_use_from_several_threads() {
        let handle = tagma_index_new();
        std::thread::scope(|s| {
            for t in 0..8 {
                s.spawn(move || {
                    for i in 0..32 {
                        let line = to_cstring(&format!("id{t}_{i} urgent"));
                        assert_eq!(unsafe { tagma_index_add(handle, line.as_ptr()) }, 0);
                    }
                });
            }
        });
        unsafe {
            let q = to_cstring("urgent");
            let ids = from_c(tagma_query(handle, q.as_ptr()));
            assert_eq!(ids.lines().count(), 8 * 32);
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    // --- validate and check (SPEC.md §13/§14) ------------------------------

    /// An index over `lines`, for the rules tests.
    unsafe fn rules_index(lines: &[&str]) -> TagmaIndex {
        let handle = tagma_index_new();
        for line in lines {
            let cline = to_cstring(line);
            assert_eq!(unsafe { tagma_index_add(handle, cline.as_ptr()) }, 0);
        }
        handle
    }

    unsafe fn validate(handle: TagmaIndex, id: &str, add: &str, remove: &str) -> *mut c_char {
        let (id, add, remove) = (to_cstring(id), to_cstring(add), to_cstring(remove));
        unsafe { tagma_validate(handle, id.as_ptr(), add.as_ptr(), remove.as_ptr()) }
    }

    #[test]
    fn validate_returns_one_json_finding_per_line_with_its_origin() {
        unsafe {
            let handle = rules_index(&["cfg tagma.enum:k=a tagma.enum:j=a", "item k=z j=bad"]);
            assert_eq!(
                from_c(validate(handle, "item", "k=y", "k=z")),
                concat!(
                    r#"{"code":"not-in-enum","tag":"j=bad","args":[["allowed","a"]],"origin":"existing"}"#,
                    "\n",
                    r#"{"code":"not-in-enum","tag":"k=y","args":[["allowed","a"]],"origin":"introduced"}"#,
                )
            );
            assert!(tagma_last_error().is_null(), "success clears the error");
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    #[test]
    fn validate_takes_whitespace_separated_tag_lists_and_is_pure() {
        unsafe {
            let handle = rules_index(&["cfg tagma.enum:k=a tagma.enum:j=a", "item k=z j=bad"]);
            assert_eq!(
                from_c(validate(handle, "item", r#"k=a j="a""#, "k=z j=bad")),
                ""
            );
            // Nothing was written: the old violations still hold.
            let q = to_cstring("k=z and j=bad");
            assert_eq!(from_c(tagma_query(handle, q.as_ptr())), "item");
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    #[test]
    fn validate_a_finding_with_no_args_has_an_empty_args_array() {
        unsafe {
            let handle = rules_index(&[r#"cfg tagma.permit:"*:*"=false"#]);
            assert_eq!(
                from_c(validate(handle, "x", "lang=en", "")),
                r#"{"code":"not-permitted","tag":"lang=en","args":[],"origin":"introduced"}"#
            );
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    #[test]
    fn validate_an_invalid_tag_names_the_list_it_was_in() {
        unsafe {
            let handle = rules_index(&[]);
            assert!(validate(handle, "x", "=5", "").is_null());
            assert!(take_error().starts_with("add: "));
            assert!(validate(handle, "x", "", "=5").is_null());
            assert!(take_error().starts_with("remove: "));
            assert!(validate(handle, "x", r#"k="open"#, "").is_null());
            assert!(take_error().starts_with("add: "));
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    #[test]
    fn validate_and_check_reject_null_and_non_utf8_arguments() {
        unsafe {
            let handle = rules_index(&[]);
            let ok = to_cstring("");
            let bad = invalid_utf8_cstring();
            let null = std::ptr::null();
            for (id, add, remove, what) in [
                (null, ok.as_ptr(), ok.as_ptr(), "id is null"),
                (ok.as_ptr(), null, ok.as_ptr(), "add is null"),
                (ok.as_ptr(), ok.as_ptr(), null, "remove is null"),
                (
                    bad.as_ptr(),
                    ok.as_ptr(),
                    ok.as_ptr(),
                    "id is not valid UTF-8",
                ),
                (
                    ok.as_ptr(),
                    bad.as_ptr(),
                    ok.as_ptr(),
                    "add is not valid UTF-8",
                ),
                (
                    ok.as_ptr(),
                    ok.as_ptr(),
                    bad.as_ptr(),
                    "remove is not valid UTF-8",
                ),
            ] {
                assert!(tagma_validate(handle, id, add, remove).is_null());
                let msg = take_error();
                assert!(msg.contains(what), "{what}: got {msg}");
            }
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    #[test]
    fn validate_and_check_reject_dead_and_foreign_handles() {
        unsafe {
            let handle = tagma_index_new();
            assert_eq!(tagma_index_free(handle), 0);
            assert!(validate(handle, "x", "", "").is_null());
            assert!(take_error().contains("freed index"));
            assert!(tagma_check(handle).is_null());
            assert!(take_error().contains("freed index"));
            assert!(tagma_check(42).is_null());
            assert!(take_error().contains("not issued by tagma"));
            assert!(tagma_check(0).is_null());
            assert!(take_error().contains("handle is null"));
        }
    }

    #[test]
    fn check_reports_the_config_one_json_finding_per_line() {
        unsafe {
            let handle = rules_index(&[
                r#"cfg tagma.permit:"*:secret"=false tagma.range:k=1..3 tagma.range:k=5..9"#,
            ]);
            assert_eq!(
                from_c(tagma_check(handle)),
                concat!(
                    r#"{"code":"inert-false","tag":"tagma.permit:\"*:secret\"=false","args":[["facet","permit"]],"origin":"existing"}"#,
                    "\n",
                    r#"{"code":"range-empty","tag":"tagma.range:k=1..3","args":[["range","5..9"]],"origin":"existing"}"#,
                    "\n",
                    r#"{"code":"range-empty","tag":"tagma.range:k=5..9","args":[["range","1..3"]],"origin":"existing"}"#,
                )
            );
            assert!(tagma_last_error().is_null(), "success clears the error");
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    #[test]
    fn check_of_an_unconfigured_index_is_the_empty_string() {
        unsafe {
            let handle = rules_index(&["a k=1"]);
            assert_eq!(from_c(tagma_check(handle)), "");
            assert_eq!(tagma_index_free(handle), 0);
        }
    }

    // --- standalone rules (Rules::from_tags) ---------------------------------

    unsafe fn rules_from(tags: &str) -> TagmaRules {
        let tags = to_cstring(tags);
        let h = unsafe { tagma_rules_from_tags(tags.as_ptr()) };
        assert_ne!(h, 0, "from_tags failed");
        h
    }

    unsafe fn rules_validate(h: TagmaRules, current: &str, add: &str, remove: &str) -> *mut c_char {
        let (c, a, r) = (to_cstring(current), to_cstring(add), to_cstring(remove));
        unsafe { tagma_rules_validate(h, c.as_ptr(), a.as_ptr(), r.as_ptr()) }
    }

    #[test]
    fn rules_validate_matches_the_index_it_mirrors() {
        unsafe {
            // Non-config tags among `tags` are ignored.
            let rules = rules_from("tagma.enum:k=a tagma.enum:j=a k=z unrelated");
            assert!(tagma_last_error().is_null(), "success clears the error");
            let idx = rules_index(&["cfg tagma.enum:k=a tagma.enum:j=a", "item k=z j=bad"]);
            let via_rules = from_c(rules_validate(rules, "k=z j=bad", "k=y", "k=z"));
            assert!(tagma_last_error().is_null(), "success clears the error");
            let via_index = from_c(validate(idx, "item", "k=y", "k=z"));
            assert_eq!(via_rules, via_index);
            assert!(via_rules.contains(r#""origin":"introduced""#));
            assert_eq!(from_c(rules_validate(rules, "", "k=a", "")), "");
            assert_eq!(tagma_rules_free(rules), 0);
            assert_eq!(tagma_index_free(idx), 0);
        }
    }

    #[test]
    fn rules_check_matches_the_index_it_mirrors() {
        unsafe {
            let config = r#"tagma.permit:"*:secret"=false tagma.arity:k=scalar tagma.arity:k=set"#;
            let rules = rules_from(config);
            let idx = rules_index(&[&format!("cfg {config}")]);
            let via_rules = from_c(tagma_rules_check(rules));
            assert!(tagma_last_error().is_null(), "success clears the error");
            assert_eq!(via_rules, from_c(tagma_check(idx)));
            assert_eq!(via_rules.lines().count(), 2);
            let empty = rules_from("");
            assert_eq!(from_c(tagma_rules_check(empty)), "");
            assert_eq!(tagma_rules_free(rules), 0);
            assert_eq!(tagma_rules_free(empty), 0);
            assert_eq!(tagma_index_free(idx), 0);
        }
    }

    #[test]
    fn keys_per_item_findings_cross_the_boundary_whole() {
        unsafe {
            let finding = |tag: &str, origin: &str| {
                format!(
                    r#"{{"code":"too-many-keys","tag":"{tag}","args":[["namespace","area"],["limit","1"],["keys","a"],["keys","b"],["keys","c"]],"origin":"{origin}"}}"#
                )
            };
            let expected = [
                finding("area:a", "existing"),
                finding("area:b", "existing"),
                finding("area:c", "introduced"),
            ]
            .join("\n");
            let idx = rules_index(&["cfg tagma.keys-per-item:area=1", "item area:a area:b"]);
            assert_eq!(from_c(validate(idx, "item", "area:c", "")), expected);
            let rules = rules_from("tagma.keys-per-item:area=1");
            assert_eq!(
                from_c(rules_validate(rules, "area:a area:b", "area:c", "")),
                expected
            );
            let conflict = rules_from("tagma.keys-per-item:area=2 tagma.keys-per-item:area=1");
            assert_eq!(
                from_c(tagma_rules_check(conflict)),
                r#"{"code":"conflict","tag":"tagma.keys-per-item:area=2","args":[["facet","keys-per-item"],["resolution","1"]],"origin":"existing"}"#
            );
            assert_eq!(tagma_rules_free(rules), 0);
            assert_eq!(tagma_rules_free(conflict), 0);
            assert_eq!(tagma_index_free(idx), 0);
        }
    }

    #[test]
    fn rules_from_tags_rejects_bad_input() {
        unsafe {
            assert_eq!(tagma_rules_from_tags(std::ptr::null()), 0);
            assert!(take_error().contains("tags is null"));
            let bad = invalid_utf8_cstring();
            assert_eq!(tagma_rules_from_tags(bad.as_ptr()), 0);
            assert!(take_error().contains("not valid UTF-8"));
            let invalid = to_cstring("ok =5");
            assert_eq!(tagma_rules_from_tags(invalid.as_ptr()), 0);
            take_error();
        }
    }

    #[test]
    fn rules_validate_names_the_failing_list_and_rejects_nulls() {
        unsafe {
            let rules = rules_from("");
            for (current, add, remove, prefix) in [
                ("=5", "", "", "current: "),
                ("", "=5", "", "add: "),
                ("", "", "=5", "remove: "),
            ] {
                assert!(rules_validate(rules, current, add, remove).is_null());
                assert!(take_error().starts_with(prefix), "{prefix}");
            }
            let ok = to_cstring("");
            let null = std::ptr::null();
            for (c, a, r, what) in [
                (null, ok.as_ptr(), ok.as_ptr(), "current is null"),
                (ok.as_ptr(), null, ok.as_ptr(), "add is null"),
                (ok.as_ptr(), ok.as_ptr(), null, "remove is null"),
            ] {
                assert!(tagma_rules_validate(rules, c, a, r).is_null());
                assert!(take_error().contains(what), "{what}");
            }
            assert_eq!(tagma_rules_free(rules), 0);
        }
    }

    #[test]
    fn rules_handles_are_validated_like_index_handles() {
        unsafe {
            let rules = rules_from("");
            let idx = tagma_index_new();
            // Neither kind is accepted as the other.
            assert_eq!(tagma_rules_free(idx), -1);
            assert!(take_error().contains("rules handle was not issued by tagma"));
            assert!(tagma_rules_check(idx).is_null());
            assert!(take_error().contains("rules handle was not issued by tagma"));
            assert!(tagma_check(rules).is_null());
            assert!(take_error().contains("index handle was not issued by tagma"));
            // Double free and use after free.
            assert_eq!(tagma_rules_free(rules), 0);
            assert_eq!(tagma_rules_free(rules), -1);
            assert!(take_error().contains("freed rules"));
            assert!(tagma_rules_check(rules).is_null());
            assert!(take_error().contains("freed rules"));
            assert!(rules_validate(rules, "", "", "").is_null());
            assert!(take_error().contains("freed rules"));
            // 0 frees as a no-op and clears nothing it should not.
            set_last_error("sentinel".to_string());
            assert_eq!(tagma_rules_free(0), 0);
            assert_eq!(take_error(), "sentinel", "a 0 handle is a no-op");
            assert_eq!(tagma_index_free(idx), 0);
        }
    }

    #[test]
    fn rules_free_success_clears_a_previous_error() {
        unsafe {
            let rules = rules_from("");
            set_last_error("stale".to_string());
            assert_eq!(tagma_rules_free(rules), 0);
            assert!(tagma_last_error().is_null());
        }
    }

    /// The 0-handle short-circuit in tagma_index_free returns 0 without
    /// touching the error channel.
    #[test]
    fn index_free_of_a_zero_handle_leaves_the_error_untouched() {
        unsafe {
            set_last_error("sentinel".to_string());
            assert_eq!(tagma_index_free(0), 0);
            assert_eq!(take_error(), "sentinel");
        }
    }

    #[test]
    fn json_str_escapes_quotes_backslashes_and_control_characters_only() {
        assert_eq!(json_str("a b"), r#""a b""#);
        // 0x7f (DEL) is not a JSON control character: it passes through.
        assert_eq!(
            json_str("q\"\\\t\n\u{1f}\u{7f}é"),
            "\"q\\\"\\\\\\u0009\\u000a\\u001f\u{7f}é\""
        );
    }

    #[test]
    fn success_clears_previous_error() {
        unsafe {
            let handle = tagma_index_new();
            let bad = to_cstring("a =5");
            assert_eq!(tagma_index_add(handle, bad.as_ptr()), -1);
            assert!(!tagma_last_error().is_null());

            let good = to_cstring("a urgent");
            assert_eq!(tagma_index_add(handle, good.as_ptr()), 0);
            assert!(tagma_last_error().is_null());

            tagma_index_free(handle);
        }
    }
}
