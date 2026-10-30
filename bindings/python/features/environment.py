"""behave environment hooks (PLAN.md §11, Y2).

Each scenario gets a fresh `tagma.Index` on `context.index`, matching the
Background's `Given an item ... tagged ...` steps starting from an empty
index per scenario.
"""

import tagma


def before_scenario(context, scenario):
    context.index = tagma.Index()
    context.findings = None  # the last validate or check call's findings
    context.given_tags = []  # every tag a Given step wrote, in order
