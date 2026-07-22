package tagma_test

// Godog conformance harness (PLAN.md G2): implements exactly the nine (ten,
// see below) steps of the frozen vocabulary in docs/steps.md against this
// package's public API, run against the shared ../../features suite.
//
// docs/steps.md's header count ("Nine steps") undercounts its own listed
// step vocabulary by one — the fenced block lists four `When` steps and
// five `Then` steps (plus one `Given`), i.e. ten total. All ten are
// implemented here regardless of the header's count, since the feature
// files use exactly these ten step texts.
//
// docs/steps.md: "{string} is a quoted cucumber-expression string" — the
// QUOTING extension (SPEC.md §2) needs fixtures that embed a literal '"'
// (e.g. a quoted qtoken), so features/*.feature spells those step
// arguments with the *other* legal {string} delimiter, a single quote
// ('...'), to avoid escaping (see e.g. features/tags.feature's "quoted
// tokens" scenario outline). Both delimiters can appear in the same step,
// even the same line (a bare id in "..." next to a tags list in '...').
// Every step regex below therefore matches either delimiter per argument
// via stringArg, capturing the delimiters along with the content in one
// group (so godog's positional arg-to-param mapping — one capture group
// per handler parameter — stays intact); unquoteArg strips them back off.

import (
	"context"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"testing"

	"github.com/cucumber/godog"

	tagma "github.com/benjaminabbitt/tagma/ports/go"
)

// conformanceState holds the "current" parse/compile/query result for the
// scenario in progress, mirroring docs/steps.md's semantics: each `When`
// records a result (value or error) that the following `Then` inspects.
type conformanceState struct {
	idx *tagma.Index

	parsedTag tagma.Tag
	parseErr  error

	compiledPostfix string
	compileErr      error

	matchResult []string
	matchErr    error
}

func (c *conformanceState) reset() {
	idx := tagma.NewIndex()
	// SPEC.md §9 (client-loadable type comparison): tagma itself ships no
	// semver/version knowledge. These registrations are the test fixtures
	// ../../features/type-comparison.feature exercises — every scenario
	// gets a fresh Index with both already registered, via ordinary
	// Given/When steps (a tagma.type:<target>=<name> tag write, then a
	// relational query), with no new step vocabulary needed (docs/steps.md's
	// frozen ten steps are untouched by this feature).
	idx.RegisterType("semver", semverComparator{})
	// versionComparator: a second, deliberately different fixture from
	// semverComparator — used by the "explicit declaration takes
	// precedence" scenario (SPEC.md §9 "Precedence"), which needs a
	// comparator that accepts 1.9/1.10 (two components; semverComparator's
	// strict three-component core would reject them as unparseable).
	idx.RegisterType("version", versionComparator{})
	*c = conformanceState{idx: idx}
}

// versionComparator is a test fixture only: a plain dotted-integer-tuple
// version comparator — 1.9 < 1.10 (component-wise numeric comparison, not
// string/float comparison), with a shorter tuple that's a prefix of a
// longer one sorting first (1.2 < 1.2.1). Deliberately simpler than
// semverComparator — no fixed arity, no pre-release/build-metadata
// grammar — specifically so it accepts 1.9/1.10, which are *also* both
// parseable under tagma's own §6 numeric grammar as the floats 1.9/1.1.
// That overlap is the whole point: it demonstrates SPEC.md §9
// "Precedence" — a declared, registered comparator is used exclusively,
// so 1.10 > 1.9 (version order) on a declared target, even though the
// numeric grammar alone would say 1.10 < 1.9 (float order) on an
// undeclared one.
type versionComparator struct{}

func (versionComparator) Compare(a, b string) (int, bool) {
	pa, ok := parseVersion(a)
	if !ok {
		return 0, false
	}
	pb, ok := parseVersion(b)
	if !ok {
		return 0, false
	}
	for i := 0; i < len(pa) && i < len(pb); i++ {
		if c := compareUint(pa[i], pb[i]); c != 0 {
			return c, true
		}
	}
	return compareInt(len(pa), len(pb)), true
}

