# tagma — Specification (draft)

*(from "tagmeme" — linguistics' slot-plus-filler unit: a key and its value)*

A tagging model of three-position tags — `namespace:key=value` with namespace and
value independently optional — plus a postfix query language (canonical/wire form)
and an infix query frontend that compiles down to it.

## 1. Data model

A **tag** is a triple `(namespace?, key, value?)`:

- `key` is mandatory; `namespace` and `value` are independently optional.
- Absent value is genuinely absent — no sentinel, no default. `urgent` and
  `urgent=high` are distinct tags that share a key.
- All stored positions are strings ("strings at rest"). Interpretation (numeric,
  regex) is chosen by the **query operator**, never at write time.
- Keys are **multi-valued**: an item may carry `lang=en` and `lang=fr`
  simultaneously. A value-matching atom matches if *some* tag on the item
  satisfies it.
- Untagging is deletion. There is no "disable" sentinel value.

## 2. Grammar (W3C-flavored EBNF)

```ebnf
/* ---- lexical ---- */
bare-token  ::= [A-Za-z0-9_] [A-Za-z0-9_.-]*
qtoken      ::= '"' ( '""' | [^"] )* '"'    /* '""' escapes one literal '"' */
token       ::= bare-token | qtoken
value-token ::= ("-"? bare-token) | qtoken  /* leading "-" only applies unquoted */

/* Reserved characters (never inside a bare-token; legal, literal content
   inside a qtoken): ":" "=" "<" ">" "~" "!" "/" "*" "+" "(" ")" and
   whitespace. Reserved words (operator names): "and" "or" "not" — also
   escapable by quoting the whole token (e.g. a key literally "and" may be
   spelled `"and"`, alongside the existing redundant `and=*` spelling).   */

/* ---- write-side tag (concrete tokens only; no "*" or "+") ---- */
tag         ::= (namespace ":")? key ("=" value)?
namespace   ::= token
key         ::= token
value       ::= value-token

/* ---- query atom ---- */
quant       ::= "*" | "+"           /* "*" any-or-absent, "+" any-but-present */
q-ns        ::= token | quant
q-key       ::= token | quant
q-value     ::= value-token | quant
op          ::= "=" | "!=" | ">" | ">=" | "<" | "<=" | "~"
atom        ::= (q-ns ":")? q-key (op q-value)?

/* ---- postfix query: canonical / wire / stored form ---- */
postfix     ::= pf-elem ("/" pf-elem)*
pf-elem     ::= atom | pf-op
pf-op       ::= "and" | "or" | "not"

/* ---- infix query: human frontend, compiles to postfix ---- */
query       ::= or-expr
or-expr     ::= and-expr ("or" and-expr)*
and-expr    ::= not-expr ("and" not-expr)*
not-expr    ::= "not" not-expr | primary
primary     ::= atom | "(" query ")"
/* infix elements are whitespace-separated; precedence: not > and > or */
```

**Lexing notes**

- Operators lex longest-match first (`>=` before `>`, `!=` before `!`).
- `temp<-5` lexes as `temp` `<` `-5` — `-` can only lead a value-token.
- Postfix well-formedness is a stack constraint on top of the grammar:
  evaluated left to right with `and`/`or` popping two operands and `not`
  popping one, the sequence must leave exactly one result.

**Quoting** — a `qtoken` may stand in for a `bare-token` in any of the three
tag positions (namespace, key, value) and their query-atom counterparts
(`q-ns`, `q-key`, `q-value`); it shares the `token`/`value-token`
productions above, so a quoted tag key parses identically whether written
into a tag or a query atom (e.g. `tagma:"triage:impact"` is the same key on
both sides).

- **Quoting is syntax, not data.** A `qtoken`'s canonical value is its
  *decoded* content — delimiting quotes stripped, `""` undoubled to a
  single literal `"` — and that decoded string is indistinguishable from
  the same content spelled as a `bare-token`, wherever the bare charset
  would have allowed it. `key="3.5"` and `key=3.5` parse to the identical
  tag; `due~"2026-..-.."` casts and matches identically to
  `due~2026-..-..` (§4's casting rule is unchanged by quoting). A token
  should be spelled quoted only when it must be: it contains a reserved
  character or whitespace, or it is the empty string.
- **Escaping.** `""` inside a `qtoken` decodes to one literal `"` — the
  only escape; there is no backslash metacharacter.
- **Reserved characters and whitespace are legal, literal content** inside
  a `qtoken`, including a literal `/` (see the postfix note below) and the
  delimiter `"` itself (via `""`).
- **Presence vs. absence.** `""` is a *present* value that happens to be
  the empty string, distinct from that position being absent — the same
  distinction §1 already draws for unquoted tags. `key=""` is a valued tag
  whose value is the empty string; bare `key` is valueless.
- **Quantifiers are syntax too.** Quoting `*` or `+` (`"*"`, `"+"`) yields
  the literal one-character token `*`/`+` as data, not the quantifier —
  quoting always turns syntax into data, never the reverse.
- **Unterminated quotes fail to parse**, exactly like any other malformed
  tag or query: an opening `"` with no matching closing `"` is a parse
  error, not a partial match.
- **Postfix stays `/`-delimited** (§5): a `qtoken` containing a literal `/`
  is always legal to *store* (write-side tags never touch postfix) and is
  legal inside a query atom too — the postfix reader treats quoted spans as
  opaque when splitting on `/`, so the `/` survives inside the quotes
  rather than being mistaken for the atom separator. This generalizes,
  rather than lifts, the §6 note that unquoted `~` patterns can't contain a
  literal `/`: quoting is what now makes that possible.

## 3. Atom semantics

An atom denotes the set of items carrying at least one tag that matches it.

| position | absent means | `*` means | `+` means | token means |
|---|---|---|---|---|
| namespace | **null namespace only** | any, **including null** | any **named** namespace | exactly that namespace |
| key | — (mandatory) | any key (≡ `+`) | any key (≡ `*`) | exactly that key |
| value (no op) | existence: valued **or** valueless | — | — | — |
| value (with op) | — | any or absent (so `key=*` ≡ bare `key`) | present (any value) | compared per operator |

- Nothing wildcards implicitly. Crossing namespaces always requires an
  explicit `*:` or `+:`.
- `*` and `+` collapse in the key position because the key cannot be absent —
  the quantifier reading predicts this; both spellings are legal.
- Bare `*` is **not** the universe: an absent namespace position always means
  null-namespace-only, even under a key wildcard, so `*` matches items having
  at least one *un-namespaced* tag. The universe atom is `*:*`. `not` always
  complements over all items in the index, independent of any atom.
- With an operator present, a quantifier in the value position means: `*` —
  matches regardless of value (including absent; so `key=*` ≡ bare `key`);
  `+` — matches iff some value is present (under any operator).
- `key=+` is the "key has a value" test; `key=*` is a legal redundant spelling
  of bare `key`.
- Reserved-word keys: a key literally named `and`, `or`, or `not` cannot be
  queried as a bare atom (it would lex as an operator). Its existence test is
  spelled `and=*` — this is why the redundant `=*` spelling exists.

## 4. Operator semantics — casting rule

**An operator only matches tags it can interpret.** No errors, no coercion
surprises — uninterpretable tags simply don't match:

| op | interpretation | valueless tag | uninterpretable value |
|---|---|---|---|
| `=` / `!=` | exact string equality | no match | — |
| `>` `>=` `<` `<=` | parse both sides as numbers | no match | no match (e.g. `range=tbd` under `range>4`) |
| `~` | regular-expression match on value | no match | no match |

`!=` under multi-valued keys means "some tag with this key has a value ≠ v"
(the existential reading, consistent with every other operator). "No tag has
value v" is spelled `key=v/not/key/and` (infix: `key and not key=v`).

## 5. Query evaluation

Postfix is the query plan: atoms resolve to id-sets via the inverted index;
`and`/`or`/`not` are bitmap intersection/union/complement on a stack.
Implementations should fuse `x/not/and` patterns into set-difference rather
than materializing complements.

Index shape (informative): `(ns, key, value) → ids` inverted index with a
`(ns, key) → ids` level serving bare atoms and `+`/`*` namespace quantifiers.
Value-position-only wildcard queries (`*:*=5`) are grammatical but may be
served by scan until a value-level index earns its keep.

## 6. Resolved for v1 / deferred

- **Quoting (v1, promoted from deferred)**: `"`-delimited `qtoken`s (§2) are
  legal in the namespace, key, and value positions of both the write-side
  tag grammar and the query atom grammar. Quoting is syntax only — the
  canonical form is always the decoded, unquoted content — so it changes
  neither matching nor the §4 casting rule for any value that didn't need
  quoting. This section used to carry a deferred note about a possible
  "quoting extension"; §2 is now that extension, formally specified.
- **`~` pattern language (v1)**: anchored full-value match; the pattern is a
  value-token, where `.` matches any single character and every other
  character matches itself — unchanged by quoting: a quoted pattern decodes
  to the same string a bare one would, so `.` still means "any char", never
  "literal dot". Quoting only lifts the *charset* a pattern may contain
  (e.g. a literal `:` or `/`), not the pattern language itself — a
  literal-dot-only match is still unexpressible in v1 (accepted). Full
  regex support for `~` remains genuinely deferred.
- **Numeric grammar (v1)**: `-? [0-9]+ ("." [0-9]+)?`, compared as IEEE-754
  doubles. No exponents, hex, or leading `+` (reserved). Values outside this
  grammar don't match numeric operators.
- **Operator lexing**: longest match first at the earliest position (`>=`
  before `>`, `!=` before `!`; a lone `!` is invalid).
- **Case sensitivity**: tokens are case-sensitive, including the reserved
  words `and`/`or`/`not`. Revisit only on user-facing friction.
