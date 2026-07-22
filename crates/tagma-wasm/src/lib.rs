//! tagma-wasm: wasm-bindgen wrapper over `tagma-core` (PLAN.md Phase 5,
//! task W1).
//!
//! Thin bindings only: an `Index` class (`add`, `query`, `queryPostfix`),
//! plus free functions `compile` and `parseTag`. No behavior beyond what
//! `tagma-core` already provides; errors from the core are thrown as JS
//! `Error`s carrying the core's `String` message. This crate is built with
//! wasm-bindgen but, per PLAN.md W1, must also compile cleanly for native
//! targets (the shared workspace runs `cargo clippy/test --workspace`).
//!
//! # Panic freedom (enforced — task oily-wheat)
//!
//! `wasm32-unknown-unknown` builds with `panic = "abort"`, so the
//! `catch_unwind` backstop that protects `tagma-ffi` is *inert* here: a
//! panic anywhere in this crate aborts the WebAssembly instance, with no
//! catchable exception and no error return for the host JS to act on. This
//! crate's guarantee can therefore only ever be "never panics", not
//! "catches panics".
//!
//! The `deny`s below are what make that a checked property rather than a
//! promise: every construct that can panic — `unwrap`, `expect`, `panic!`,
//! `unreachable!`, `todo!`, `unimplemented!`, indexing, and string slicing —
//! is a hard error in this crate. Errors must be returned as
//! `Result<_, JsValue>` and surface in JS as a thrown `Error`.
//!
//! Enforced by `just lint-wasm` (part of `just check`), which passes the
//! same denials on the command line so the gate survives even if these
//! attributes are removed, and again by the workspace-wide `just lint`
//! through the attributes themselves. If a future test module needs
//! `unwrap`, put `#[allow(clippy::unwrap_used)]` on that module alone —
//! never widen the crate-level deny, which is the whole gate.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented,
    clippy::indexing_slicing,
    clippy::string_slice
)]

use tagma_core::tag::Tag;
use wasm_bindgen::prelude::*;

/// Converts a core `Result<T, String>` error into a JS `Error`.
fn to_js_err(e: String) -> JsValue {
    js_sys::Error::new(&e).into()
}

/// An in-memory tag index, queryable via infix or postfix queries.
#[wasm_bindgen]
pub struct Index {
    inner: tagma_core::Index,
}

#[wasm_bindgen]
impl Index {
    /// Creates a new, empty index.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Index {
        Index {
            inner: tagma_core::Index::new(),
        }
    }

    /// Parses and adds a `<id> <tag> <tag>...` line to the index. Throws on
    /// an invalid tag.
    pub fn add(&mut self, line: &str) -> Result<(), JsValue> {
        self.inner.add_line(line).map_err(to_js_err)
    }

    /// Compiles `query` (infix) and evaluates it against the index,
    /// returning sorted matching ids. Throws on compile or evaluation
    /// failure.
    pub fn query(&self, query: &str) -> Result<Vec<String>, JsValue> {
        self.inner.query(query).map_err(to_js_err)
    }

    /// Evaluates an already-compiled postfix query directly, returning
    /// sorted matching ids. Throws on evaluation failure.
    #[wasm_bindgen(js_name = queryPostfix)]
    pub fn query_postfix(&self, query: &str) -> Result<Vec<String>, JsValue> {
        self.inner.query_postfix(query).map_err(to_js_err)
    }
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

/// Compiles an infix query to its canonical postfix form. Throws on compile
/// failure.
#[wasm_bindgen]
pub fn compile(query: &str) -> Result<String, JsValue> {
    tagma_core::infix::compile(query).map_err(to_js_err)
}

/// Parses a write-side tag string, returning `{namespace, key, value}`
/// (`namespace`/`value` are `null` when absent). Throws on invalid input.
#[wasm_bindgen(js_name = parseTag)]
pub fn parse_tag(tag: &str) -> Result<JsValue, JsValue> {
    let Tag {
        namespace,
        key,
        value,
    } = Tag::parse(tag).map_err(to_js_err)?;

    let obj = js_sys::Object::new();
    js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("namespace"),
        &namespace.map(JsValue::from).unwrap_or(JsValue::NULL),
    )
    .map_err(|_| to_js_err("wasm: failed to set namespace".to_string()))?;
    js_sys::Reflect::set(&obj, &JsValue::from_str("key"), &JsValue::from_str(&key))
        .map_err(|_| to_js_err("wasm: failed to set key".to_string()))?;
    js_sys::Reflect::set(
        &obj,
        &JsValue::from_str("value"),
        &value.map(JsValue::from).unwrap_or(JsValue::NULL),
    )
    .map_err(|_| to_js_err("wasm: failed to set value".to_string()))?;

    Ok(obj.into())
}
