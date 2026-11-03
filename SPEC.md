# tagma — Specification (draft)

*(tag + **-ma**, the Greek suffix (-μα) that names the result of an action — as in
*pragma*, the thing done, or *schema*, the shape held: a tagma is the result of
tagging. The name also echoes "tagmeme", linguistics' slot-plus-filler unit: a
key and its value.)*

A tagging model of three-position tags — `namespace:key=value` with namespace and
value independently optional — plus a postfix query language (canonical/wire form)
and an infix query frontend that compiles down to it. Infix is a gateway
only: nothing evaluates infix; it compiles to postfix and runs as postfix
(§5).

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
bare-token  ::= ( [A-Za-z0-9_+-] [A-Za-z0-9_.+-]* ) - ( "*" | "+" )
qtoken      ::= '"' ( '\' ('"' | '\') | [^"\] )* '"'  /* \" and \\ are the only escapes */
token       ::= bare-token | qtoken
value-token ::= bare-token | qtoken         /* identical to token; see below */

/* Reserved characters (never inside a bare-token; legal, literal content
   inside a qtoken): ":" "=" "<" ">" "~" "!" "/" "*" "(" ")" and
   whitespace. Both SIGNS are ordinary token characters in every
   position — "." is the one continuation-only character — and the sole
   rule about the quantifiers is the "- ( "*" | "+" )" above: they are
   quantifiers when, and ONLY when, they constitute the entire token
   (see the "signs and quantifiers" note below). Reserved words
   (operator names): "and" "or" "not", matched
   CASE-INSENSITIVELY as operators on both the postfix and infix sides
   ("AND" "And" "OR" "NOT" etc. all lex as operators, exactly like their
   lowercase spellings) — also escapable by quoting the whole token (e.g. a
   key literally "and", in any case, may be spelled `"and"`, alongside the
   existing redundant `and=*` spelling). Quoting always escapes
   operator-hood: a quoted reserved word (`"and"`, `"AND"`, ...) is never an
   operator, only ever the literal atom, in every case.                   */

/* ---- write-side tag (concrete tokens only; a whole-token "*"/"+" is
   the quantifier, so neither is ever a write-side token) ---- */
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
pf-op       ::= "and" | "or" | "not"   /* case-insensitive; see the reserved-words note above */
/* A postfix query never fails on account of what's left on the stack once
   evaluated (§5): more than one leftover operand folds together with
   "and", left-associatively, rather than erroring. */

/* ---- infix query: human frontend, compiles to postfix ---- */
query       ::= or-expr
or-expr     ::= and-expr ("or" and-expr)*
and-expr    ::= juxt-expr ("and" juxt-expr)*
juxt-expr   ::= not-expr not-expr*    /* juxtaposition: adjacent operands with no explicit "and" between them mean "and" */
not-expr    ::= "not" not-expr | primary
primary     ::= atom | "(" query ")"
/* infix elements are whitespace-separated; precedence: not > and > or;
   juxtaposition (an omitted "and") binds at the same precedence as an
   explicit "and", left-associatively, mirroring the postfix leftover-stack
   fold above so the two forms agree. */
