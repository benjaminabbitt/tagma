/* C smoke test for the tagma C ABI (PLAN.md §8, task C2). Builds an index
 * from the PLAN.md Appendix B.4 fixture, runs a query, exercises
 * tagma_compile plus its error path, and frees everything. Exits 0 on
 * success, non-zero (via assert) otherwise. */

#include <assert.h>
#include <string.h>

#include "tagma.h"

int main(void) {
    void *idx = tagma_index_new();
    assert(idx != NULL);

    assert(tagma_index_add(
               idx, "a urgent lang=en lang=fr range=5 geo:lat=57.64 status=done") == 0);
    assert(tagma_index_add(idx, "b range=tbd lang=en prio:urgent due=2026-08-01") == 0);
    assert(tagma_index_add(idx, "c urgent=false score=-3 note") == 0);

    char *result = tagma_query(idx, "urgent and not status=done");
    assert(result != NULL);
    assert(strcmp(result, "c") == 0);
    tagma_str_free(result);

    char *postfix = tagma_compile("a or b and c");
    assert(postfix != NULL);
    assert(strcmp(postfix, "a/b/c/and/or") == 0);
    tagma_str_free(postfix);

    char *bad = tagma_compile("a and");
    assert(bad == NULL);
    char *err = tagma_last_error();
    assert(err != NULL);
    assert(strlen(err) > 0);
    tagma_str_free(err);

    tagma_index_free(idx);

    return 0;
}
