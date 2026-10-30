// Node smoke test (PLAN.md W2 done-when): Index round-trip + compile +
// parseTag + an error throw, against the Node WASM build.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Index, compile, parseTag } from "../src/node.mjs";

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
  assert.throws(() => parseTag("=5"), /tag: invalid key/);
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
