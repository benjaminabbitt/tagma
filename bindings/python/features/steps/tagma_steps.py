"""behave step definitions for the frozen tagma step vocabulary
(docs/steps.md / PLAN.md Appendix A). Implements exactly the steps listed
there against the `tagma` Python module — nothing else.

Uses behave's regex step matcher (rather than the default parse-style one)
because the frozen vocabulary's `{string}` placeholder is defined as "a
quoted cucumber-expression string" (docs/steps.md): its value may be
delimited by a matching pair of *either* `"` or `'`, chosen independently
per occurrence in the step text. The QUOTING conformance vectors rely on
this — a fixture that embeds a literal `"` delimits the whole step argument
with `'` instead (see features/tags.feature's "quoted tokens" scenario and
features/matching.feature's quoted-atom scenarios). cucumber-rs and
cucumber-js get this for free from their native {string} expression type;
behave's default parse-style matcher does not, so it's reproduced here by
hand with one regex alternation per placeholder.
"""

import tagma
from behave import given, then, use_step_matcher, when

use_step_matcher("re")


def _qstr(name):
    """One occurrence of the frozen vocabulary's {string}: a value
    delimited by a matching pair of either `"` or `'`, captured into two
    named groups (exactly one of which will match)."""
    return rf'(?:"(?P<{name}_d>[^"]*)"|\'(?P<{name}_s>[^\']*)\')'


def _pick(kwargs, name):
    d = kwargs.pop(f"{name}_d")
    s = kwargs.pop(f"{name}_s")
    return d if d is not None else s


@given(rf"an item {_qstr('item_id')} tagged {_qstr('tags')}")
def step_given_item_tagged(context, **kwargs):
    item_id = _pick(kwargs, "item_id")
    tags = _pick(kwargs, "tags")
    # Same "<id> <tag> <tag>..." line format tagma.Index.add expects;
    # raises ValueError (Python's stand-in for "panics") on an invalid tag.
    context.index.add(f"{item_id} {tags}")


@when(rf"the tag {_qstr('input')} is parsed")
def step_when_tag_is_parsed(context, **kwargs):
    input = _pick(kwargs, "input")
    context.error = None
    context.tag_result = None
    try:
        context.tag_result = tagma.parse_tag(input)
    except ValueError as exc:
        context.error = exc


@when(rf"the query {_qstr('query')} is compiled")
def step_when_query_is_compiled(context, **kwargs):
    query = _pick(kwargs, "query")
    context.error = None
    context.postfix_result = None
    try:
        context.postfix_result = tagma.compile(query)
    except ValueError as exc:
        context.error = exc


@when(rf"the query {_qstr('query')} is run")
def step_when_query_is_run(context, **kwargs):
    query = _pick(kwargs, "query")
    context.error = None
    context.match_result = None
    try:
        context.match_result = context.index.query(query)
    except ValueError as exc:
        context.error = exc


@when(rf"the postfix query {_qstr('query')} is run")
def step_when_postfix_query_is_run(context, **kwargs):
    query = _pick(kwargs, "query")
    context.error = None
    context.match_result = None
    try:
        context.match_result = context.index.query_postfix(query)
    except ValueError as exc:
        context.error = exc


@then(
    rf"it parses with namespace {_qstr('namespace')}, "
    rf"key {_qstr('key')}, value {_qstr('value')}"
)
def step_then_it_parses_with(context, **kwargs):
    namespace = _pick(kwargs, "namespace")
    key = _pick(kwargs, "key")
    value = _pick(kwargs, "value")
    assert context.error is None, f"expected parse to succeed, got {context.error!r}"
    expected = {
        "namespace": namespace or None,
        "key": key,
        "value": value or None,
    }
    assert context.tag_result == expected, f"{context.tag_result!r} != {expected!r}"


@then("parsing fails")
def step_then_parsing_fails(context):
    assert context.error is not None, "expected parsing to fail, it succeeded"


@then(rf"the postfix is {_qstr('postfix')}")
def step_then_the_postfix_is(context, **kwargs):
    postfix = _pick(kwargs, "postfix")
    assert context.error is None, f"expected compile to succeed, got {context.error!r}"
    assert context.postfix_result == postfix, f"{context.postfix_result!r} != {postfix!r}"


@then("compilation fails")
def step_then_compilation_fails(context):
    assert context.error is not None, "expected compilation to fail, it succeeded"


@then(rf"it matches exactly {_qstr('ids')}")
def step_then_it_matches_exactly(context, **kwargs):
    ids = _pick(kwargs, "ids")
    assert context.error is None, f"expected query to succeed, got {context.error!r}"
    expected = sorted(ids.split())
    assert sorted(context.match_result) == expected, (
        f"{sorted(context.match_result)!r} != {expected!r}"
    )
