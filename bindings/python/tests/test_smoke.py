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


def test_index_add_invalid_raises():
    idx = tagma.Index()
    try:
        idx.add("a =5")
    except ValueError:
        pass
    else:
        raise AssertionError("expected ValueError for invalid tag")


def _run_all():
    tests = [v for k, v in globals().items() if k.startswith("test_") and callable(v)]
    for t in tests:
        t()
        print(f"ok: {t.__name__}")
    print(f"{len(tests)} smoke tests passed")


if __name__ == "__main__":
    _run_all()