```

**Lexing notes**

- Operators lex longest-match first (`>=` before `>`, `!=` before `!`).
- `temp<-5` lexes as `temp` `<` `-5`: the operator scan finds the earliest
  unquoted operator, so the `<` is taken first and the `-` is left inside
  the value token. No sign is ever a separator, so this is unaffected by
  either sign being an ordinary token character (next note).
- **Signs and quantifiers.** `+` and `-` are ordinary `bare-token`
  characters wherever `[A-Za-z0-9_]` is: `-1`, `+1`, `a-b`, `-key` and
  `1.0.0+build.5` (SemVer 2.0.0 §10 build metadata) are each a single
  token, needing neither quotes nor a per-position carve-out. `.` remains
  continuation-only.
  The **only** rule about `*` and `+` is that they are quantifiers when,
  and only when, they constitute the *entire* token. That is exactly how
  `*` has always behaved; stating it once is what lets the charset simply
  contain `+`. A position holds exactly one token, so `1.0.0+build.5` has
  no competing parse, while `k=+` is still the quantifier.
  This **deletes** a special case rather than adding one: `value-token`
  used to read `("-"? bare-token)` purely to re-admit a leading `-` that
  the charset excluded. With both signs in the charset that patch has no
  job, and `value-token` is now the same production as `token` — one
  validator for all three positions, on the tag side and the query side
  alike. Net: one fewer special case than before, with `-1`, `+1` and
  `1.0.0+build.5` legal as a side effect rather than as three carve-outs.
  `*` is **not** in the charset. That is a UX judgement, not a grammar
  one — the whole-token rule above would keep a lone `*` unambiguous just
  as it does `+` — but someone writing `k=v*` in the hope of a prefix
  match gets a loud parse error today, whereas admitting `*` would give
  them a literal that silently matches nothing. `+` has a must-have
  literal use (SemVer §10); `*` has none.
  The relaxation is **monotonic**: it only newly-accepts input that was a
  parse error before. No sign is a grammar *separator*, so how any string
  splits into tokens is untouched; only the charset check applied to an
  already-delimited component loosens. The one behavioural consequence
  beyond newly-accepted input is that a value spelled with a leading `+`
  before a numeral — reachable today only by quoting, e.g. `k="+1"` —
  now compares numerically under `>` `>=` `<` `<=` instead of never
  matching, because §6's numeral grammar carries the same sign pair (a
  value that *lexes* as a numeral must also *compare* as one, or it
  becomes a silent no-match).

**Quoting** — a `qtoken` may stand in for a `bare-token` in any of the three
tag positions (namespace, key, value) and their query-atom counterparts
(`q-ns`, `q-key`, `q-value`); it shares the `token`/`value-token`
productions above, so a quoted tag key parses identically whether written
into a tag or a query atom (e.g. `tagma:"triage:impact"` is the same key on
both sides).

- **Quoting is syntax, not data.** A `qtoken`'s canonical value is its
  *decoded* content — delimiting quotes stripped, `\"` and `\\` decoded to a
  literal `"` and `\` — and that decoded string is indistinguishable from
  the same content spelled as a `bare-token`, wherever the bare charset
  would have allowed it. `key="3.5"` and `key=3.5` parse to the identical
  tag; `due~"2026-..-.."` casts and matches identically to
  `due~2026-..-..` (§4's casting rule is unchanged by quoting). A token
  should be spelled quoted only when it must be: it contains a reserved
  character or whitespace, or it is the empty string.
- **Escaping.** Backslash is the escape: `\"` decodes to a literal `"` and
  `\\` to a literal `\`. Those are the *only* two escapes — a `\` followed by
  anything else is a parse error, so the grammar stays closed (no `\n`/`\t`/
  `\uXXXX` sequence to define or misread). This matches the escaping the
  formats tags are embedded in already use (JSON, YAML double-quoted, and the
  hew pointer grammar), so a quoted value survives one level of nesting without
  a second escaping convention colliding with it.
- **Reserved characters and whitespace are legal, literal content** inside
  a `qtoken`, including a literal `/` (see the postfix note below); only `"`
  (as `\"`) and `\` (as `\\`) are escaped.
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
- Reserved-word keys: a key literally named `and`, `or`, or `not`, **in any
  case** (§2, §5: the reserved words match case-insensitively), cannot be
  queried as a bare atom (it would lex as an operator). Its existence test is
  spelled `and=*` (or `AND=*`, etc.) — this is why the redundant `=*`
  spelling exists — or the key can be reached directly via quoting (§2),
  e.g. `"and"` or `"AND"`.

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

**Infix is a gateway (normative).** Postfix is the *only* form tagma
evaluates, queries, plans, or indexes against. Infix (§2's `query`) exists
solely as a human front end: an infix query is first compiled to its
canonical postfix string, and that string is then evaluated by the postfix
VM exactly as if the caller had supplied it directly. No implementation,
binding, or port may evaluate infix any other way — no infix AST walker, no
infix-specific matcher, no infix query planner. Consequently, for every
infix query `q` and every index, `query(q)` equals
`query_postfix(compile(q))` (results and errors alike: an infix query that
fails to compile fails before evaluation), and every surface that accepts
infix also exposes `compile`, so the postfix it will run is always
obtainable. Anything new that operates on a query (an API, an optimisation,
an index access path) is defined over postfix; infix gets it for free by
compiling first.

**Leftover-stack fold.** A non-empty postfix query is never rejected for
what it leaves on the stack. Once every token is evaluated, exactly one of
two things is true: exactly one result remains (unchanged from before), or
more than one does — and in that second case the leftovers fold together
with `and`, left-associatively, in the order they sit on the stack (bottom
to top, i.e. the order the atoms/sub-results were originally pushed):
`a/b` means `a and b`; `a/b/c` means `(a and b) and c`; `a/b/or/c` means
`(a or b) and c` — the trailing `c` folds onto whatever the `or` already
reduced to, it does not distribute into it. Stack underflow (an operator
with too few operands) and an empty query remain errors, unaffected by this
rule. This mirrors a downstream consumer's own left-associative fold of a
leftover evaluation stack, and keeps postfix queries assembled by
concatenation (e.g. `a/b/`-joining a filter list) meaningful without every
caller having to interleave explicit `and`s.

**Case-insensitive operators.** `and`/`or`/`not` are matched
case-insensitively as postfix/infix operators (§2) — this makes them
reserved in *every* case, not just lowercase: a bare, unquoted atom spelled
`AND`, `And`, `OR`, `Not`, etc. now lexes as the corresponding operator, the
same way plain `and`/`or`/`not` already did. A quoted spelling (`"and"`,
`"AND"`, ...) always stays a literal atom — quoting escapes operator-hood
regardless of case (§2's QUOTING extension), so a key genuinely named `AND`
is still reachable, spelled `"AND"` (or, via the redundant-`=*` convention,
`"AND"=*` for its bare existence test).

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
- **Numeric grammar (v1)**: `[-+]? [0-9]+ ("." [0-9]+)?`, compared as
  IEEE-754 doubles. No exponents or hex. This is simply what a number
  looks like, and it carries the same sign pair the `bare-token` charset
  does (§2) — a value that *lexes* as a signed numeral also *compares* as
  one, so `k>=1` matches a stored `k=+1`. Values outside this grammar
  don't match numeric operators.
  `=` remains string equality throughout (§4), so `k=+1` and `k=1` are
  distinct tags while both answer `k>=1` — the same asymmetry `-0` and `0`
  already have.
- **Operator lexing**: longest match first at the earliest position (`>=`
  before `>`, `!=` before `!`; a lone `!` is invalid).
- **Case sensitivity**: tokens (namespaces, keys, values) are
  case-sensitive. **Revisited** for the reserved words: `and`/`or`/`not`
  are matched case-insensitively as operators (§2, §5) — `AND`/`And`/`OR`/
  `NOT`/etc. all lex as the corresponding operator — to accept a downstream
  consumer's grammar; this is the one exception; quoting still always
  yields a case-sensitive literal atom, in any case, unaffected by this
  rule.

## 7. Self-hosted meta-configuration — `tagma.hide`

**Self-hosted configuration is a description, not a program.** Every
`tagma.*` facet in this and the following sections (§7–§15) *declares a
fact* about tags — what is hidden, how many values a key holds, what is
permitted, which values or ranges are admitted — and tagma derives the
effective configuration from those facts by fixed rules (restrictive wins).
It MUST NOT be construed as, or grown into, a programming language: no
conditionals, variables, expressions, functions, loops, evaluation order or
user-defined computation, and no facet whose value is code. A need the
existing facets cannot describe is met by a new *declarative* facet with its
own section here, or left to the client, never by making facet values
executable.

tagma configures itself using its own tag model: reserved `tagma.*`
namespaces carry meta-configuration tags, written and read exactly like
ordinary tags. `hide` is the first such feature: pattern-based visibility
control over ordinary queries, at `ns:key` granularity. **This section
replaces the retired namespace-only `tagma.hide-ns` facet outright — it is
a rename plus a generalization, not an addition alongside it.** See
"Renamed from `hide-ns`" at the end of this section for the migration.

**Config tag form.** `tagma.hide:<target>=<bool>` declares whether the
pattern encoded in `<target>` — the tag's own key — is hidden; `<bool>` is
the literal token `true` or `false` (case-sensitive; any other value
configures nothing, per §4's "no errors, no coercion surprises" style).

**Target encoding — a first-colon split**, the same convention §8's
`tagma.arity` target uses for its own `(namespace?, key)` pair: `<target>`
is `<ns-pattern>:<key-pattern>` (quoted, §2, whenever it needs to be — e.g.
`tagma.hide:"tagma:*"=true`, quoted because the target contains a literal
`:`), or `<key-pattern>` alone, with no colon, for a pattern pinned to the
**null namespace** (e.g. `tagma.hide:secret=true`). Recovering the pattern
from `<target>` is not applied recursively: a `<key-pattern>` that itself
contains a `:` is only reachable by quoting `<target>` at config-write time,
and is indistinguishable from a namespace separator at read time — the same
documented (not solved) limitation §8 already carries for its own target
grammar. Similarly, a literal ns- or key-pattern spelled exactly `*` is only
reachable by quoting (`bare-token`'s charset never admits `*`, §2), and is
indistinguishable at read time from the wildcard token below — also
documented, not solved.

**The pattern grammar.** A hide pattern is `<ns-pattern>:<key-pattern>` (or
bare `<key-pattern>` for the null namespace):

- **ns-pattern** matches a tag's namespace by **dot-subtree** — the same
  relation the retired `hide-ns` facet used: `<ns-pattern>` covers a tag's
  namespace `C` iff `C == <ns-pattern>` or `C` starts with `<ns-pattern>`
  immediately followed by `.`. The literal token `*` as ns-pattern means
  **any** namespace, named or null. A `<target>` with no colon pins the
  pattern to the **null namespace only** — an exact match against "no
  namespace," not a subtree match (the null namespace has no subtree to
  recurse into).
- **key-pattern** matches a tag's key **exactly**, or, spelled `*`, matches
  **any** key.

A tag is **hidden** iff it matches at least one currently active hide
pattern — the ns-side (subtree, null, or any) *and* the key-side (exact, or
any) both satisfied. This subsumes the retired `hide-ns` facet exactly:
`tagma.hide:"tagma:*"=true` hides the same set `tagma.hide-ns:tagma=true`
did (the whole `tagma.*` family, every key). It adds two things `hide-ns`
couldn't express: a **per-key** hide within one namespace
(`tagma.hide:"triage:cwe"=true` hides only that one key, leaving sibling
keys under `triage` untouched), and a **cross-namespace** per-key hide
(`tagma.hide:"*:secret"=true` hides a key named `secret` under every
namespace, including the null one). Value-level hiding is out of scope —
`ns:key` is the finest grain this rework adds.

| target | hides |
|---|---|
| `"tagma:*"` | `tagma.*` family, every key (the default; ≡ old `hide-ns:tagma=true`) |
| `"triage:*"` | `triage.*` subtree, every key (≡ old `hide-ns:triage=true`) |
| `"triage:cwe"` | only key `cwe` under `triage`'s subtree; `triage:type` stays visible |
| `"*:secret"` | key `secret` under every namespace, named or null |
| `secret` (no colon) | key `secret` **only** when the tag's namespace is null |
| `*` (no colon) | every null-namespace tag, any key |

**Config is stored as tags, and read back as tags**, exactly as `hide-ns`
worked: hide tags live in the ordinary tag store, never a separate
structure; the hide configuration is *derived* at query (or display) time by
reading `tagma.hide:*` tags back out. Implementations may cache the derived
result for query performance but must rebuild or invalidate it whenever a
hide tag is added. A hide tag's effect is store-wide and unconditional — it
need not be attached to any particular item, and once present it governs
every subsequent query, not only ones that reference it. Because this
reference core has no untag/delete operation, one `<target>` may end up with
both a `=true` and a `=false` tag on record; on that conflict, **hide wins**
(the fail-safe reading) — this reconciliation is per exact `<target>`
string only. Two *different* targets that happen to overlap (e.g. a broad
`"tagma:*"=true` and a narrower `"tagma:foo"=false`) are never reconciled by
specificity: a tag is hidden if it matches **any** currently-active pattern,
full stop — a narrower target explicitly un-hiding a subset that a broader
target still hides does not carve out an exception. Nothing in this rework
required specificity-based tie-breaking among *different* targets, so none
was added; flagged here as a real modeling choice, not an oversight.

**Default.** tagma behaves as if an implicit `tagma.hide:"tagma:*"=true` is
always present: the entire `tagma.*` meta-family — including `hide`'s own
config tags — is hidden by default, at every key. An explicit
`tagma.hide:"tagma:*"=false` un-hides it, store-wide.

**Visibility rule.** Visibility is still decided in the two separate steps
`hide-ns` established, unchanged in shape, generalized in grain: whether an
item *participates* in a query at all (query-wide), and, independently,
whether one particular *atom* is allowed to match one particular tag
(always local to that one atom).

- **Participation.** An item participates in a query iff it has at least
  one query-visible tag. An item with none does not appear in that query's
  result under any combination of operators — this is also the universe
  `not` complements against, not the raw set of every item ever added; a
  universal query (bare `*`, `*:*`) returns exactly the participating set.
- **Matching is per-atom.** An atom matches a hidden tag only if *that atom
  itself* — not some other atom elsewhere in the query — references it
  clearly enough to unhide it (see "Unhide-by-reference" immediately
  below). The query's revealed set governs participation only; it never
  makes a hidden tag matchable by an atom that doesn't itself reference it
  clearly enough.

**Unhide-by-reference — reveal specificity must match hide specificity.**
`hide-ns` had one reveal primitive: naming a namespace concretely unhides
its whole dot-subtree. Generalizing to `ns:key` hides raises a genuine
question `hide-ns` never had to answer: does naming *just the namespace*
(e.g. querying `triage:type`) unhide a *key-level* hide underneath it (e.g.
one declared by `tagma.hide:"triage:cwe"=true`), even though the atom never
names `cwe` at all?

**Chosen rule: no — an atom must be at least as specific as the pattern it
would reveal, in *both* positions.** A hide pattern is a `(ns-pattern,
key-pattern)` pair (see "The pattern grammar" above); a query atom reveals
one iff:

- **ns position**: the atom names a namespace within the pattern's
  dot-subtree — its namespace equals the pattern's ns-pattern, or is a
  dot-descendant of it. A namespace *quantifier* (`*:key`, `+:key`) never
  counts as naming, exactly as in `hide-ns`.
- **key position**: the pattern's key-pattern is `*` (an ns-level hide,
  satisfied regardless of the atom's own key), **or** the atom's key
  equals the pattern's exact key-pattern, **or** the atom's key is itself
  `*`/`+` (a key-wildcard atom reveals an exact key-pattern too, mirroring
  how `*`/`+` already collapse to "any key" for matching itself, §3).

**A tag is query-visible iff *every* active hide pattern that matches it is
revealed** by some atom (participation: any atom anywhere in the query;
matching: that one atom alone — unchanged two-level structure from
`hide-ns`). A tag hidden by two patterns at once — e.g. a broad `"triage:*"`
ns-hide and a narrower `"triage:cwe"` key-hide both landing on the same
`triage:cwe` tag — stays hidden until a query reveals **both**, not just
one.

*Edge case, confirmed*: `triage:type` (an atom naming ns `triage`, key
`type`) does **not** unhide a `triage:cwe`-level hide — it is at least as
specific in the ns position (it names `triage`), but *not* in the key
position (`type` ≠ `cwe`, and `type` is not a key-wildcard), so it fails
the key-position test and the hide stays in force. This was chosen over the
alternative — "naming the ns unhides everything under it, key-level hides
included" (the simpler-sounding rule, and this rework's own first attempt)
— because:

1. **Hide and reveal must be symmetric in grain, or the facet lies about
   its own granularity.** A key-level hide's entire point is "hide *this
   key*, not the whole namespace" — a reveal rule that lets naming the bare
   namespace defeat it silently reintroduces namespace-only granularity
   through the back door, so a per-key hide is never actually more targeted
   than an ns-level one from the query side, only from the config side.
2. It composes correctly when a tag is hidden by *multiple* patterns at
   once (the ns-hide-and-key-hide-together case below): each pattern must
   be independently revealed, and that only means something if "reveal"
   already respects each pattern's own specificity — an asymmetric reveal
   (broad ns-naming defeats everything) would make the narrower pattern
   pointless the moment any broader one coexists with it.
3. It keeps one reveal test ("at least as specific as the hidden pattern,
   in both positions") rather than a special case for "unless the hide
   happens to be key-level, in which case naming the ns alone isn't
   enough" — a single uniform rule, not a rule-plus-exception.

| query atom | hidden tag | reveals `"triage:*"` (ns-hide)? | reveals `"triage:cwe"` (key-hide)? |
|---|---|---|---|
| `triage:type` | `triage:cwe=79` | yes — key-pattern `*` is satisfied regardless | **no** — key `type` ≠ `cwe`, and not a key-wildcard |
| `triage:cwe` | `triage:cwe=79` | yes | yes — exact key match |
| `triage:*` | `triage:cwe=79` | yes | yes — a key-wildcard atom reveals an exact key-pattern too |
| `*:x=1` | (any hidden tag) | no | no — a namespace quantifier never counts as naming |

Consequently: a `triage:cwe=79` tag hidden **only** by `"triage:cwe"=true`
is revealed by `triage:cwe` or `triage:*`, but *not* by `triage:type`. A
`triage:cwe=79` tag hidden by **both** `"triage:*"=true` *and*
`"triage:cwe"=true` needs a query that reveals both at once — `triage:cwe`
does (it satisfies both tables' rows above); `triage:type` reveals only the
ns-hide, so the tag stays hidden by the still-unrevealed key-hide.

An item whose only tags fall in a hidden, unrevealed pattern therefore
never appears in that query's results, in any position — not an error, just
an empty visible tag set, and (since it doesn't participate) excluded from
what `not` complements against too. The "dotfile" mental model from
`hide-ns` still applies unchanged: a hidden namespace or key is invisible to
a bare `ls`, visible to `ls -a` only for whichever `ls -a` invocation is
itself at least as specific as the hide — a sibling command revealing it
elsewhere doesn't retroactively make *this* one show it.

- A namespace or key quantifier atom never reveals anything, for
  participation or for its own matching — quantifiers only ever hide (by
  matching a hide pattern's own `*`), never reveal.
- The store-wide default/override from **Default** above always applies
  first, to both participation and matching; per-query revealing is a
  second, additive way a tag becomes visible, but strictly for
  participation (and for the revealing atom's own matching) — it never
  extends to any other atom's matching in the same query.

**`.` is a separator in namespaces, not in keys.** Unchanged from `hide-ns`:
in a namespace, `.` is the dot-delimited hierarchy separator the ns-pattern
prefix rule uses; in a key, `.` is an ordinary token character, opaque end
to end, compared only for exact equality (§3-4). This asymmetry is
unaffected by generalizing to per-key hides: a hide pattern's key-pattern is
still either exact or `*`, never itself dot-subtree matched. The tokenizer
itself is unchanged — `.` remains lexically an ordinary bare-token
character in both positions; the separator meaning is purely semantic,
applied only by the ns-pattern's prefix matching, never by the lexer or by
key comparison.

**Display predicate — for filtering outside any query.** The visibility
rule above (hide + unhide-by-reference) is inherently query-shaped:
"unhide-by-reference" only makes sense relative to a query that might
reference something. A consumer that just wants to filter an item's tags
for **display** — e.g. rendering a task's tag list, independent of any
search — has no query to reference anything with, so this rework adds a
second, simpler predicate alongside the query-time rule rather than fitting
display awkwardly into it.

**A tag is display-hidden iff it matches at least one currently active hide
pattern — full stop, no unhide-by-reference.** This is deliberately *more*
conservative than query-time visibility: a tag a query could reveal by
naming it is still display-hidden, because display filtering has no query
naming anything. The reference implementation exposes this as a pure
function over a tag and a derived hide-pattern set, so a caller (e.g. a
downstream consumer filtering a task's tags for display) can ask the
question without running a query at all. See the Rust reference's
`tag_hidden`/`HideConfig` (crate `tagma-core`) for the exact public shape;
the Go/Python/JS ports mirror it in their own idiom as a separate, later
task.

**Renamed from `hide-ns`.** This section replaces the namespace-only
`tagma.hide-ns:<ns>=<bool>` facet outright — it is not additive, and this is
an intentional breaking config change: a store carrying `tagma.hide-ns:*`
config tags must be re-written as `tagma.hide:*` ones (a
`tagma.hide-ns:<ns>=<bool>` tag becomes `tagma.hide:"<ns>:*"=<bool>`); the
old facet's tags are not read by the new one — they are now just ordinary
(if invisible-by-default, since they live under `tagma.*`) tags with no
special meaning. tagma's stated posture is to break old users rather than
carry a legacy reading (no backward-compat shims); re-init is the
documented upgrade path.

## 8. Self-hosted meta-configuration — `tagma.arity`

tagma configures itself using its own tag model (§7): `arity` is the second
self-hosted meta-feature, declaring how many values a given target key may
hold per item. Its config tags live in namespace `tagma.arity`, itself under
the `tagma` family, so they are hidden by §7's default with no
special-casing required — like `hide`, arity config is *derived* by reading
`tagma.arity:*` tags directly back out of the store, bypassing the
query-time hide.

**Config tag form.** `tagma.arity:<target>=<arity>` declares the arity of
the target key encoded in `<target>` — the config tag's own key, not its
value. `<arity>` is the literal token `scalar` or `set` (case-sensitive; any
other value configures nothing, per §4's "no errors, no coercion surprises"
style, mirroring `hide`'s `<bool>` handling in §7).

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
`hide`'s append-only config (§7).

**Conflicting declarations.** Because this reference core has no
untag/delete operation, a target key's arity config is append-only, so
`<target>` may end up with both a `=scalar` and a `=set` tag on record; on
that conflict, `scalar` wins — the same fail-safe posture as `hide`'s
hide-wins rule (§7), the more restrictive reading taking precedence.

## 9. Client-loadable type comparison — `tagma.type`

§6's numeric grammar (`-?[0-9]+(\.[0-9]+)?`) is the only interpretation
`>` `>=` `<` `<=` know natively, so a value outside it — a semver string
like `0.7.0`, a date, a version range — can never be ordered: a consumer
cannot ask `blocks-release<=0.7.0`. This section adds a **client-loadable
comparison extension**: an application registers a typed comparator for a
type name at the host-language binding level, without tagma itself ever
knowing what the type means, and declares which `(namespace?, key)`
targets use it via a third self-hosted meta-feature, `tagma.type`,
specified strictly parallel to `tagma.hide` (§7) and `tagma.arity` (§8).

**The comparison interface.** Spec-level and language-neutral:

```text
compare(a: string, b: string) -> { Less, Equal, Greater, NotComparable }
```

Four-valued, deliberately **not** an integer: there is no cross-language
standard for a three-way-compare return type — C and Java specify only the
*sign* of an integer, Go's own convention pins it to exactly `-1`/`0`/`+1`,
and Rust and C++ return an enum. `NotComparable` is modeled on C++20's
`std::partial_ordering::unordered`: a comparator is allowed to say "I
cannot order this pair at all," distinct from `Equal`.

Per-port renderings of the same four-valued type:

- **Rust**: `fn compare(&self, a: &str, b: &str) -> Option<std::cmp::Ordering>`
  on a `TypeComparator` trait (`Send + Sync`), registered via
  `Index::register_type(&mut self, name: &str, cmp: Arc<dyn TypeComparator>)`.
  `None` is `NotComparable`.
- **Go**: `type TypeComparator interface { Compare(a, b string) (result int, ok bool) }`
  on the `Index`, registered via `Index.RegisterType(name string, cmp TypeComparator)`.
  `result` is Go's usual three-way-compare convention, pinned to *exactly*
  `-1`/`0`/`+1` when `ok` is `true`; `ok == false` is `NotComparable`.

**Contract.** A `TypeComparator` implementation MUST be **pure and
deterministic** (the same pair always yields the same result) and MUST
**NOT panic** (or, in a host language, raise across the registration
boundary) — an implementation does not guard each individual comparator
call, so a panicking comparator's effect on the *result* of the query it
interrupts is implementation-defined. It must never, however, be undefined
behavior: an implementation that exposes comparator registration across a
foreign-function boundary MUST contain the unwind at that boundary and
report it through its ordinary error channel rather than letting it escape
into the host's frames. The C ABI does this for every entry point (see
"Panic safety" in `include/tagma.h`), which is what makes registering a
host comparator through it safe to specify at all.

For every pair of values it does *not* return `NotComparable` for, an
implementation MUST satisfy the standard ordering-relation properties:

- **Antisymmetry** — if `compare(a, b)` is `Less`, `compare(b, a)` is
  `Greater` (and vice versa); if `compare(a, b)` is `Equal`, so is
  `compare(b, a)`.
- **Transitivity** — if `compare(a, b)` and `compare(b, c)` agree on a
  direction (both `Less`, or both `Greater`), `compare(a, c)` agrees too.
- **Equality-consistency** — if `compare(a, b)` is `Equal`, then
  `compare(a, c)` and `compare(b, c)` agree, for any `c`, i.e. `a` and `b`
  are interchangeable with respect to ordering against anything else.

**Config tag form.** `tagma.type:<target>=<typename>` declares the type
name of the target key encoded in `<target>` — the config tag's own key,
not its value, exactly `tagma.arity`'s shape (§8). `<typename>` is any
legal tag value (no fixed enumeration, unlike `hide`'s `<bool>` or
`arity`'s `<arity>` token, since the set of type names is open — whatever
names a client registers).

**Target encoding — a first-colon split**, reusing `tagma.arity`'s target
grammar verbatim (§8): `<targetkey>` alone for a null target namespace
(e.g. `tagma.type:blocks-release=semver`), or `<targetns>:<targetkey>` for
a named one, quoted (§2) whenever the target string itself contains a `:`
(e.g. `tagma.type:"triage:eta"=date`). The same first-colon-split
recovery, and the same documented-not-solved colon-in-key limitation, apply
unchanged (§8).

**Conflicting declarations.** Because this reference core has no
untag/delete operation, a target's type config is append-only, so
`<target>` may end up with more than one *distinct* `tagma.type:<target>=`
value on record. Unlike `arity`'s scalar/set (an ordered pair with a
most-restrictive winner) or `hide`'s true/false (hide always wins), there
is no ordering between two type names — `semver` is not "more restrictive"
than `date`. **A target with conflicting `tagma.type` declarations
disables typed comparison for that target outright, falling back to the §6
numeric grammar** — the same outcome as an undeclared target, not an
error.

**Ordering — evaluated at query time.** `tagma.arity` is enforced at
*write* time (§8): a declaration governs writes that happen after tagma
has it on record, and never retroactively changes storage. `tagma.type` is
different — it changes *comparison*, not storage, so it is evaluated at
**query** time: every query re-reads the currently-declared `tagma.type`
config and currently-registered comparators, and a value already written
before a declaration or a comparator registration is compared under
whatever config is active *at query time*, not at write time.

**Failure semantics — all no-match, never error.** This is tagma's own
house rule (§4: "An operator only matches tags it can interpret. No
errors, no coercion surprises"), extended rather than broken by this
feature — typed comparison must not become the only part of tagma that can
fail an evaluation instead of simply not matching:

| condition | outcome |
|---|---|
| value not parseable as its declared type | atom does not match that tag |
| declared type name has no registered comparator | ignore the declaration, fall back to §6 numeric grammar |
| comparator returns `NotComparable` | atom does not match that tag |
| conflicting declarations on the target | fall back to §6 numeric grammar |
| comparator panics | contract violation (see Contract above); query result implementation-defined, but never undefined behavior — an FFI boundary MUST contain the unwind and surface it as an error |

**Precedence — an explicit declaration trumps the numeric grammar.** When
a relational operator (`>` `>=` `<` `<=`) compares a tag's value against
an atom's literal value:

1. If the tag's `(namespace, key)` target has a non-conflicting
   `tagma.type` declaration **and** a comparator is registered under that
   declared name, the comparator is used **exclusively**: the atom matches
   iff the comparator's result satisfies the operator, and if the
   comparator returns `NotComparable` — including because a value isn't
   well-formed for that type — the atom does not match. The §6 numeric
   grammar is **not** consulted for this pair, even if both values happen
   to also parse as numerals.
2. Otherwise — no declaration, a declaration naming an unregistered type,
   or a conflicting declaration (see above) — comparison falls back to the
   §6 numeric grammar, exactly as if this section didn't exist.

Declaring `tagma.type` is an **opt-in to typed semantics for that
target**, and typed semantics take precedence over any syntactic
resemblance to a numeral. Concretely: `1.9` and `1.10` both parse under
§6's grammar as floats, so an undeclared target orders them `1.10 < 1.9`.
But if that target is declared and registered as, say, a `version` type
whose comparator treats `.`-separated components as an ordered tuple
(the ordinary reading for a version number), the declared comparator's
order governs instead — `1.10 > 1.9` — because the declaration says so.
Numeric-shaped is not the same claim as numeric-meaning, and an explicit
declaration resolves that ambiguity in the declarer's favor, not the
grammar's.

**Monotonicity.** This holds in a narrower, but still useful, form than a
first read of "extension" might suggest:

- **For undeclared targets — the overwhelming majority of any store —
  monotonicity holds unconditionally.** Case 2 above governs an undeclared
  target regardless of what comparators are registered or under what
  names; registering a `TypeComparator` can never perturb a query over a
  target nobody declared a type for. Registering an extension is always
  safe with respect to the rest of the store.
- **For a *declared* target, a `tagma.type` declaration MAY change a
  result the numeric grammar alone would have produced — and that is the
  point of declaring it, not a side effect to be suppressed.** The
  declaration is explicit and visible in the data (SPEC.md §7's
  self-hosted-config posture: config lives as ordinary, readable tags, not
  hidden machinery); a client who writes `tagma.type:v=version` is asking
  for `v`'s ordering to change, on that target only.

An earlier draft of this section tried the opposite precedence — numeric
grammar first, unconditionally, with typed comparison only a fallback
when parsing failed — specifically to keep the stronger, SPARQL-style
monotonicity guarantee below intact. That was wrong: it let numeric
interpretation silently override an explicit declaration whenever a value
happened to also be numeral-shaped, which is exactly the `1.9`/`1.10`
case above — the declaration is ignored precisely where it was written to
matter. This section chooses a correct answer for declared targets over
preserving a monotonicity guarantee for them; the guarantee that survives
(undeclared targets are never affected) is the one that matters in
practice, since typed comparison is opt-in, per target, by construction.

**Prior art.**

- **SPARQL 1.1 §17.3.1, Operator Extensibility** — prior art for the
  *mechanism*: a pluggable `<` for a datatype SPARQL doesn't know
  natively, registered by an implementation or client. This section
  **deliberately diverges from SPARQL's own precedence rule**: SPARQL only
  invokes an extension function in place of what would otherwise be a type
  error, so a built-in interpretation that succeeds always wins over an
  extension. tagma instead lets an explicit `tagma.type` declaration take
  precedence over its own built-in numeric grammar whenever a comparator
  is registered for it (see "Precedence" above) — the declaration is what
  the client explicitly asked for, and letting a syntactic coincidence
  silently override it produces wrong orderings (§6's grammar has no
  concept of `1.10 > 1.9`). §17.3.1 is cited here for the
  registration/pluggable-ordering idea, not for its precedence rule, which
  this section does not follow.
  <https://www.w3.org/TR/sparql11-query/#operatorExtensibility>
- **SPARQL 1.1 §15.1, ORDER BY** — "SPARQL does not define a total
  ordering of all possible RDF terms"; an unsupported datatype's relative
  order is left undefined, not an error — the same posture this section's
  failure-semantics table takes. <https://www.w3.org/TR/sparql11-query/#modOrderBy>
- **PostgreSQL §36.14, Index Method Strategies (xindex)** — a B-tree
  operator class's support function 1, `compare`, is the required,
  canonical registered-comparator contract (`sortsupport`, an optional
  accelerant, is not required); this section's `TypeComparator` mirrors
  that required/optional split by specifying only `compare`.
  <https://www.postgresql.org/docs/current/xindex.html>
- **Unicode UTS #10 / ICU collation** — why this section specifies a
  *comparator*, not a sort-key function: ICU's own guidance is that
  `compare()` is faster for a one-off comparison, and sort keys suit
  databases that pay per call; UTS #10 calls a sort key "a logical
  intermediate object," with only the resulting order normative. An
  optional order-preserving key-projection extension point is a
  recognised future addition here too, if indexed range scans over typed
  values are ever added — not part of this slice.
  <https://www.unicode.org/reports/tr10/>
- **JSON Schema 2020-12 §7, `format`** — annotation by default: an
  implementation MUST collect an unrecognized `format` value and MUST NOT
  fail validation because of it. This section's "unregistered type name ->
  ignore the declaration" rule is the same posture: an unrecognized name
  configures nothing, quietly. <https://json-schema.org/draft/2020-12/json-schema-validation>

## 10. Self-hosted meta-configuration — `tagma.permit`

*(owner-approved 2026-10-30 — §10–§14 are the "rules travel with the data"
slice, which §15 extends: tag-schema rules a client self-hosts as config tags in its own
store, plus the two calls that read them. Nothing here changes how a write
or a query behaves; §13 and §14 are new, read-only calls.)*

`permit` is the fourth self-hosted meta-feature (§7–§9): it declares which
tags a store admits, at the same `ns:key` granularity — and with the same
pattern grammar — as `tagma.hide`. It never refuses a write (writes stay
infallible, §4/§8); it is enforced only by a caller that asks first, via
§13's `validate`.

*(owner ruling F6, 2026-11-02: "An undeclared tag is REFUSED when
written. A namespace or tag has to be permitted by an explicit command
first." A permit is a presence, not a boolean, and every store is an
allowlist.)*

**Config tag form.** `tagma.permit:<target>` — a tag with **no value** —
declares that the pattern encoded in `<target>`, the tag's own key, is
permitted. A permit tag carrying any value (`=true`, `=false`, anything
else) configures nothing; §14 reports it as `malformed-value`.

**Target encoding and pattern grammar — §7's, verbatim.** `<target>` is
`<ns-pattern>:<key-pattern>` (quoted whenever it contains a `:`), or a bare
`<key-pattern>` pinned to the **null namespace**. ns-pattern matches by
dot-subtree, or `*` for any namespace (named or null); key-pattern matches
exactly, or `*` for any key. Every target in §7's table means the same set
here: `"triage:*"` (the `triage` subtree, every key), `"triage:cwe"`,
`"*:secret"`, `secret` (null namespace only), `*` (every null-namespace
tag), `"*:*"` (every tag). §7's documented-not-solved limitations (a
literal `:` in a key-pattern, a literal `*` pattern) carry over unchanged.

**Resolution — an allowlist, always.** A tag is **permitted iff it matches
at least one declared permit pattern.** There is no implicit permit: a
store that declares none permits no tag (but the `tagma` family, below),
and one that means to admit every tag declares `tagma.permit:"*:*"`. The
permits on record are one allowlist wherever they are declared — on one
item or many, in any order — and with no value to contradict, two permits
never conflict.

Consequences, stated so nobody has to derive them:

- **Narrowing means removing a permit** (§1: untagging is deletion), never
  declaring a negative one. A pattern is either on record or not.
- **Denylist is not expressible.** Forbidding a pattern inside a broader
  permitted one would need specificity, which §7 rejected.
- **A permit another covers is redundant** (§14: `shadowed-permit`), as
  every other permit is once `"*:*"` is declared.

| config on record | `triage:kind=bug` | `triage.sub:x` | `lang=en` | `secret` |
|---|---|---|---|---|
| *(none)* | **not** | **not** | **not** | **not** |
| `"*:*"` | permitted | permitted | permitted | permitted |
| `"triage:*"` | permitted | permitted (subtree) | **not** | **not** |
| `"triage:kind"`, `lang` | permitted | **not** | permitted | **not** |
| `"*:*"=false`, `"triage:*"=true` | **not** (a valued permit configures nothing) | **not** | **not** | **not** |

**The `tagma` family is exempt.** A tag whose namespace is `tagma` or a
dot-descendant of it (every config tag, §7–§12) is always permitted —
otherwise a store could not declare its first permit, nor any other rule.
Config tags are vetted by §14's `check`, not by `permit`.

**Config is stored as tags and derived at read time**, store-wide and
unconditional, exactly as §7: permit tags live in the ordinary tag store
on any item, and the allowlist is re-derived (or a cache invalidated)
whenever one is added. They are hidden by §7's default like every
`tagma.*` tag.

## 11. Self-hosted meta-configuration — `tagma.enum`

`enum` declares the values a target key admits.

**Config tag form — one tag per value.** `tagma.enum:<target>=<value>`
declares that `<value>` is an admitted value of the target encoded in
`<target>`. An enum of three values is three tags:

```text
tagma.enum:"triage:kind"=defect
tagma.enum:"triage:kind"=capability
tagma.enum:"triage:kind"=chore
```

This is tagma's own multi-value idiom (§1: a key holds many values as many
tags). There is no list syntax: `tagma.enum:"triage:kind"="defect,chore"`
admits exactly one value, the literal string `defect,chore`. A
`tagma.enum` tag with **no value** configures nothing (§14:
`malformed-value`).

**Target encoding — §8's first-colon split, exact targets only.** `<target>`
is `<targetkey>` (null namespace) or `<targetns>:<targetkey>`, recovered
exactly as `tagma.arity`'s is, with §8's colon-in-key limitation. It names
**one exact `(namespace?, key)`** — not a §7 pattern: a target whose ns or
key is `*` configures nothing (§14: `malformed-target`), as for `arity` and
`type`.

**Semantics.** For a declared target, the **admitted set is the set of
every declared value** — the union of all `tagma.enum:<target>=…` tags on
record. A tag on that target is admitted iff it carries a value in the set.
A **valueless** tag on an enum-declared target is not admitted (an enum
constrains the value to be one of a set; absence is not a member). An
undeclared target admits any value.

**Restrictive-wins, applied to sets — why the reading is union** *(owner
ruling, 2026-10-30: union for enum, intersection for range)*. `hide` and `arity` reconcile two
*contradictory* tags on one target (`true`/`false`, `scalar`/`set`); the
more restrictive tag wins. Enum tags on one target never contradict each
other: each one asserts a single fact, "*this* value is admitted," and two
such facts about two different values are both true at once. The
alternative reading — each enum tag is a whole declaration and
restrictive-wins intersects them — makes any enum of two or more values
admit nothing (`{defect} ∩ {chore} = ∅`), i.e. it makes the facet unable to
express its only use. So the restrictive-wins posture applies at the
*facet* level, not between values: **a declared enum (any value on record)
is more restrictive than no enum, and wins** — the moment one
`tagma.enum:<target>` tag lands, every undeclared value on that target
stops being admitted. Narrowing an enum means deleting value tags (§1:
untagging is deletion), which the store's own write path provides; this
reference core, append-only, cannot narrow one (the same posture as §7/§8).

**Interaction with `range` and `type`.** Independent constraints, all of
which must hold (§13). An enum value that a `range` or `type` declaration
on the same target can never admit is dead config (§14:
`dead-enum-value`).

## 12. Self-hosted meta-configuration — `tagma.range`

`range` declares inclusive bounds a target key's values must lie within.

**Config tag form.** `tagma.range:<target>=<range>`, `<target>` exactly as
§11 (§8 first-colon split, one exact `(namespace?, key)`, no wildcards).

**Range grammar.**

```ebnf
range ::= bound? ".." bound?        /* at least one bound present */
bound ::= any non-empty string not containing ".."
```

- Split at the **first** `..`. `1..5`, `-5..-1`, `0.5..1.5`, `5..` (no upper
  bound) and `..5` (no lower bound) are all ranges. `..` alone, a value with
  no `..`, or a bound that is not interpretable (below) configures nothing
  (§14: `malformed-value`).
- **Both bounds are inclusive.** Exclusive bounds are not in this slice.
- **Bound interpretation follows §9's precedence.** If the target has an
  active `tagma.type` declaration (non-conflicting, §9) whose type has a
  registered comparator, bounds and values are compared by that comparator
  (so `tagma.range:"triage:blocks-release"=0.1.0..1.0.0` orders by semver);
  a bound is interpretable iff the comparator orders it against itself
  (`compare(b, b) = Equal`). Otherwise — undeclared, unregistered, or
  conflicting type — bounds must match §6's numeric grammar and compare as
  IEEE-754 doubles.
- **Lexing.** `1..5`, `-5..-1`, `5..` and `0.5..1.5` are bare value-tokens
  (§2: `.` and the signs are continuation characters; a sign may lead). A
  range with no lower bound starts with `.`, which a bare-token cannot, so
  it must be quoted: `tagma.range:k="..5"`. A typed bound containing `..`
  is unrepresentable (documented, not solved — the same posture as §8's
  colon-in-key case).

**Semantics.** A tag on a range-declared target is **in range** iff its
value `v` satisfies `lo ≤ v ≤ hi` for every present bound, compared as
above. A **valueless** tag, or a value the comparison cannot interpret
(not a numeral on an untyped target — §4's casting rule; `NotComparable`
on a typed one), is **not comparable**: it fails the range (§13:
`not-comparable`), exactly as it would fail to match `k>=lo` in a query.

**Restrictive-wins = intersection.** Several `tagma.range` tags on one
target (append-only, as everywhere) are *all* in force: a value must lie
within every one of them, so the effective range is their intersection.
Declarations whose intersection is empty admit no value at all; §14
reports that as `range-empty` rather than silently admitting nothing.

## 13. Validate — asking before writing

Writes stay infallible (§4, §8): `permit`, `enum`, `range`, `type`,
`arity` and `keys-per-item` (§15) never make a write fail. A caller that wants a store to refuse a
bad tag **validates first**, and decides itself what to do with the
findings. Validation is pure: it reads the store's config and one item's
tags, and mutates nothing.

**Shape (language-neutral).**

```text
validate(item: id, change: { add: [tag], remove: [tag] }) -> [finding]
finding = { code: string, tag: tag, args: [(name: string, value: string)],
            origin: "introduced" | "existing" }
