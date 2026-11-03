"""Smoke test for the tagma Python module (PLAN.md §11, Y1).

Dependency-light on purpose: plain asserts, no pytest requirement. Runs
under `python -m pytest` if pytest happens to be installed, but is also a
valid plain script (`python test_smoke.py`) so `just dev-py` can invoke it
without assuming pytest is present.
"""

import tagma


def test_parse_tag_valid():
    t = tagma.parse_tag("geo:lat=57.64")
    assert t == {"namespace": "geo", "key": "lat", "value": "57.64"}

    t = tagma.parse_tag("urgent")
    assert t == {"namespace": None, "key": "urgent", "value": None}


def test_parse_tag_invalid():
    try:
        tagma.parse_tag("=5")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for '=5'")


def test_compile():
    assert tagma.compile("urgent and range>4") == "urgent/range>4/and"

    try:
        tagma.compile("a and")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for 'a and'")


def test_index_add_query():
    idx = tagma.Index()
    idx.add("a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done")
    idx.add("b range=tbd lang=en prio:urgent due=2026-08-01")
    idx.add("c urgent=false score=-3 note")

    assert sorted(idx.query("urgent")) == ["a", "c"]
    assert sorted(idx.query("lang=en")) == ["a", "b"]
    assert idx.query("range>5") == []
    assert sorted(idx.query_postfix("urgent/status=done/not/and")) == ["c"]


GATEWAY_TABLE = [
    "urgent",
    "urgent and range>4",
    "lang=en or note",
    "not status=done",
    "(urgent or note) and not lang=fr",
    "*:urgent",
    "lang=en lang=fr",
    "urgent AND NOT note",
    "nothing",
]


def test_infix_query_is_query_postfix_of_compile():
    """Infix is a gateway (SPEC.md §5): query(q) == query_postfix(compile(q))."""
    idx = tagma.Index()
    idx.add("a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done")
    idx.add("b range=tbd lang=en prio:urgent due=2026-08-01")
    idx.add("c urgent=false score=-3 note")

    distinct = set()
    for q in GATEWAY_TABLE:
        via_infix = idx.query(q)
        assert via_infix == idx.query_postfix(tagma.compile(q)), q
        distinct.add(tuple(via_infix))
    assert len(distinct) > 3, "table must discriminate between results"


def test_index_add_invalid_raises():
    idx = tagma.Index()
    try:
        idx.add("a =5")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for invalid tag")


def _rules_index():
    idx = tagma.Index()
    # A store declaring no permit permits nothing (SPEC.md §10): this one
    # permits every tag, so only the enums judge.
    idx.add('cfg tagma.permit:"*:*" tagma.enum:k=a tagma.enum:j=a')
    idx.add("item k=z j=bad")
    return idx


def test_validate_returns_findings_with_their_origin():
    idx = _rules_index()
    assert idx.validate("item", ["k=y"], ["k=z"]) == [
        {"code": "not-in-enum", "tag": "j=bad", "args": [("allowed", "a")], "origin": "existing"},
        {"code": "not-in-enum", "tag": "k=y", "args": [("allowed", "a")], "origin": "introduced"},
    ]
    assert idx.validate("item", add=["k=a", 'j="a"'], remove=["k=z", "j=bad"]) == []
    # Pure: nothing was written.
    assert idx.query("k=z and j=bad") == ["item"]


def test_validate_change_defaults_to_empty():
    idx = _rules_index()
    assert [f["origin"] for f in idx.validate("item")] == ["existing", "existing"]
    assert [f["tag"] for f in idx.validate("item", remove=["k=z"])] == ["j=bad"]


def test_validate_finding_without_args_has_an_empty_list():
    idx = tagma.Index()
    idx.add("cfg tagma.permit:other")
    assert idx.validate("x", ["lang=en"]) == [
        {"code": "not-permitted", "tag": "lang=en", "args": [], "origin": "introduced"}
    ]


