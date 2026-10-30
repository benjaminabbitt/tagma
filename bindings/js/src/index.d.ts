// Hand-written ergonomic types for the tagma WASM binding (PLAN.md W2).
// Describes the surface shared by every entry point (./  and ./inline).

/** A parsed tag: `namespace`/`value` are `null` when absent, `key` is always present. */
export interface ParsedTag {
  namespace: string | null;
  key: string;
  value: string | null;
}

/**
 * One finding (SPEC.md §13/§14): a stable `code`, the `tag` it is about in
 * its canonical spelling, named `args` as `[name, value]` pairs in order (a
 * multi-valued argument repeated once per value), and whether the change
 * introduced it or it already held (`check` findings are all `"existing"`).
 */
export interface Finding {
  code: string;
  tag: string;
  args: [string, string][];
  origin: "introduced" | "existing";
}

/** An in-memory tag index, queryable via infix or postfix queries. */
export declare class Index {
  constructor();

  /**
   * Parses and adds a `<id> <tag> <tag>...` line to the index. Throws on an
   * invalid tag.
   */
  add(line: string): void;

  /**
   * Compiles `query` (infix) and evaluates it against the index, returning
   * sorted matching ids. Throws on compile or evaluation failure.
   */
  query(query: string): string[];

  /**
   * Evaluates an already-compiled postfix query directly, returning sorted
   * matching ids. Throws on evaluation failure.
   */
  queryPostfix(query: string): string[];

  /**
   * Validates a change to item `id` (SPEC.md §13): the findings for the tag
   * set the item would carry after removing every tag in `remove`, then
   * adding every tag in `add` (one tag per element), against the index's
   * `tagma.*` config as it stands. Pure: the index is not modified. Sorted
   * by tag spelling, then code, then args. Throws on an invalid tag.
   */
  validate(id: string, add: string[], remove: string[]): Finding[];

  /**
   * Checks the index's own `tagma.*` config (SPEC.md §14); every finding's
   * origin is `"existing"`.
   */
  check(): Finding[];

  /** Releases the underlying WASM-side index. */
  free(): void;
}

/** Compiles an infix query to its canonical postfix form. Throws on compile failure. */
export declare function compile(query: string): string;

/** Parses a write-side tag string. Throws on invalid input. */
export declare function parseTag(tag: string): ParsedTag;