```

**The resulting set.** `validate` judges the tag set the item **would**
carry after the change, computed by tagma's own write rules, never the
change in isolation:

1. Start from the item's current tags (empty for an unknown id).
2. Apply every `remove`: delete each current tag equal to it (namespace,
   key and value). Removing a tag the item does not carry is a no-op, not a
   finding.
3. Apply every `add`, in order, exactly as a write would — including §8's
   scalar collapse (a different value on a `scalar` target displaces the
   old one, last-value-wins, within the batch too; an identical value is a
   no-op).

Removes apply **before** adds: a change that both removes and
adds the same tag leaves it on the item. Config is read from the store as
it stands before the change, as §8 reads arity — a config tag inside the
change governs later calls, not this one.

**Every finding is about the resulting set as a whole**, not only the tags
the change touches: a change that removes the offending tag makes an
invalid item valid (no findings), and a change that touches nothing still
reports a pre-existing violation (e.g. two values on a key declared
`scalar` after they were written, §8 "Ordering").

**Introduced or existing.** Every finding carries an `origin`, so a
caller can refuse a write only on what the write itself would break and
merely warn about what was already wrong (as taskloom does). Let *before*
be the item's current tag set and *after* the resulting set above, and
judge **both under the same config** — the store's, as it stands before
the change. A finding of *after* is **`existing`** iff a finding with the
same `code`, the same `tag` (namespace, key and value) and the same `args`
is among *before*'s findings — for `too-many-keys`, the same args but its
`keys` (§15) — otherwise it is **`introduced`**.

- It is defined on outcomes, not on the edit, so it does not depend on how
  the change is spelled: re-adding a tag the item already carries,
  removing and re-adding it in one change, or a §8 collapse that keeps it
  all leave its findings `existing`.
- `permit`, `enum`, `range` and `type` judge one tag on its own, against a
  config that is the same on both sides, so such a finding is `existing`
  iff its tag was already on the item. Only `arity-exceeded` and
  `too-many-keys` depend on a tag's siblings; requiring equal `args` and
  `tag` makes `arity-exceeded` `existing` exactly when the same tag already
  exceeded the same declared arity, and `too-many-keys` (its `keys` aside)
  exactly when the same tag already exceeded the same limit on the same
  namespace. Its `keys` are left out because they name every sibling: a
  change that drops one key but leaves the item over its limit would
  otherwise turn every finding `introduced`, refusing an improvement.
- A violation the change fixes is simply absent from the result — there is
  no "fixed" origin. An empty change reports every finding `existing`.
- Comparing the *change's* tags instead ("is the tag in `add`?") is not
  equivalent: a tag in `add` the item already carries is not new, and a
  tag the change never names can still have a finding it did not cause.

**Finding codes (stable).** One finding per (resulting tag, violated
rule):

| code | when | `tag` | args |
|---|---|---|---|
| `not-permitted` | the tag matches no declared permit pattern (§10); never for a `tagma`-family tag | the tag | — |
| `not-in-enum` | the target has an enum (§11) and the tag's value is absent or not in it | the tag | `allowed` — one arg per admitted value, sorted |
| `out-of-range` | the value is comparable but outside one `tagma.range` declaration (§12) | the tag | `range` — the violated declaration's value, e.g. `1..5` |
| `not-comparable` | the target has a range and the value is absent or cannot be compared to its bounds | the tag | `range` |
| `type-mismatch` | the target has an active, registered `tagma.type` (§9) and the comparator cannot order the value against itself (`compare(v, v) ≠ Equal`) | the tag | `type` |
| `arity-exceeded` | the target is `scalar` (§8) and the resulting set carries more than one value for it | each tag on that target | `arity` = `scalar` |
| `too-many-keys` | the namespace has a `tagma.keys-per-item` limit (§15) and the resulting set carries more distinct keys under it than the limit | each tag in that namespace | `namespace`; `limit` — the effective limit, canonical (§15); `keys` — one arg per distinct key in the namespace, sorted |

`out-of-range` and `not-comparable` are reported once per violated
declaration (a value below two overlapping ranges yields two findings). A valueless tag on a typed
target is not a `type-mismatch`: §9 types govern how values *compare*, and
no value means nothing to compare. Findings are returned sorted by the
tag's canonical spelling, then code (its stable string), then args — the
origin is determined by those three and is not a sort key; an empty list
means the resulting set satisfies every rule. A tag the resulting set
carries twice (tagma's write path does not deduplicate a repeated tag on a
`set` target) draws its findings once.

**Not validated.** `tagma.hide` (visibility is not validity), and the
well-formedness of config tags inside the change (that is §14's job, over
the store).

## 14. Check — a doctor over the config itself

§7–§12 make every malformed or contradictory config tag *configure
nothing* or *lose quietly* — correct for evaluation, opaque to the person
who wrote it. `check` is the compiler-style pass that says so. It is pure,
reads only the store's `tagma.*` tags (and, for `type`, the registered
comparators), and returns findings in §13's shape, sorted the same way.
Every `check` finding's `origin` is `existing`: `check` proposes no change,
so — as for `validate` with an empty change — whatever it finds already
holds. A config tag on record on several items is judged once.

```text
check() -> [finding]
```

**Reserved facets.** The `tagma` namespace family is reserved for
self-hosted config. Its facets are exactly `hide` (§7), `arity` (§8),
`type` (§9), `permit` (§10), `enum` (§11), `range` (§12) and
`keys-per-item` (§15). A tag in namespace `tagma` itself, or in any
`tagma.<x>` (including deeper `tagma.<x>.<y>`) that is not one of those
seven, is reported as
`unknown-facet`.

**Finding codes (stable).** `tag` is always the config tag the finding is
about — for a conflict, the tag that **lost**.

| code | when | args |
|---|---|---|
| `unknown-facet` | namespace in the `tagma` family that is not a facet above | `facet` — the namespace with its leading `tagma.` removed (empty for `tagma` itself) |
| `malformed-value` | `hide` value not `true`/`false`; a `permit` with any value (§10: a presence); `arity` not `scalar`/`set`; `enum`, `type` or `range` with no value; a `range` that does not parse or has an uninterpretable bound (§12); a `keys-per-item` value that is not a non-negative integer (§15) | `facet`, `expected` — `true\|false` (hide), `no value` (permit), `scalar\|set`, `value` (enum), `type name` (type), `lo..hi` (range), `non-negative integer` (keys-per-item) |
| `malformed-target` | a target with an empty namespace or key part (`":k"`, `"ns:"`); a `*` in an `arity`/`type`/`enum`/`range` target (exact targets only); a `keys-per-item` target that is not one exact namespace (`*`, or containing `:`, §15) | `facet` |
| `conflict` | one exact target carries contradictory values: `hide` `true`+`false`, `arity` `scalar`+`set`, `type` two names, `keys-per-item` two different limits (every limit larger than the smallest loses) | `facet`, `resolution` — what won: `true` (hide), `scalar` (arity), `disabled` (type, §9), the smallest limit, canonical (keys-per-item) |
| `inert-false` | a `hide` `=false` on a target with no `=true` on record (and not its default target, `"tagma:*"`) — it deactivates nothing | `facet` |
| `shadowed-permit` | a declared `permit` pattern whose every match is already matched by another declared pattern (§10) | `by` — the covering target |
| `rule-on-unpermitted` | an `enum`/`range`/`type`/`arity` declaration on a target `(ns, key)` that no declared permit pattern admits, or a `keys-per-item` on a namespace no declared permit pattern admits any key of (§15) — it can never constrain a valid tag; in a store declaring no permit, every such declaration | `facet` |
| `range-empty` | two `range` declarations on one target (or one with `lo > hi`) whose intersection admits nothing (§12) | `range` — the other declaration (itself when `lo > hi`) |
| `dead-enum-value` | an `enum` value that a `range` or `type` declaration on the same target never admits | `reason` — `out-of-range`, `not-comparable` or `type-mismatch` |
| `type-unregistered` | a non-conflicting `type` declaration whose name has no registered comparator — §9 falls back to the numeric grammar | `type` |

Coverage for `shadowed-permit` is the §7 pattern relation: ns `*` covers
every ns-pattern; a named ns-pattern covers itself and its dot-descendants;
the null ns-pattern covers only itself; key `*` covers every key-pattern;
an exact key covers only itself. `hide` has no counterpart finding on
purpose: a narrower hide under a broader one is *not* redundant — §7
requires both to be revealed.

A config tag reported `unknown-facet`, `malformed-value` or
`malformed-target` configures nothing, so it draws no further finding (a
malformed permit target is never `shadowed-permit`, for instance). A tag
whose target and value are both malformed is reported `malformed-target`
only: the target is judged first.

Where a finding could name more than one thing, it names one:

- `shadowed-permit`'s `by` is the **broadest** covering pattern — one no
  other covering pattern covers — the first by target spelling if several
  are.
- `range-empty`: a declaration with `lo > hi` is reported once, against
  itself, and is left out of the pairwise comparison (it would otherwise
  be empty against every other declaration too). For the rest, pairwise is
  enough: intervals on one total order that meet pairwise meet jointly.
- `dead-enum-value`: a value the target's type cannot order is reported
  `type-mismatch` alone; otherwise it is reported once per distinct reason
  any `range` on the target gives (`out-of-range`, `not-comparable`).

`check` never changes what §7–§13 and §15 do. A store with findings
evaluates exactly as before; the findings only say why.

## 15. Self-hosted meta-configuration — `tagma.keys-per-item`

*(proposed 2026-10-30, for the owner's review: the one-area/one-repo rule —
an item belongs to at most one `area`, one `repo` — as a facet, not as
application code. Like §10–§12 it never refuses a write; §13's `validate`
reports it and §14's `check` vets it.)*

`keys-per-item` limits how many **distinct keys** one item may carry under
one namespace: `tagma.keys-per-item:area=1` admits `area:cli` or
`area:core`, never both on one item. It is §8's `arity` one level up —
`arity` counts the values under one key, `keys-per-item` the keys under one
namespace — and it is specified the same way.

**Config tag form.** `tagma.keys-per-item:<target>=<limit>`.

**Target — one exact, named namespace.** `<target>` is the namespace
itself, spelled as the config tag's key (`tagma.keys-per-item:area=1`,
`tagma.keys-per-item:triage.sub=1`; quoting, §2, only where the namespace
needs it). It is **exact**, not a §7 pattern:

- the limit counts the keys of tags whose namespace **equals** `<target>`;
  a dot-descendant is another namespace (`area.sub:x` is not under
  `area`), with a limit of its own or none;
- a `*` target, a target containing `:`, and an empty one configure
  nothing (§14: `malformed-target`);
- the **null namespace has no target**: a bare `<target>` already names a
  namespace, so there is no spelling left for "no namespace". Documented,
  not solved.

Exact rather than §7's subtree-or-`*` patterns, because the facet is a
*constraint*, like `arity`, `enum` and `range`, which all take exact
targets (§8, §11, §12), and not a *membership* predicate like `hide` and
`permit`: a subtree target would have to say whether `area:x` and
`area.sub:x` are one key or two, and a `*` target would cap every
namespace at once, which no use asks for.

**Limit — a non-negative integer.** `<limit>` is one or more ASCII digits
(`[0-9]+`), compared as the integer it spells, at any length: leading zeros
are allowed and ignored (`01` is `1`), and a limit beyond any machine
integer still compares exactly. Its **canonical** spelling — the digits
without leading zeros, `0` for all zeros — is what findings report. Any
other value (a sign, a fraction, a word, spaces, no value) configures
nothing (§14: `malformed-value`, expected `non-negative integer`). `0`
admits no key at all under the namespace.

**Semantics.** For a limited namespace, the resulting set (§13) is over the
limit iff it carries more distinct keys under the namespace than the limit;
values do not count (`area:cli=1` and `area:cli=2` are one key). Every tag
in an over-limit namespace is reported `too-many-keys` (§13) — as
`arity-exceeded` reports every value of a scalar key, since no tag among
them is the one at fault. An unlimited namespace admits any number of keys.

**Restrictive-wins = the smallest limit.** Several `tagma.keys-per-item`
tags on one target (append-only, as everywhere) all hold: an item within
the smallest is within every one, so the smallest limit is the one in
force. Two different limits on one target contradict each other, and §14
reports each larger one as a `conflict` naming the smallest; two spellings
of one limit (`1`, `01`) do not.

**Interaction with other facets.** Independent of `permit`, `enum`,
`range`, `type` and `arity`: every tag counts toward its namespace's keys,
permitted or not. A limit on a namespace no declared permit pattern admits
any key of can never constrain a valid tag (§14: `rule-on-unpermitted`).
The `tagma` family is not exempt — a limit on a `tagma.<x>` namespace
counts its config tags' keys, as any other.

**Config is stored as tags and derived at read time**, store-wide and
unconditional, as §7–§12: limit tags live on any item, and are hidden by
§7's default like every `tagma.*` tag.
