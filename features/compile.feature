Feature: Infix query compilation

  The infix query frontend compiles to the postfix wire form. Precedence is
  not > and > or; parentheses override precedence. Fixtures transcribed
  verbatim from PLAN.md Appendix B.2 and B.3.

  Scenario Outline: compiling infix to postfix
    Step arguments are single-quote-delimited (the other legal {string}
    delimiter) so the quoted-atom rows below can embed literal `"`
    characters with no escaping.
    When the query '<infix>' is compiled
    Then the postfix is '<postfix>'

    Examples:
      | infix                          | postfix                       |
      | urgent                         | urgent                        |
      | urgent and range>4             | urgent/range>4/and            |
      | a or b and c                   | a/b/c/and/or                  |
      | (a or b) and c                 | a/b/or/c/and                  |
      | not a and b                    | a/not/b/and                   |
      | not (a and b)                  | a/b/and/not                   |
      | not not a                      | a/not/not                     |
      | a and b and c                  | a/b/and/c/and                 |
      | *:lang=en and not status=done  | *:lang=en/status=done/not/and |
      | *                              | *                              |
      | and=*                          | and=*                         |
      | note="hello world"             | note="hello world"            |
      | "a:b"=c and x                  | "a:b"=c/x/and                 |
      | urgent AND range>4             | urgent/range>4/and            |
      | urgent And not status=done     | urgent/status=done/not/and    |
      | a OR b And c                   | a/b/c/and/or                  |
      | a b                            | a/b/and                       |
      | a b c                          | a/b/and/c/and                 |
      | a (b or c)                     | a/b/c/or/and                  |
      | not a b                        | a/not/b/and                   |

  Scenario Outline: compilation failures
    Step arguments are single-quote-delimited so the unterminated-quote row
    below can embed a literal `"` with no escaping.
    When the query '<infix>' is compiled
    Then compilation fails

    Examples:
      | infix   |
      | a and   |
      | and a   |
      | (a      |
      | a )     |
      | a & b   |
      | not     |
      | a=* or  |
      | note="unterminated |
