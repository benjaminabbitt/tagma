// Hand-written ergonomic types for the tagma WASM binding (PLAN.md W2).
// Describes the surface shared by every entry point (./  and ./inline).

/** A parsed tag: `namespace`/`value` are `null` when absent, `key` is always present. */
export interface ParsedTag {
  namespace: string | null;
  key: string;
  value: string | null;
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

  /** Releases the underlying WASM-side index. */
  free(): void;
}

/** Compiles an infix query to its canonical postfix form. Throws on compile failure. */
export declare function compile(query: string): string;

/** Parses a write-side tag string. Throws on invalid input. */
export declare function parseTag(tag: string): ParsedTag;
