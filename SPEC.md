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
token       ::= [A-Za-z0-9_] [A-Za-z0-9_.-]*
value-token ::= "-"? token          /* leading "-" admits negative numbers */

/* Reserved characters (never inside tokens):
   ":" "=" "<" ">" "~" "!" "/" "*" "+" "(" ")" and whitespace.
   Reserved words (operator names): "and" "or" "not".                        */

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

- **`~` pattern language (v1)**: anchored full-value match; the pattern is a
  value-token, where `.` matches any single character and every other
  character matches itself. This is what the unquoted charset admits — no
  escapes, so a literal-dot-only match is unexpressible in v1 (accepted).
  A quoting extension to the infix frontend may later admit full regexes;
  postfix stays unquoted, so such patterns must avoid `/`. Deferred.
- **Numeric grammar (v1)**: `-? [0-9]+ ("." [0-9]+)?`, compared as IEEE-754
  doubles. No exponents, hex, or leading `+` (reserved). Values outside this
  grammar don't match numeric operators.
- **Operator lexing**: longest match first at the earliest position (`>=`
  before `>`, `!=` before `!`; a lone `!` is invalid).
- **Case sensitivity**: tokens are case-sensitive, including the reserved
  words `and`/`or`/`not`. Revisit only on user-facing friction.
