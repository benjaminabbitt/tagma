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
