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
bare-token  ::= ( [A-Za-z0-9_+-] [A-Za-z0-9_.+-]* ) - ( "*" | "+" )
qtoken      ::= '"' ( '""' | [^"] )* '"'    /* '""' escapes one literal '"' */
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
