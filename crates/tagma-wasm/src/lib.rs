//! tagma-wasm: wasm-bindgen wrapper over `tagma-core` (PLAN.md Phase 5,
//! task W1).
//!
//! Thin bindings only: an `Index` class (`add`, `query`, `queryPostfix`,
//! `validate`, `check`), a standalone `Rules` class (`Rules.fromTags`,
//! `validate`, `check`), plus free functions `compile` and `parseTag`. No behavior beyond what
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
use tagma_core::{Change, Finding};
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

    /// Validates a change to item `id` (SPEC.md §13): the findings for the
    /// tag set the item would carry after removing every tag in `remove`
    /// and then adding every tag in `add` (one tag string per element),
    /// against the index's `tagma.*` config as it stands. Pure: the index
    /// is not modified. Returns `{code, tag, args, origin}` objects (see
    /// [`finding_to_js`]). Throws on an invalid tag.
    pub fn validate(
        &self,
        id: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> Result<js_sys::Array, JsValue> {
        let change = parse_change(&add, &remove)?;
        findings_to_js(&self.inner.validate(id, &change))
    }

    /// Checks the index's own `tagma.*` config (SPEC.md §14), returning
    /// findings as [`Index::validate`] does, every `origin` `"existing"`.
    pub fn check(&self) -> Result<js_sys::Array, JsValue> {
        findings_to_js(&self.inner.check())
    }
}

/// A standalone rule set (SPEC.md §10–§12, with §8 arity and §9 type),
/// derived from config tags the caller holds itself rather than from an
/// [`Index`] — `tagma_core::Rules`. No comparators can be registered from
/// JS (this binding takes no callbacks).
#[wasm_bindgen]
pub struct Rules {
    inner: tagma_core::Rules,
}

#[wasm_bindgen]
impl Rules {
    /// Derives the rules from the `tagma.*` config tags among `tags` (one
    /// tag string per element; every other tag is ignored). Throws on an
    /// invalid tag.
    #[wasm_bindgen(js_name = fromTags)]
    pub fn from_tags(tags: Vec<String>) -> Result<Rules, JsValue> {
        let tags = parse_tags(&tags).map_err(to_js_err)?;
        Ok(Rules {
            inner: tagma_core::Rules::from_tags(&tags),
        })
    }

    /// Validates a change to the tag set `current` (SPEC.md §13): the
    /// findings for the set it would become after removing every tag in
    /// `remove`, then adding every tag in `add`, as [`Index::validate`]
    /// returns them. Throws on an invalid tag, naming its list
    /// (`current: `, `add: `, `remove: `).
    pub fn validate(
        &self,
        current: Vec<String>,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> Result<js_sys::Array, JsValue> {
        let current = parse_tags(&current).map_err(|e| to_js_err(format!("current: {e}")))?;
        let change = parse_change(&add, &remove)?;
        findings_to_js(&self.inner.validate(&current, &change))
    }

    /// Checks the config itself (SPEC.md §14), as [`Index::check`] does.
    pub fn check(&self) -> Result<js_sys::Array, JsValue> {
        findings_to_js(&self.inner.check())
    }
}

/// A validate change from its two tag lists, a bad tag named by its list.
fn parse_change(add: &[String], remove: &[String]) -> Result<Change, JsValue> {
    Ok(Change {
        add: parse_tags(add).map_err(|e| to_js_err(format!("add: {e}")))?,
        remove: parse_tags(remove).map_err(|e| to_js_err(format!("remove: {e}")))?,
    })
}

/// Parses each element of `tags` as one tag (SPEC.md §2).
fn parse_tags(tags: &[String]) -> Result<Vec<Tag>, String> {
    tags.iter().map(|t| Tag::parse(t)).collect()
}

/// Sets `obj[key] = value`, as a JS `Error` on failure rather than a panic.
fn set(obj: &js_sys::Object, key: &str, value: &JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(obj, &JsValue::from_str(key), value)
        .map(|_| ())
        .map_err(|_| to_js_err(format!("wasm: failed to set {key}")))
}

/// One finding as a plain object, keys in SPEC.md §13's order: `code` and
/// `origin` (`"introduced"` or `"existing"`) as their stable strings, `tag`
/// in its canonical spelling (SPEC.md §2), and `args` as `[name, value]`
/// pairs in order (a multi-valued argument repeated once per value).
fn finding_to_js(f: &Finding) -> Result<JsValue, JsValue> {
    let args = js_sys::Array::new();
    for (name, value) in &f.args {
        args.push(&js_sys::Array::of2(
            &JsValue::from_str(name),
            &JsValue::from_str(value),
        ));
    }
    let obj = js_sys::Object::new();
    set(&obj, "code", &JsValue::from_str(f.code.as_str()))?;
    set(&obj, "tag", &JsValue::from_str(&f.tag.to_string()))?;
    set(&obj, "args", &args)?;
    set(&obj, "origin", &JsValue::from_str(f.origin.as_str()))?;
    Ok(obj.into())
}

/// Findings as a JS array of [`finding_to_js`] objects, order kept.
fn findings_to_js(findings: &[Finding]) -> Result<js_sys::Array, JsValue> {
    let out = js_sys::Array::new();
    for f in findings {
        out.push(&finding_to_js(f)?);
    }
    Ok(out)
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
