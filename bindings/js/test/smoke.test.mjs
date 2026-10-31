// Node smoke test (PLAN.md W2 done-when): Index round-trip + compile +
// parseTag + an error throw, against the Node WASM build.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Index, Rules, compile, parseTag } from "../src/node.mjs";

test("Index round-trip: add + query + queryPostfix", () => {
  const idx = new Index();
  idx.add("a urgent lang=en");
  idx.add("b lang=en");
  idx.add("c urgent=false");

  assert.deepEqual(idx.query("urgent").slice().sort(), ["a", "c"]);
  assert.deepEqual(idx.query("lang=en").slice().sort(), ["a", "b"]);
  assert.deepEqual(
    idx.queryPostfix("urgent/lang=en/and").slice().sort(),
    ["a"],
  );
});

test("compile: infix to postfix", () => {
  assert.equal(compile("urgent and range>4"), "urgent/range>4/and");
  assert.equal(compile("not (a and b)"), "a/b/and/not");
});

test("parseTag: returns a plain object", () => {
  assert.deepEqual(parseTag("geo:lat=57.64"), {
    namespace: "geo",
    key: "lat",
    value: "57.64",
  });
  assert.deepEqual(parseTag("urgent"), {
    namespace: null,
    key: "urgent",
    value: null,
  });
});

test("errors throw JS Error with the core's message", () => {
  assert.throws(() => parseTag("=5"), /^Error: token: invalid component ""/);
  assert.throws(() => compile("a and"), Error);
  const idx = new Index();
  assert.throws(() => idx.add("a =5"), Error);
  assert.throws(() => idx.query("a and"), Error);
});

test("infix is a gateway: query(q) === queryPostfix(compile(q))", () => {
  const idx = new Index();
  idx.add("a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done");
  idx.add("b range=tbd lang=en prio:urgent due=2026-08-01");
  idx.add("c urgent=false score=-3 note");
  const table = [
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
  const distinct = new Set();
  for (const q of table) {
    const viaInfix = idx.query(q);
    assert.deepEqual(viaInfix, idx.queryPostfix(compile(q)), q);
    distinct.add(JSON.stringify(viaInfix));
  }
  assert.ok(distinct.size > 3, "table must discriminate between results");
});

test("validate: findings with their origin, args as pairs", () => {
  const idx = new Index();
  idx.add("cfg tagma.enum:k=a tagma.enum:j=a");
  idx.add("item k=z j=bad");
  assert.deepEqual(idx.validate("item", ["k=y"], ["k=z"]), [
    { code: "not-in-enum", tag: "j=bad", args: [["allowed", "a"]], origin: "existing" },
    { code: "not-in-enum", tag: "k=y", args: [["allowed", "a"]], origin: "introduced" },
  ]);
  assert.deepEqual(idx.validate("item", ["k=a", 'j="a"'], ["k=z", "j=bad"]), []);
  // Pure: nothing was written.
  assert.deepEqual(idx.query("k=z and j=bad"), ["item"]);
});

test("validate: a finding with no args has an empty args array", () => {
  const idx = new Index();
  idx.add('cfg tagma.permit:"*:*"=false');
  assert.deepEqual(idx.validate("x", ["lang=en"], []), [
    { code: "not-permitted", tag: "lang=en", args: [], origin: "introduced" },
  ]);
});

test("validate: an invalid tag throws, naming its list", () => {
  const idx = new Index();
  assert.throws(() => idx.validate("x", ["=5"], []), /^Error: add: /);
  assert.throws(() => idx.validate("x", [], ["=5"]), /^Error: remove: /);
});

test("check: config findings, all existing, sorted by tag", () => {
  const idx = new Index();
  assert.deepEqual(idx.check(), []);
  idx.add('cfg tagma.permit:"*:secret"=false tagma.arity:k=scalar tagma.arity:k=set');
  assert.deepEqual(idx.check(), [
    {
      code: "conflict",
      tag: "tagma.arity:k=set",
      args: [["facet", "arity"], ["resolution", "scalar"]],
      origin: "existing",
    },
    {
      code: "inert-false",
      tag: 'tagma.permit:"*:secret"=false',
      args: [["facet", "permit"]],
      origin: "existing",
    },
  ]);
});

test("Rules.fromTags: validate and check mirror the index they come from", () => {
  // Non-config tags among the input are ignored.
  const rules = Rules.fromTags(["tagma.enum:k=a", "tagma.enum:j=a", "k=z"]);
  const idx = new Index();
  idx.add("cfg tagma.enum:k=a tagma.enum:j=a");
  idx.add("item k=z j=bad");
  const viaRules = rules.validate(["k=z", "j=bad"], ["k=y"], ["k=z"]);
  assert.deepEqual(viaRules, idx.validate("item", ["k=y"], ["k=z"]));
  assert.equal(viaRules.length, 2);
  assert.deepEqual(rules.validate([], ["k=a"], []), []);
  assert.deepEqual(rules.check(), []);
  const conflicted = Rules.fromTags(["tagma.arity:k=scalar", "tagma.arity:k=set"]);
  assert.deepEqual(conflicted.check(), [
    {
      code: "conflict",
      tag: "tagma.arity:k=set",
      args: [["facet", "arity"], ["resolution", "scalar"]],
      origin: "existing",
    },
  ]);
});

test("keys-per-item: a namespace over its limit, through the index and Rules", () => {
  const keys = [["namespace", "area"], ["limit", "1"], ["keys", "a"], ["keys", "b"], ["keys", "c"]];
  const expected = [
    { code: "too-many-keys", tag: "area:a", args: keys, origin: "existing" },
    { code: "too-many-keys", tag: "area:b", args: keys, origin: "existing" },
    { code: "too-many-keys", tag: "area:c", args: keys, origin: "introduced" },
  ];
  const idx = new Index();
  idx.add("cfg tagma.keys-per-item:area=1");
  idx.add("item area:a area:b");
  assert.deepEqual(idx.validate("item", ["area:c"], []), expected);
  const rules = Rules.fromTags(["tagma.keys-per-item:area=1"]);
  assert.deepEqual(rules.validate(["area:a", "area:b"], ["area:c"], []), expected);
  const conflicted = Rules.fromTags(["tagma.keys-per-item:area=2", "tagma.keys-per-item:area=1"]);
  assert.deepEqual(conflicted.check(), [
    {
      code: "conflict",
      tag: "tagma.keys-per-item:area=2",
      args: [["facet", "keys-per-item"], ["resolution", "1"]],
      origin: "existing",
    },
  ]);
});

test("Rules: an invalid tag throws, naming its list", () => {
  assert.throws(() => Rules.fromTags(["=5"]), Error);
  const rules = Rules.fromTags([]);
  assert.throws(() => rules.validate(["=5"], [], []), /^Error: current: /);
  assert.throws(() => rules.validate([], ["=5"], []), /^Error: add: /);
  assert.throws(() => rules.validate([], [], ["=5"]), /^Error: remove: /);
});
