//! Cucumber-rs conformance harness: implements the frozen step vocabulary
//! (docs/steps.md / PLAN.md Appendix A) against `tagma_core`, running the
//! shared `features/` suite. `[[test]] harness = false` (Cargo.toml).

use std::cmp::Ordering;
use std::sync::Arc;

use cucumber::{given, then, when, World};
use tagma_core::{infix, token, Index, Tag, TypeComparator};

/// Cucumber world: an index plus the outcome slots for the last tag parse,
/// compile, and query/match operations. `#[derive(World)]` builds a fresh
/// world per scenario via `Default::default()`; [`Default`] is implemented
/// by hand below (rather than derived) so every fresh [`Index`] comes with
/// [`SemverComparator`] pre-registered — see the manual impl for why.
#[derive(Debug, World)]
pub struct TagmaWorld {
    index: Index,
    last_tag: Option<Result<Tag, String>>,
    last_compile: Option<Result<String, String>>,
    last_match: Option<Result<Vec<String>, String>>,
}

impl Default for TagmaWorld {
    fn default() -> Self {
        let mut index = Index::default();
        // SPEC.md §9 (client-loadable type comparison): tagma-core itself
        // ships no semver knowledge. This registration is the test
        // fixture `features/type-comparison.feature` exercises — every
        // scenario gets a fresh Index with "semver" already registered,
        // via ordinary `Given`/`When` steps (a `tagma.type:<target>=semver`
        // tag write, then a relational query), with no new step
        // vocabulary needed (docs/steps.md's frozen ten steps are
        // untouched by this feature).
        index.register_type("semver", Arc::new(SemverComparator));
        TagmaWorld {
            index,
            last_tag: None,
            last_compile: None,
            last_match: None,
        }
    }
}

/// Test fixture only (SemVer 2.0.0, <https://semver.org/#spec-item-11>):
/// full precedence, including the pre-release comparison rules of §11 and
/// build-metadata-is-ignored of §10. Not part of tagma-core's own public
/// API surface — registered only by this conformance harness, standing in
/// for a real client's own comparator.
struct SemverComparator;

impl TypeComparator for SemverComparator {
    fn compare(&self, a: &str, b: &str) -> Option<Ordering> {
        Some(parse_semver(a)?.cmp(&parse_semver(b)?))
    }
}

/// `(major, minor, patch)` plus optional pre-release identifiers. Build
/// metadata (`+...`) is stripped and ignored before this is ever built
/// (SemVer §10), so two strings differing only in build metadata parse
/// identical and compare `Equal`. `None` (a release version) sorts after
/// `Some` (SemVer §11.4: "a pre-release version has lower precedence than
/// the associated normal version").
#[derive(Debug, PartialEq, Eq)]
struct SemverKey {
    core: (u64, u64, u64),
    prerelease: Option<Vec<Identifier>>,
}

impl Ord for SemverKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                // Vec<Identifier> compares lexicographically by its Ord
                // impl, with a shared-prefix-but-shorter Vec sorting
                // Less — exactly SemVer §11.4.4's rule.
                (Some(a), Some(b)) => a.cmp(b),
            }
        })
    }
}

impl PartialOrd for SemverKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One dot-separated pre-release identifier (SemVer §9, §11.4.3):
/// digits-only compares numerically; otherwise lexically (ASCII byte
/// order); a numeric identifier always has lower precedence than an
/// alphanumeric one, regardless of value.
#[derive(Debug, PartialEq, Eq)]
enum Identifier {
    Numeric(u64),
    Alnum(String),
}

impl Ord for Identifier {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Identifier::Numeric(a), Identifier::Numeric(b)) => a.cmp(b),
            (Identifier::Alnum(a), Identifier::Alnum(b)) => a.cmp(b),
            (Identifier::Numeric(_), Identifier::Alnum(_)) => Ordering::Less,
            (Identifier::Alnum(_), Identifier::Numeric(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parses `s` as `MAJOR.MINOR.PATCH(-PRERELEASE)?(+BUILD)?` (SemVer §2,
/// §9, §10), returning `None` for anything that doesn't fit — an
/// unparseable value is `NotComparable` (SPEC.md §9), never a panic.
fn parse_semver(s: &str) -> Option<SemverKey> {
    let core_and_pre = s.split('+').next().unwrap_or(s); // strip build metadata (§10)
    let mut it = core_and_pre.splitn(2, '-');
    let core_str = it.next().unwrap();
    let pre_str = it.next();

    let mut parts = core_str.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None; // more than three dotted components: not X.Y.Z
    }

    let prerelease = match pre_str {
        None => None,
        Some(p) => Some(
            p.split('.')
                .map(parse_identifier)
                .collect::<Option<Vec<_>>>()?,
        ),
    };

    Some(SemverKey {
        core: (major, minor, patch),
        prerelease,
    })
}

fn parse_identifier(part: &str) -> Option<Identifier> {
    if part.is_empty() {
        return None;
    }
    if part.bytes().all(|b| b.is_ascii_digit()) {
        Some(Identifier::Numeric(part.parse().ok()?))
    } else {
        Some(Identifier::Alnum(part.to_string()))
    }
}

#[given(expr = "an item {string} tagged {string}")]
fn given_item(world: &mut TagmaWorld, id: String, tags: String) {
    // Same unquoted-whitespace split as the ARCHITECTURE.md bulk-ingest
    // line format (`Index::add_line`), so a fixture tag can quote a value
    // containing a literal space (SPEC.md §2 QUOTING extension) without
    // being torn into two fields; plain whitespace-separated fixtures
    // split exactly as `str::split_whitespace` would.
    let parsed: Vec<Tag> = token::split_unquoted_whitespace(&tags)
        .unwrap_or_else(|e| panic!("invalid tag list {tags:?}: {e}"))
        .into_iter()
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