func parseVersion(s string) ([]uint64, bool) {
	if s == "" {
		return nil, false
	}
	parts := strings.Split(s, ".")
	out := make([]uint64, len(parts))
	for i, p := range parts {
		n, err := strconv.ParseUint(p, 10, 64)
		if err != nil {
			return nil, false
		}
		out[i] = n
	}
	return out, true
}

// semverComparator is a test fixture only (SemVer 2.0.0,
// https://semver.org/#spec-item-11): full precedence, including the
// pre-release comparison rules of §11 and build-metadata-is-ignored of
// §10. Not part of ports/go's own public API surface — registered only by
// this conformance harness, standing in for a real client's own
// comparator.
type semverComparator struct{}

func (semverComparator) Compare(a, b string) (int, bool) {
	pa, ok := parseSemver(a)
	if !ok {
		return 0, false
	}
	pb, ok := parseSemver(b)
	if !ok {
		return 0, false
	}
	return pa.compare(pb), true
}

type semverIdentKind int

const (
	semverNumeric semverIdentKind = iota
	semverAlnum
)

// semverIdent is one dot-separated pre-release identifier (SemVer §9,
// §11.4.3): digits-only compares numerically; otherwise lexically (ASCII
// byte order); a numeric identifier always has lower precedence than an
// alphanumeric one, regardless of value.
type semverIdent struct {
	kind semverIdentKind
	num  uint64
	str  string
}

func (a semverIdent) compare(b semverIdent) int {
	switch {
	case a.kind == semverNumeric && b.kind == semverNumeric:
		return compareUint(a.num, b.num)
	case a.kind == semverAlnum && b.kind == semverAlnum:
		return strings.Compare(a.str, b.str)
	case a.kind == semverNumeric: // b is alnum: numeric always sorts lower
		return -1
	default: // a is alnum, b is numeric
		return 1
	}
}

// semverKey is (major, minor, patch) plus optional pre-release
// identifiers. Build metadata (+...) is stripped and ignored before this
// is ever built (SemVer §10), so two strings differing only in build
// metadata parse identical and compare equal. !hasPre (a release version)
// sorts after hasPre (SemVer §11.4: "a pre-release version has lower
// precedence than the associated normal version").
type semverKey struct {
	major, minor, patch uint64
	hasPre              bool
	pre                 []semverIdent
}

func (a semverKey) compare(b semverKey) int {
	if c := compareUint(a.major, b.major); c != 0 {
		return c
	}
	if c := compareUint(a.minor, b.minor); c != 0 {
		return c
	}
	if c := compareUint(a.patch, b.patch); c != 0 {
		return c
	}
	switch {
	case !a.hasPre && !b.hasPre:
		return 0
	case !a.hasPre: // a is a release, b a pre-release: release wins (SemVer §11.4)
		return 1
	case !b.hasPre:
		return -1
	}
	for i := 0; i < len(a.pre) && i < len(b.pre); i++ {
		if c := a.pre[i].compare(b.pre[i]); c != 0 {
			return c
		}
	}
	// Shared prefix ties: the longer pre-release identifier set wins
	// (SemVer §11.4.4).
	return compareInt(len(a.pre), len(b.pre))
}

func compareUint(a, b uint64) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	default:
		return 0
	}
}

func compareInt(a, b int) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	default:
		return 0
	}
}