def test_validate_invalid_tag_raises_naming_its_list():
    idx = tagma.Index()
    for kwargs, prefix in (({"add": ["=5"]}, "add: "), ({"remove": ["=5"]}, "remove: ")):
        try:
            idx.validate("x", **kwargs)
        except ValueError as exc:
            assert str(exc).startswith(prefix), exc
        else:
            raise AssertionError(f"expected ValueError for {kwargs}")


def test_check_returns_config_findings_sorted_by_tag():
    idx = tagma.Index()
    assert idx.check() == []
    idx.add("cfg tagma.permit:k tagma.hide:x=false tagma.arity:k=scalar tagma.arity:k=set")
    assert idx.check() == [
        {
            "code": "conflict",
            "tag": "tagma.arity:k=set",
            "args": [("facet", "arity"), ("resolution", "scalar")],
            "origin": "existing",
        },
        {
            "code": "inert-false",
            "tag": "tagma.hide:x=false",
            "args": [("facet", "hide")],
            "origin": "existing",
        },
    ]


def test_rules_from_tags_mirrors_the_index_it_comes_from():
    # Non-config tags among the input are ignored.
    rules = tagma.Rules.from_tags(['tagma.permit:"*:*"', "tagma.enum:k=a", "tagma.enum:j=a", "k=z"])
    via_rules = rules.validate(["k=z", "j=bad"], ["k=y"], ["k=z"])
    assert via_rules == _rules_index().validate("item", ["k=y"], ["k=z"])
    assert len(via_rules) == 2
    assert rules.validate([], add=["k=a"]) == []
    assert [f["tag"] for f in rules.validate(["k=z"])] == ["k=z"]
    assert rules.check() == []
    conflicted = tagma.Rules.from_tags(["tagma.permit:k", "tagma.arity:k=scalar", "tagma.arity:k=set"])
    assert conflicted.check() == [
        {
            "code": "conflict",
            "tag": "tagma.arity:k=set",
            "args": [("facet", "arity"), ("resolution", "scalar")],
            "origin": "existing",
        }
    ]


def test_keys_per_item_through_the_index_and_rules():
    keys = [("namespace", "area"), ("limit", "1"), ("keys", "a"), ("keys", "b"), ("keys", "c")]
    expected = [
        {"code": "too-many-keys", "tag": "area:a", "args": keys, "origin": "existing"},
        {"code": "too-many-keys", "tag": "area:b", "args": keys, "origin": "existing"},
        {"code": "too-many-keys", "tag": "area:c", "args": keys, "origin": "introduced"},
    ]
    idx = tagma.Index()
    idx.add('cfg tagma.permit:"area:*" tagma.keys-per-item:area=1')
    idx.add("item area:a area:b")
    assert idx.validate("item", ["area:c"]) == expected
    rules = tagma.Rules.from_tags(['tagma.permit:"area:*"', "tagma.keys-per-item:area=1"])
    assert rules.validate(["area:a", "area:b"], ["area:c"]) == expected
    conflicted = tagma.Rules.from_tags(
        ['tagma.permit:"area:*"', "tagma.keys-per-item:area=2", "tagma.keys-per-item:area=1"]
    )
    assert conflicted.check() == [
        {
            "code": "conflict",
            "tag": "tagma.keys-per-item:area=2",
            "args": [("facet", "keys-per-item"), ("resolution", "1")],
            "origin": "existing",
        }
    ]


def test_rules_invalid_tag_raises_naming_its_list():
    try:
        tagma.Rules.from_tags(["=5"])
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError from from_tags")
    rules = tagma.Rules.from_tags([])
    for args, kwargs, prefix in (
        ((["=5"],), {}, "current: "),
        (([],), {"add": ["=5"]}, "add: "),
        (([],), {"remove": ["=5"]}, "remove: "),
    ):
        try:
            rules.validate(*args, **kwargs)
        except ValueError as exc:
            assert str(exc).startswith(prefix), exc
        else:
            raise AssertionError(f"expected ValueError for {prefix}")


def _run_all():
    tests = [v for k, v in globals().items() if k.startswith("test_") and callable(v)]
    for t in tests:
        t()
        print(f"ok: {t.__name__}")
    print(f"{len(tests)} smoke tests passed")


if __name__ == "__main__":
    _run_all()
