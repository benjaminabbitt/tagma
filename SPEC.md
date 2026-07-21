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

## 7. Self-hosted meta-configuration — `tagma.hide-ns`

tagma configures itself using its own tag model: reserved `tagma.*`
namespaces carry meta-configuration tags, written and read exactly like
ordinary tags. `hide-ns` is the first such feature: per-namespace visibility
control over ordinary queries.

**Config tag form.** `tagma.hide-ns:<ns>=<bool>` declares whether namespace
`<ns>` — the tag's key — is hidden; `<bool>` is the literal token `true` or
`false` (case-sensitive; any other value configures nothing, per §4's "no
errors, no coercion surprises" style). `<ns>` is quoted (§2) when it needs to
be, e.g. `tagma.hide-ns:"weird:ns"=true`.

**Config is stored as tags, and read back as tags.** hide-ns tags live in the
ordinary tag store, never a separate structure; the hide configuration is
*derived* at query time by reading `tagma.hide-ns:*` tags back out.
Implementations may cache the derived result for query performance but must
rebuild or invalidate it whenever a hide-ns tag is added. A hide-ns tag's
effect is store-wide and unconditional — it need not be attached to any
particular item, and once present it governs every subsequent query, not
only ones that reference it. Because this reference core has no untag/delete
operation, a namespace's hide-ns tags are append-only, so `<ns>` may end up
with both a `=true` and a `=false` tag on record; on that conflict, hide wins
(the fail-safe reading).

**Default.** tagma behaves as if an implicit `tagma.hide-ns:tagma=true` is
always present: the entire `tagma.*` meta-family — including hide-ns's own
config tags — is hidden by default. An explicit `tagma.hide-ns:tagma=false`
un-hides it, store-wide.

**Prefix match is dot-delimited, in namespaces only.** Hiding namespace `N`
hides `N` itself and every namespace `N.<anything>`, recursively — `.` is a
genuine hierarchy separator between namespace path components, not a
lexical prefix match:

| hidden | covers | does not cover |
|---|---|---|
| `tagma` | `tagma`, `tagma.arity`, `tagma.hide-ns`, `tagma.arity.sub` | `tagmax`, `tagma-foo`, `tagmaZ` |

Formally: namespace `C` is covered by hidden namespace `N` iff `C == N` or
`C` starts with `N` immediately followed by `.`.

**Visibility rule.** Visibility is decided in two separate steps that must
not be conflated: whether an item *participates* in a query at all, and,
independently, whether one particular *atom* is allowed to match one
particular tag. Only the first is query-wide; the second is always local to
the one atom doing the matching.

- **The query's revealed set** is every namespace named by a concrete
  (non-wildcard) token in *any* atom of the whole query, each revealing its
  own dot-delimited subtree (the same relation as the hide prefix rule
  above, applied in the opposite direction). A namespace wildcard atom
  (`*:key`, `+:key`, bare `*`, `*:*`) names nothing and reveals nothing.
- **A tag is query-visible** iff its namespace is not hidden, or is covered
  by the query's revealed set.
- **Participation.** An item participates in a query iff it has at least
  one query-visible tag. An item with none does not appear in that query's
  result under any combination of operators — this is also the universe
  `not` complements against, not the raw set of every item ever added; a
  universal query (bare `*`, `*:*`) returns exactly the participating set.
- **Matching is per-atom.** An atom matches a tag in a hidden namespace only
  if *that atom itself* — not some other atom elsewhere in the query —
  explicitly names the namespace (a concrete token, its own dot-subtree).
  The query's revealed set governs participation only; it never makes a
  hidden tag matchable by an atom that doesn't itself name it. So in
  `tagma:foo or *:x`, the `*:x` clause never matches a `tagma.arity:x` tag,
  even though the sibling `tagma:foo` clause names `tagma` — that naming
  only affects whether an item carrying `tagma.arity:x` *participates* in
  the query, never what `*:x` itself is allowed to match.

| query | hidden-ns tag | participates? | matched by the non-naming atom? |
|---|---|---|---|
| `tagma.arity:*` | `tagma.arity:x=1` | yes | yes — the atom names it itself |
| `urgent` | `tagma.arity:x=1` (item's only tag) | no | — (item never appears) |
| `*:*` / bare `*` | any hidden-ns tag | no | — (wildcards reveal nothing) |
| `tagma:foo or *:x=1` | `tagma.arity:x=1` | yes (revealed by `tagma:foo`) | no — `*:x=1` doesn't name `tagma.arity` |

An item whose only tags fall in a hidden, unreferenced namespace therefore
never appears in that query's results, in any position — not an error, just
an empty visible tag set, and (since it doesn't participate) excluded from
what `not` complements against too. Mental model: a hidden namespace is a
dotfile — invisible to a bare `ls` (any unreferencing query, universal
included), visible to `ls -a` (participation) only for whichever `ls -a`
invocation actually names it (matching) — a sibling command naming it
elsewhere doesn't retroactively make *this* `ls` show it.

- A namespace wildcard atom (`*:key`, `+:key`, bare `*`, `*:*`) never
  reveals a namespace, for participation or for its own matching —
  wildcards only ever hide.
- The store-wide default/override from **Default** above always applies
  first, to both participation and matching; per-query revealing is a
  second, additive way a namespace becomes visible, but strictly for
  participation (and for the naming atom's own matching) — it never
  extends to any other atom's matching in the same query.

**`.` is a separator in namespaces, not in keys.** In a namespace, `.` is
the dot-delimited hierarchy separator the prefix rule above uses. In a key,
`.` is an ordinary token character — already legal in `bare-token`'s
charset (§2), and often used by convention to suggest hierarchy (`a.b.c` as
a key) — to which tagma applies no splitting semantics: a key is compared
only for exact equality (§3-4), opaque end to end. This is a deliberate
asymmetry: namespaces get real hierarchy semantics via hide-ns; keys do
not. The tokenizer itself is unchanged — `.` remains lexically an ordinary
bare-token character in both positions; the separator meaning is purely
semantic, applied only by hide-ns's prefix matching, never by the lexer or
by key comparison.

## 8. Self-hosted meta-configuration — `tagma.arity`

tagma configures itself using its own tag model (§7): `arity` is the second
self-hosted meta-feature, declaring how many values a given target key may
hold per item. Its config tags live in namespace `tagma.arity`, itself under
the `tagma` family, so they are hidden by §7's default with no
special-casing required — like hide-ns, arity config is *derived* by reading
`tagma.arity:*` tags directly back out of the store, bypassing the
query-time hide.

**Config tag form.** `tagma.arity:<target>=<arity>` declares the arity of
the target key encoded in `<target>` — the config tag's own key, not its
value. `<arity>` is the literal token `scalar` or `set` (case-sensitive; any
other value configures nothing, per §4's "no errors, no coercion surprises"
style, mirroring hide-ns's `<bool>` handling in §7).

**Target encoding — a first-colon split.** `<target>` packs the target
`(namespace?, key)` pair into one string, quoted (§2) whenever it needs to
be: `<targetkey>` alone for a null target namespace (no colon, so no quoting
is needed on that account — e.g. `tagma.arity:k=scalar`), or
`<targetns>:<targetkey>` for a named one (e.g.
`tagma.arity:"triage:impact"=scalar`, quoted because the target string
itself contains a literal `:`). Recovering the target pair from `<target>`
is a **first-colon split**: everything before the first `:` is the target
namespace, everything after is the target key; no `:` means a null target
namespace and the whole string is the target key — the same first-colon
convention a tag's own `namespace:key` grammar uses (§2). It is not applied
recursively: a target key that itself contains a `:` (only reachable by
quoting the target string at config-write time) is indistinguishable from a
namespace separator at read time. This reference implementation does not
attempt to disambiguate that pathological case — documented here, not
solved.

**Arity levels.**
- `set` — the **default** for any undeclared `(namespace, key)`: today's
  unchanged behavior. A key is multi-valued (§1): many values per item,
  unordered, dedup-at-query.
- `scalar` — **at most one value per (target-ns, target-key), per item.**
  Distinct items are unrelated: each independently holds at most one live
  value for a scalar key; a scalar declaration never relates values across
  different items.

**Enforcement — collapse, not rejection.** Writing stays infallible: it is
never an error to write a second value for a scalar key. Instead, when a
tag being written targets a `(ns, key)` declared `scalar`, and the item
already carries a tag with that same `(ns, key)` but a *different* value,
the old value is silently **collapsed** — removed as the new one is kept —
**last-value-wins**. Writing the same value again (an already-present,
*identical* value) is a no-op. Collapse applies uniformly whether the
conflicting values arrive across two separate writes to the same item or
together within one write's tag batch — both leave at most one value
standing.

**Ordering.** Arity config is evaluated **at write time**: a `scalar`
declaration governs writes that happen after tagma has that declaration on
record. Retroactively collapsing values that were already written under the
old (`set`, or undeclared) reading before the `scalar` declaration landed is
out of scope for this reference core — deferred, the same posture as
hide-ns's append-only config (§7).

**Conflicting declarations.** Because this reference core has no
untag/delete operation, a target key's arity config is append-only, so
`<target>` may end up with both a `=scalar` and a `=set` tag on record; on
that conflict, `scalar` wins — the same fail-safe posture as hide-ns's
hide-wins rule (§7), the more restrictive reading taking precedence.
