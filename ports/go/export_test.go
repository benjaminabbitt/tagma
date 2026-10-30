package tagma

// SplitUnquotedWhitespace exposes the bulk-ingest line splitter (SPEC.md
// §2 QUOTING) to the external conformance harness only, which needs it for
// the validate step's tag-list arguments (docs/steps.md). A _test.go file,
// so it is not part of the package's public API.
var SplitUnquotedWhitespace = splitUnquotedWhitespace
