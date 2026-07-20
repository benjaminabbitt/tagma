Feature: Tag parsing

  A tag is a triple (namespace?, key, value?) written as
  `namespace:key=value`, with namespace and value independently optional.
  Fixtures transcribed verbatim from PLAN.md Appendix B.1.

  Scenario Outline: valid tags
    Reserved words ("and", "or", "not") are reserved on the query side only
    — as a tag key (see the "and" row below) they parse like any other
    token. A blank namespace or value cell means that component is absent
    from the parsed tag.
    When the tag "<input>" is parsed
    Then it parses with namespace "<namespace>", key "<key>", value "<value>"

    Examples:
      | input                 | namespace | key     | value          |
      | urgent                |           | urgent  |                |
      | range=5               |           | range   | 5              |
      | geo:lat=57.64         | geo       | lat     | 57.64          |
      | geo:lat               | geo       | lat     |                |
      | temp=-5               |           | temp    | -5             |
      | version=2.0.0-rc1     |           | version | 2.0.0-rc1      |
      | and                   |           | and     |                |
      | due=2026-08-01        |           | due     | 2026-08-01     |

  Scenario Outline: quoted tokens (QUOTING extension, SPEC.md §2)
    A `"`-quoted token is legal in the namespace, key, or value position.
    Quoting is syntax, not data: the canonical, stored value is always the
    decoded content, so a quoted spelling that didn't need quoting (e.g.
    "3.5") parses identically to its bare spelling. `""` inside the quotes
    escapes one literal `"`. (Table cells here embed literal `"` characters,
    so the step arguments below are single-quote-delimited — the same
    {string} cucumber-expression type, just the other legal delimiter.)
    When the tag '<input>' is parsed
    Then it parses with namespace '<namespace>', key '<key>', value '<value>'

    Examples:
      | input                       | namespace | key  | value                 |
      | due="2026-08-01T10:00:00"   |           | due  | 2026-08-01T10:00:00   |
      | note="hello world"          |           | note | hello world           |
      | "a:b"=c                     |           | a:b  | c                     |
      | x="3.5"                     |           | x    | 3.5                   |
      | x="say ""hi"""              |           | x    | say "hi"              |

  Scenario Outline: invalid tags
    Includes the empty-string input, which must also fail to parse. The
    step arguments are single-quote-delimited so the unterminated-quote
    row below can embed a literal `"` with no escaping.
    When the tag '<input>' is parsed
    Then parsing fails

    Examples:
      | input       |
      | =5          |
      | :key        |
      | ns:         |
      | key=        |
      | *           |
      | ns:*=5      |
      | key=+       |
      | -key        |
      | .key        |
      | a b         |
      | a=b=c       |
      | a:b:c       |
      | key=va~lue  |
      |             |
      | x="abc      |
