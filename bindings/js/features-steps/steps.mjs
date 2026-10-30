// cucumber-js conformance harness (PLAN.md W3): implements the frozen step
// vocabulary (docs/steps.md / PLAN.md Appendix A) against the Node WASM
// build, running the shared ../../features suite. Mirrors the semantics of
// crates/tagma-core/tests/conformance.rs (the cucumber-rs reference
// harness) exactly.
import {
  Given,
  When,
  Then,
  defineParameterType,
  setWorldConstructor,
} from "@cucumber/cucumber";
import assert from "node:assert/strict";
import { Index, Rules, compile, parseTag } from "../src/node.mjs";

// docs/steps.md's {string} argument, taken VERBATIM between its delimiters.
// cucumber-js's built-in {string} also rewrites `\"` to `"` and `\'` to `'`
// in the captured text — even inside a '-delimited argument — so the
// QUOTING fixture 'x="say \"hi\""' (features/tags.feature) reached the
// binding as x="say "hi"", a different and invalid tag. cucumber-rs and the
// Go port's stringArg/unquoteArg both pass the bytes through untouched; this
// is the same rule: a "-delimited or '-delimited literal, delimiters
// stripped, nothing unescaped.
defineParameterType({
  name: "verbatim",
  regexp: /"[^"]*"|'[^']*'/,
  transformer: (s) => s.slice(1, -1),
});

class TagmaWorld {
  constructor() {
    this.index = new Index();
    this.lastTag = undefined; // { ok: true, value } | { ok: false, error }
    this.lastCompile = undefined;
    this.lastMatch = undefined;
    this.lastFindings = undefined; // the last validate or check call's
    this.givenTags = []; // every tag a Given step wrote, in order
  }
}
setWorldConstructor(TagmaWorld);

Given("an item {verbatim} tagged {verbatim}", function (id, tags) {
  // Composes the bulk-ingest line format (id + " " + tags) and fails loudly
  // (an uncaught throw fails the step) on an invalid tag.
  this.index.add(`${id} ${tags}`);
  this.givenTags.push(...tagList(tags));
});

When("the tag {verbatim} is parsed", function (tag) {
  try {
    this.lastTag = { ok: true, value: parseTag(tag) };
  } catch (error) {
    this.lastTag = { ok: false, error };
  }
});

When("the query {verbatim} is compiled", function (query) {
  try {
    this.lastCompile = { ok: true, value: compile(query) };
  } catch (error) {
    this.lastCompile = { ok: false, error };
  }
});

When("the query {verbatim} is run", function (query) {
  try {
    this.lastMatch = { ok: true, value: this.index.query(query) };
  } catch (error) {
    this.lastMatch = { ok: false, error };
  }
});

When("the postfix query {verbatim} is run", function (query) {
  try {
    this.lastMatch = { ok: true, value: this.index.queryPostfix(query) };
  } catch (error) {
    this.lastMatch = { ok: false, error };
  }
});

Then(
  "it parses with namespace {verbatim}, key {verbatim}, value {verbatim}",
  function (namespace, key, value) {
    assert.ok(this.lastTag, "no tag parse was attempted");
    assert.ok(
      this.lastTag.ok,
      `expected tag parsing to succeed, got error: ${this.lastTag.ok === false ? this.lastTag.error : ""}`,
    );
    const tag = this.lastTag.value;
    const expectedNs = namespace === "" ? null : namespace;
    const expectedValue = value === "" ? null : value;
    assert.equal(tag.namespace, expectedNs, "namespace mismatch");
    assert.equal(tag.key, key, "key mismatch");
    assert.equal(tag.value, expectedValue, "value mismatch");
  },
);

Then("parsing fails", function () {
  assert.ok(this.lastTag, "no tag parse was attempted");
  assert.equal(
    this.lastTag.ok,
    false,
    `expected parsing to fail, got ${JSON.stringify(this.lastTag.value)}`,
  );
});

Then("the postfix is {verbatim}", function (expected) {
  assert.ok(this.lastCompile, "no compile was attempted");
  assert.ok(
    this.lastCompile.ok,
    `expected compile to succeed, got error: ${this.lastCompile.ok === false ? this.lastCompile.error : ""}`,
  );
  assert.equal(this.lastCompile.value, expected);
});

Then("compilation fails", function () {
  assert.ok(this.lastCompile, "no compile was attempted");
  assert.equal(
    this.lastCompile.ok,
    false,
    `expected compilation to fail, got ${JSON.stringify(this.lastCompile.value)}`,
  );
});

Then("it matches exactly {verbatim}", function (expected) {
  assert.ok(this.lastMatch, "no query was run");
  assert.ok(
    this.lastMatch.ok,
    `expected query evaluation to succeed, got error: ${this.lastMatch.ok === false ? this.lastMatch.error : ""}`,
  );
  const expectedIds = expected === "" ? [] : expected.split(/\s+/);
  expectedIds.sort();
  const actualIds = [...this.lastMatch.value].sort();
  assert.deepEqual(actualIds, expectedIds);
});

// docs/steps.md: a tag-list argument, split on unquoted whitespace as the
// bulk-ingest line format is (a "-quoted span, backslash escapes inside it,
// stays one field); "" is the empty list.
function tagList(s) {
  return s.match(/(?:[^\s"]+|"(?:[^"\\]|\\.)*")+/g) ?? [];
}

When(
  "item {verbatim} is validated adding {verbatim} and removing {verbatim}",
  function (id, adding, removing) {
    this.lastFindings = this.index.validate(id, tagList(adding), tagList(removing));
  },
);

When("the config is checked", function () {
  this.lastFindings = this.index.check();
  // The standalone rule set over the same config tags checks identically
  // (Rules.fromTags, SPEC.md §14 reads only the config). No step names the
  // standalone form, so this is how the scenarios reach it. Not done for
  // validate: an item's set is what the index's write path (§8 collapse)
  // left, which the tags the Given steps wrote need not be.
  assert.deepEqual(Rules.fromTags(this.givenTags).check(), this.lastFindings);
});

// Compares findings with a "<code> <tag> ; ..." list as a multiset of
// (code, parsed tag) pairs, ignoring order, args and origin (docs/steps.md).
function assertFindings(actual, expected) {
  const key = (code, tag) => JSON.stringify([code, parseTag(tag)]);
  const got = actual.map((f) => key(f.code, f.tag)).sort();
  const want = expected
    .split(" ; ")
    .map((e) => e.trim())
    .filter((e) => e !== "")
    .map((e) => {
      const space = e.indexOf(" ");
      assert.ok(space > 0, `finding ${JSON.stringify(e)} is not \`<code> <tag>\``);
      return key(e.slice(0, space), e.slice(space + 1));
    })
    .sort();
  assert.deepEqual(got, want);
}

Then("the findings are {verbatim}", function (expected) {
  assert.ok(this.lastFindings, "no validate or check was run");
  assertFindings(this.lastFindings, expected);
});

Then("the introduced findings are {verbatim}", function (expected) {
  assert.ok(this.lastFindings, "no validate or check was run");
  assertFindings(
    this.lastFindings.filter((f) => f.origin === "introduced"),
    expected,
  );
});
