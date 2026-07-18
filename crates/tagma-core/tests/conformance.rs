//! Cucumber-rs conformance harness: implements the frozen step vocabulary
//! (docs/steps.md / PLAN.md Appendix A) against `tagma_core`, running the
//! shared `features/` suite. `[[test]] harness = false` (Cargo.toml).

use cucumber::{given, then, when, World};
use tagma_core::{infix, Index, Tag};

/// Cucumber world: an index plus the outcome slots for the last tag parse,
/// compile, and query/match operations.
#[derive(Debug, Default, World)]
pub struct TagmaWorld {
    index: Index,
    last_tag: Option<Result<Tag, String>>,
    last_compile: Option<Result<String, String>>,
    last_match: Option<Result<Vec<String>, String>>,
}

#[given(expr = "an item {string} tagged {string}")]
fn given_item(world: &mut TagmaWorld, id: String, tags: String) {
    let parsed: Vec<Tag> = tags
        .split_whitespace()
        .map(|t| Tag::parse(t).unwrap_or_else(|e| panic!("invalid tag {t:?}: {e}")))
        .collect();
    world.index.add_item(&id, parsed);
}

#[when(expr = "the tag {string} is parsed")]
fn when_tag_parsed(world: &mut TagmaWorld, s: String) {
    world.last_tag = Some(Tag::parse(&s));
}

#[when(expr = "the query {string} is compiled")]
fn when_query_compiled(world: &mut TagmaWorld, s: String) {
    world.last_compile = Some(infix::compile(&s));
}

#[when(expr = "the query {string} is run")]
fn when_query_run(world: &mut TagmaWorld, s: String) {
    world.last_match = Some(world.index.query(&s));
}

#[when(expr = "the postfix query {string} is run")]
fn when_postfix_query_run(world: &mut TagmaWorld, s: String) {
    world.last_match = Some(world.index.query_postfix(&s));
}

#[then(expr = "it parses with namespace {string}, key {string}, value {string}")]
fn then_it_parses(world: &mut TagmaWorld, namespace: String, key: String, value: String) {
    let tag = world
        .last_tag
        .take()
        .expect("no tag parse was attempted")
        .expect("expected tag parsing to succeed");
    let expected_ns = if namespace.is_empty() {
        None
    } else {
        Some(namespace)
    };
    let expected_value = if value.is_empty() { None } else { Some(value) };
    assert_eq!(tag.namespace, expected_ns, "namespace mismatch");
    assert_eq!(tag.key, key, "key mismatch");
    assert_eq!(tag.value, expected_value, "value mismatch");
}

#[then(expr = "parsing fails")]
fn then_parsing_fails(world: &mut TagmaWorld) {
    let result = world.last_tag.take().expect("no tag parse was attempted");
    assert!(result.is_err(), "expected parsing to fail, got {result:?}");
}

#[then(expr = "the postfix is {string}")]
fn then_the_postfix_is(world: &mut TagmaWorld, expected: String) {
    let result = world.last_compile.take().expect("no compile was attempted");
    assert_eq!(result, Ok(expected));
}

#[then(expr = "compilation fails")]
fn then_compilation_fails(world: &mut TagmaWorld) {
    let result = world.last_compile.take().expect("no compile was attempted");
    assert!(
        result.is_err(),
        "expected compilation to fail, got {result:?}"
    );
}

#[then(expr = "it matches exactly {string}")]
fn then_it_matches_exactly(world: &mut TagmaWorld, expected: String) {
    let result = world
        .last_match
        .take()
        .expect("no query was run")
        .expect("expected query evaluation to succeed");
    let mut expected_ids: Vec<String> = if expected.is_empty() {
        Vec::new()
    } else {
        expected.split_whitespace().map(str::to_string).collect()
    };
    expected_ids.sort();
    let mut actual_ids = result;
    actual_ids.sort();
    assert_eq!(actual_ids, expected_ids);
}

fn main() {
    futures::executor::block_on(
        TagmaWorld::cucumber()
            .fail_on_skipped()
            .run_and_exit(concat!(env!("CARGO_MANIFEST_DIR"), "/../../features")),
    );
}
