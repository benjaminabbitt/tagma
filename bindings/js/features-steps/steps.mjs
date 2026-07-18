// cucumber-js conformance harness (PLAN.md W3): implements the frozen step
// vocabulary (docs/steps.md / PLAN.md Appendix A) against the Node WASM
// build, running the shared ../../features suite. Mirrors the semantics of
// crates/tagma-core/tests/conformance.rs (the cucumber-rs reference
// harness) exactly.
import { Given, When, Then, setWorldConstructor } from "@cucumber/cucumber";
import assert from "node:assert/strict";
import { Index, compile, parseTag } from "../src/node.mjs";

class TagmaWorld {
  constructor() {
    this.index = new Index();
    this.lastTag = undefined; // { ok: true, value } | { ok: false, error }
    this.lastCompile = undefined;
    this.lastMatch = undefined;
  }
}
setWorldConstructor(TagmaWorld);

Given("an item {string} tagged {string}", function (id, tags) {
  // Composes the bulk-ingest line format (id + " " + tags) and fails loudly
  // (an uncaught throw fails the step) on an invalid tag.
  this.index.add(`${id} ${tags}`);
});

When("the tag {string} is parsed", function (tag) {
  try {
    this.lastTag = { ok: true, value: parseTag(tag) };
  } catch (error) {
    this.lastTag = { ok: false, error };
  }
});

When("the query {string} is compiled", function (query) {
  try {
    this.lastCompile = { ok: true, value: compile(query) };
  } catch (error) {
    this.lastCompile = { ok: false, error };
  }
});

When("the query {string} is run", function (query) {
  try {
    this.lastMatch = { ok: true, value: this.index.query(query) };
  } catch (error) {
    this.lastMatch = { ok: false, error };
  }
});

When("the postfix query {string} is run", function (query) {
  try {
    this.lastMatch = { ok: true, value: this.index.queryPostfix(query) };
  } catch (error) {
    this.lastMatch = { ok: false, error };
  }
});

Then(
  "it parses with namespace {string}, key {string}, value {string}",
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

Then("the postfix is {string}", function (expected) {
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

Then("it matches exactly {string}", function (expected) {
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
