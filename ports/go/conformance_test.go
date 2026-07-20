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
	*c = conformanceState{idx: tagma.NewIndex()}
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