// parseSemver parses s as MAJOR.MINOR.PATCH(-PRERELEASE)?(+BUILD)? (SemVer
// §2, §9, §10), returning (_, false) for anything that doesn't fit — an
// unparseable value is NotComparable (SPEC.md §9), never a panic.
func parseSemver(s string) (semverKey, bool) {
	if i := strings.IndexByte(s, '+'); i != -1 {
		s = s[:i] // strip build metadata (SemVer §10) — ignored entirely for precedence
	}
	core := s
	var preStr string
	hasPre := false
	if i := strings.IndexByte(s, '-'); i != -1 {
		core = s[:i]
		preStr = s[i+1:]
		hasPre = true
	}
	parts := strings.Split(core, ".")
	if len(parts) != 3 {
		return semverKey{}, false
	}
	major, err1 := strconv.ParseUint(parts[0], 10, 64)
	minor, err2 := strconv.ParseUint(parts[1], 10, 64)
	patch, err3 := strconv.ParseUint(parts[2], 10, 64)
	if err1 != nil || err2 != nil || err3 != nil {
		return semverKey{}, false
	}
	key := semverKey{major: major, minor: minor, patch: patch, hasPre: hasPre}
	if hasPre {
		for _, part := range strings.Split(preStr, ".") {
			ident, ok := parseSemverIdent(part)
			if !ok {
				return semverKey{}, false
			}
			key.pre = append(key.pre, ident)
		}
	}
	return key, true
}

func parseSemverIdent(part string) (semverIdent, bool) {
	if part == "" {
		return semverIdent{}, false
	}
	if isAllDigits(part) {
		n, err := strconv.ParseUint(part, 10, 64)
		if err != nil {
			return semverIdent{}, false
		}
		return semverIdent{kind: semverNumeric, num: n}, true
	}
	return semverIdent{kind: semverAlnum, str: part}, true
}

