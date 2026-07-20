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
	c.parsedTag, c.parseErr = tagma.ParseTag(input)
	return nil
}

// When the query {string} is compiled
func (c *conformanceState) theQueryIsCompiled(input string) error {
	c.compiledPostfix, c.compileErr = tagma.Compile(input)
	return nil
}

// When the query {string} is run
func (c *conformanceState) theQueryIsRun(input string) error {
	c.matchResult, c.matchErr = c.idx.Query(input)
	return nil
}

// When the postfix query {string} is run
func (c *conformanceState) thePostfixQueryIsRun(input string) error {
	c.matchResult, c.matchErr = c.idx.QueryPostfix(input)
	return nil
}

// Then it parses with namespace {string}, key {string}, value {string}
func (c *conformanceState) itParsesWithNamespaceKeyValue(ns, key, val string) error {
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

	sc.Given(`^an item "([^"]*)" tagged "([^"]*)"$`, state.anItemTagged)
	sc.When(`^the tag "([^"]*)" is parsed$`, state.theTagIsParsed)
	sc.When(`^the query "([^"]*)" is compiled$`, state.theQueryIsCompiled)
	sc.When(`^the query "([^"]*)" is run$`, state.theQueryIsRun)
	sc.When(`^the postfix query "([^"]*)" is run$`, state.thePostfixQueryIsRun)
	sc.Then(`^it parses with namespace "([^"]*)", key "([^"]*)", value "([^"]*)"$`, state.itParsesWithNamespaceKeyValue)
	sc.Then(`^parsing fails$`, state.parsingFails)
	sc.Then(`^the postfix is "([^"]*)"$`, state.thePostfixIs)
	sc.Then(`^compilation fails$`, state.compilationFails)
	sc.Then(`^it matches exactly "([^"]*)"$`, state.itMatchesExactly)
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
