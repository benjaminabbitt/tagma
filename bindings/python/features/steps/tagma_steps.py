"""behave step definitions for the frozen tagma step vocabulary
(docs/steps.md / PLAN.md Appendix A). Implements exactly the steps listed
there against the `tagma` Python module — nothing else.
"""

import parse
import tagma
from behave import given, register_type, then, when


# behave's default parse-style matcher requires quoted `{name}` captures to
# be non-empty, but several fixtures use `""` to mean "absent"/"empty set"
# (docs/steps.md semantics notes). Register a type that also matches the
# empty string, and stops at the next literal quote so adjacent quoted
# fields in the same step text don't bleed into each other.
@parse.with_pattern(r'[^"]*')
def _quoted(text):
    return text


register_type(Quoted=_quoted)


@given('an item "{item_id:Quoted}" tagged "{tags:Quoted}"')
def step_given_item_tagged(context, item_id, tags):
    # Same "<id> <tag> <tag>..." line format tagma.Index.add expects;
    # raises ValueError (Python's stand-in for "panics") on an invalid tag.
    context.index.add(f"{item_id} {tags}")


@when('the tag "{input:Quoted}" is parsed')
def step_when_tag_is_parsed(context, input):
    context.error = None
    context.tag_result = None
    try:
        context.tag_result = tagma.parse_tag(input)
    except ValueError as exc:
        context.error = exc


@when('the query "{query:Quoted}" is compiled')
def step_when_query_is_compiled(context, query):
    context.error = None
    context.postfix_result = None
    try:
        context.postfix_result = tagma.compile(query)
    except ValueError as exc:
        context.error = exc


@when('the query "{query:Quoted}" is run')
def step_when_query_is_run(context, query):
    context.error = None
    context.match_result = None
    try:
        context.match_result = context.index.query(query)
    except ValueError as exc:
        context.error = exc


@when('the postfix query "{query:Quoted}" is run')
def step_when_postfix_query_is_run(context, query):
    context.error = None
    context.match_result = None
    try:
        context.match_result = context.index.query_postfix(query)
    except ValueError as exc:
        context.error = exc


@then('it parses with namespace "{namespace:Quoted}", key "{key:Quoted}", value "{value:Quoted}"')
def step_then_it_parses_with(context, namespace, key, value):
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


@then('the postfix is "{postfix:Quoted}"')
def step_then_the_postfix_is(context, postfix):
    assert context.error is None, f"expected compile to succeed, got {context.error!r}"
    assert context.postfix_result == postfix, f"{context.postfix_result!r} != {postfix!r}"


@then("compilation fails")
def step_then_compilation_fails(context):
    assert context.error is not None, "expected compilation to fail, it succeeded"


@then('it matches exactly "{ids:Quoted}"')
def step_then_it_matches_exactly(context, ids):
    assert context.error is None, f"expected query to succeed, got {context.error!r}"
    expected = sorted(ids.split())
    assert sorted(context.match_result) == expected, (
        f"{sorted(context.match_result)!r} != {expected!r}"
    )