func isAllDigits(s string) bool {
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

// Given an item {string} tagged {string}
func (c *conformanceState) anItemTagged(id, tags string) error {
	id, tags = unquoteArg(id), unquoteArg(tags)
	// Same unquoted-whitespace split as the ARCHITECTURE.md bulk-ingest
	// line format (Index.AddLine), so a fixture tag can quote a value
	// containing a literal space (SPEC.md §2 QUOTING extension) without
	// being torn into two fields: concatenating id and tags and handing
	// the whole line to AddLine reuses that splitter instead of
	// duplicating it here.
	line := id + " " + tags
	if err := c.idx.AddLine(line); err != nil {
		// docs/steps.md: "panics on invalid tag" — Background fixtures are
		// assumed always well-formed; a bad fixture is a harness bug.
		panic(fmt.Sprintf("tagma: invalid tag in item %q tagged %q: %v", id, tags, err))
	}
	return nil
}

// When the tag {string} is parsed
func (c *conformanceState) theTagIsParsed(input string) error {
	c.parsedTag, c.parseErr = tagma.ParseTag(unquoteArg(input))
	return nil
}

// When the query {string} is compiled
func (c *conformanceState) theQueryIsCompiled(input string) error {
	c.compiledPostfix, c.compileErr = tagma.Compile(unquoteArg(input))
	return nil
}

// When the query {string} is run
func (c *conformanceState) theQueryIsRun(input string) error {
	c.matchResult, c.matchErr = c.idx.Query(unquoteArg(input))
	return nil
}

// When the postfix query {string} is run
func (c *conformanceState) thePostfixQueryIsRun(input string) error {
	c.matchResult, c.matchErr = c.idx.QueryPostfix(unquoteArg(input))
	return nil
}

// Then it parses with namespace {string}, key {string}, value {string}
func (c *conformanceState) itParsesWithNamespaceKeyValue(ns, key, val string) error {
	ns, key, val = unquoteArg(ns), unquoteArg(key), unquoteArg(val)
	if c.parseErr != nil {
		return fmt.Errorf("expected successful parse, got error: %v", c.parseErr)
	}
	var gotNS, gotVal string
	if c.parsedTag.Namespace != nil {
		gotNS = *c.parsedTag.Namespace
	}
	if c.parsedTag.Value != nil {
		gotVal = *c.parsedTag.Value
	}
	if gotNS != ns || c.parsedTag.Key != key || gotVal != val {
		return fmt.Errorf("got (ns=%q, key=%q, value=%q), want (ns=%q, key=%q, value=%q)",
			gotNS, c.parsedTag.Key, gotVal, ns, key, val)
	}
	return nil
}

// Then parsing fails
func (c *conformanceState) parsingFails() error {
	if c.parseErr == nil {
		return fmt.Errorf("expected parsing to fail, got %+v", c.parsedTag)
	}
	return nil
}

// Then the postfix is {string}
func (c *conformanceState) thePostfixIs(want string) error {
	want = unquoteArg(want)
	if c.compileErr != nil {
		return fmt.Errorf("expected successful compile, got error: %v", c.compileErr)
	}
	if c.compiledPostfix != want {
		return fmt.Errorf("got postfix %q, want %q", c.compiledPostfix, want)
	}
	return nil
}

// Then compilation fails
func (c *conformanceState) compilationFails() error {
	if c.compileErr == nil {
		return fmt.Errorf("expected compilation to fail, got postfix %q", c.compiledPostfix)
	}
	return nil
}

// Then it matches exactly {string}
func (c *conformanceState) itMatchesExactly(want string) error {
	want = unquoteArg(want)
	if c.matchErr != nil {
		return fmt.Errorf("expected successful query, got error: %v", c.matchErr)
	}
	var wantIDs []string
	if strings.TrimSpace(want) != "" {
		wantIDs = strings.Fields(want)
	}
	sort.Strings(wantIDs)
	got := append([]string(nil), c.matchResult...)
	sort.Strings(got)
	if !equalStringSlices(got, wantIDs) {
		return fmt.Errorf("matched %v, want %v", got, wantIDs)
	}
	return nil
}

func equalStringSlices(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// InitializeScenario wires the frozen step vocabulary (docs/steps.md) to
// conformanceState, with a fresh Index (and cleared result state) before
// every scenario.
func InitializeScenario(sc *godog.ScenarioContext) {
	state := &conformanceState{}

	sc.Before(func(ctx context.Context, s *godog.Scenario) (context.Context, error) {
		state.reset()
		return ctx, nil
	})

	sc.Given(`^an item `+stringArg+` tagged `+stringArg+`$`, state.anItemTagged)
	sc.When(`^the tag `+stringArg+` is parsed$`, state.theTagIsParsed)
	sc.When(`^the query `+stringArg+` is compiled$`, state.theQueryIsCompiled)
	sc.When(`^the query `+stringArg+` is run$`, state.theQueryIsRun)
	sc.When(`^the postfix query `+stringArg+` is run$`, state.thePostfixQueryIsRun)
	sc.Then(`^it parses with namespace `+stringArg+`, key `+stringArg+`, value `+stringArg+`$`, state.itParsesWithNamespaceKeyValue)
	sc.Then(`^parsing fails$`, state.parsingFails)
	sc.Then(`^the postfix is `+stringArg+`$`, state.thePostfixIs)
	sc.Then(`^compilation fails$`, state.compilationFails)
	sc.Then(`^it matches exactly `+stringArg+`$`, state.itMatchesExactly)
}

// stringArg is the regex fragment for a single {string}-style step
// argument (docs/steps.md): a "-delimited or '-delimited literal, with the
// delimiters captured as part of the single group. Capturing the
// delimiters too (rather than splitting into two per-delimiter groups)
// keeps exactly one capture group per logical argument, matching godog's
// positional arg-to-handler-param mapping; unquoteArg strips them back off.
const stringArg = `("[^"]*"|'[^']*')`

// unquoteArg strips the single leading/trailing delimiter byte a stringArg
// match captured. The delimiter is always exactly one ASCII byte, a double
// or single quote, so a plain byte slice is safe.
func unquoteArg(s string) string {
	return s[1 : len(s)-1]
}

func TestConformance(t *testing.T) {
	suite := godog.TestSuite{
		ScenarioInitializer: InitializeScenario,
		Options: &godog.Options{
			Paths:  []string{"../../features"},
			Format: "pretty",
			Strict: true,
		},
	}
	if suite.Run() != 0 {
		t.Fatal("non-zero status returned, failed to run feature tests")
	}
}
