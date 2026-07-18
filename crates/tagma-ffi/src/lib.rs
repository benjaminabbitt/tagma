//! tagma-ffi: C ABI over `tagma-core` (Phase 3, PLAN.md §8, task C2).
//!
//! Data-in/data-out over an opaque handle (ARCHITECTURE.md API constraint):
//! strings in, id arrays out (newline-joined), index state inside the core.
//! All strings are UTF-8. A thread-local holds the last error message,
//! readable via [`tagma_last_error`].
#![deny(missing_docs)]

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr, CString};

use tagma_core::Index;

thread_local! {
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_last_error(e: String) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(e));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
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

/// Creates a new, empty index and returns an opaque owning handle to it.
/// The caller must eventually pass the handle to [`tagma_index_free`].
#[no_mangle]
pub extern "C" fn tagma_index_new() -> *mut c_void {
    let idx = Box::new(Index::new());
    Box::into_raw(idx) as *mut c_void
}

/// Frees an index handle previously returned by [`tagma_index_new`]. A null
/// handle is a no-op.
///
/// # Safety
///
/// `handle`, if non-null, must be a still-live pointer previously returned
/// by [`tagma_index_new`], not already freed, and not used again after this
/// call.
#[no_mangle]
pub unsafe extern "C" fn tagma_index_free(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(handle as *mut Index) });
}

/// Parses and adds a `<id> <tag> <tag>...` line to the index (same line
/// format as `tagma-cli query`'s stdin). Returns `0` on success, `-1` on
/// error (see [`tagma_last_error`]).
///
/// # Safety
///
/// `handle` must be a live pointer from [`tagma_index_new`]. `line`, if
/// non-null, must point to a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn tagma_index_add(handle: *mut c_void, line: *const c_char) -> c_int {
    if handle.is_null() {
        set_last_error("ffi: handle is null".to_string());
        return -1;
    }
    let Some(line) = (unsafe { borrow_str(line, "line") }) else {
        return -1;
    };
    let idx = unsafe { &mut *(handle as *mut Index) };
    match idx.add_line(line) {
        Ok(()) => {
            clear_last_error();
            0
        }
        Err(e) => {
            set_last_error(e);
            -1
        }
    }
}

/// Compiles `q` (infix) and evaluates it against the index, returning a
/// newly allocated, newline-joined, sorted list of matching ids (empty
/// string for no matches). Returns `NULL` on error (see
/// [`tagma_last_error`]); free the result with [`tagma_str_free`].
///
/// # Safety
///
/// `handle` must be a live pointer from [`tagma_index_new`]. `q`, if
/// non-null, must point to a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn tagma_query(handle: *mut c_void, q: *const c_char) -> *mut c_char {
    if handle.is_null() {
        set_last_error("ffi: handle is null".to_string());
        return std::ptr::null_mut();
    }
    let Some(q) = (unsafe { borrow_str(q, "query") }) else {
        return std::ptr::null_mut();
    };
    let idx = unsafe { &*(handle as *const Index) };
    match idx.query(q) {
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
}

/// Evaluates an already-compiled postfix query against the index; same
/// return convention as [`tagma_query`].
///
/// # Safety
///
/// `handle` must be a live pointer from [`tagma_index_new`]. `q`, if
/// non-null, must point to a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn tagma_query_postfix(handle: *mut c_void, q: *const c_char) -> *mut c_char {
    if handle.is_null() {
        set_last_error("ffi: handle is null".to_string());
        return std::ptr::null_mut();
    }
    let Some(q) = (unsafe { borrow_str(q, "query") }) else {
        return std::ptr::null_mut();
    };
    let idx = unsafe { &*(handle as *const Index) };
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
}

/// Returns a newly allocated copy of the last error message recorded on
/// this thread, or `NULL` if there is none. Free the result with
/// [`tagma_str_free`].
#[no_mangle]
pub extern "C" fn tagma_last_error() -> *mut c_char {
    LAST_ERROR.with(|slot| match &*slot.borrow() {
        Some(e) => to_c_string(e.clone()),
        None => std::ptr::null_mut(),
    })
}

/// Frees a string previously returned by [`tagma_query`],
/// [`tagma_query_postfix`], [`tagma_compile`], or [`tagma_last_error`]. A
/// null pointer is a no-op.
///
/// # Safety
///
/// `s`, if non-null, must be a still-live pointer previously returned by
/// one of this crate's string-returning functions, not already freed, and
/// not used again after this call.
#[no_mangle]
pub unsafe extern "C" fn tagma_str_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(s) });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_cstring(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    unsafe fn from_c(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null(), "expected non-null string");
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tagma_str_free(ptr) };
        s
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
            assert!(tagma_query(std::ptr::null_mut(), q.as_ptr()).is_null());
            assert_eq!(tagma_index_add(std::ptr::null_mut(), q.as_ptr()), -1);

            tagma_index_free(handle);
            tagma_index_free(std::ptr::null_mut());
            tagma_str_free(std::ptr::null_mut());
        }
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
