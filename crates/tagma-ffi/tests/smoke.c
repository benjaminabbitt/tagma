/* C smoke test for the tagma C ABI (PLAN.md §8, task C2). Builds an index
 * from the PLAN.md Appendix B.4 fixture, runs a query, exercises
 * tagma_compile plus its error path, and frees everything. Exits 0 on
 * success, non-zero (via assert) otherwise. */

#include <assert.h>
#include <string.h>

#include "tagma.h"

/* Drains tagma_last_error() and asserts it mentions `needle`. */
static void assert_error_contains(const char *needle) {
    char *err = tagma_last_error();
    assert(err != NULL);
    assert(strstr(err, needle) != NULL);
    tagma_str_free(err);
}

/* Handle safety (task kooky-snub), exercised at the real ABI: a freed,
 * reused, or never-issued handle must be a defined error on every entry
 * point that takes one -- not undefined behaviour. */
static void handle_safety(void) {
    TagmaIndex h = tagma_index_new();
    assert(h != 0);

    /* Double free. */
    assert(tagma_index_free(h) == 0);
    assert(tagma_index_free(h) == -1);
    assert_error_contains("freed index");

    /* Use after free, on every handle-taking entry point. */
    assert(tagma_index_add(h, "a urgent") == -1);
    assert_error_contains("freed index");
    assert(tagma_query(h, "urgent") == NULL);
    assert_error_contains("freed index");
    assert(tagma_query_postfix(h, "urgent") == NULL);
    assert_error_contains("freed index");

    /* Never issued by tagma: garbage integers and a real address, which is
     * what a handle used to be. */
    int stack_object = 0;
    TagmaIndex foreign[] = {1, 42, 0xDEADBEEF, (TagmaIndex)(uintptr_t)&stack_object};
    for (size_t i = 0; i < sizeof foreign / sizeof foreign[0]; i++) {
        assert(tagma_index_free(foreign[i]) == -1);
        assert_error_contains("not issued by tagma");
        assert(tagma_index_add(foreign[i], "a urgent") == -1);
        assert_error_contains("not issued by tagma");
        assert(tagma_query(foreign[i], "urgent") == NULL);
        assert_error_contains("not issued by tagma");
    }

    /* Generation reuse: the freed slot comes back, the stale handle does
     * not. This is the case a slot table without a generation counter gets
     * wrong -- the stale handle would silently address the new index. */
    TagmaIndex reused = tagma_index_new();
    assert(reused != h);
    assert(tagma_index_add(reused, "fresh urgent") == 0);

    assert(tagma_index_add(h, "stale urgent") == -1);
    assert_error_contains("freed index");

    char *only_fresh = tagma_query(reused, "urgent");
    assert(only_fresh != NULL);
    assert(strcmp(only_fresh, "fresh") == 0); /* the stale write went nowhere */
    tagma_str_free(only_fresh);

    assert(tagma_index_free(reused) == 0);

    /* A zero handle frees as a no-op, like free(NULL). */
    assert(tagma_index_free(0) == 0);
}

int main(void) {
    TagmaIndex idx = tagma_index_new();
    assert(idx != 0);

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

    /* Panic safety (task tasty-snub): caller-controlled invalid input must
     * produce a defined error return, not a Rust panic unwinding across the
     * ABI. Non-UTF-8 bytes are the case any C caller can trigger today; if
     * this ever regresses the process aborts here rather than asserting. */
    const char *not_utf8 = "a\xff\xfe" "b";

    assert(tagma_index_add(idx, not_utf8) == -1);
    assert(tagma_query(idx, not_utf8) == NULL);
    assert(tagma_query_postfix(idx, not_utf8) == NULL);
    assert(tagma_compile(not_utf8) == NULL);

    char *utf8_err = tagma_last_error();
    assert(utf8_err != NULL);
    assert(strstr(utf8_err, "UTF-8") != NULL);
    tagma_str_free(utf8_err);

    /* Null pointers and null handles, at the ABI rather than in Rust tests. */
    assert(tagma_index_add(idx, NULL) == -1);
    assert(tagma_query(idx, NULL) == NULL);
    assert(tagma_query_postfix(idx, NULL) == NULL);
    assert(tagma_compile(NULL) == NULL);
    assert(tagma_index_add(0, "a urgent") == -1);
    assert(tagma_query(0, "urgent") == NULL);
    assert(tagma_query_postfix(0, "urgent") == NULL);
    assert(tagma_index_free(0) == 0);
    tagma_str_free(NULL);

    /* An interior NUL cannot reach the library through a C string at all --
     * the argument simply ends early -- so this is a well-formed short query,
     * not an error. The interior-NUL hazard is on the way out, covered by the
     * Rust unit tests over to_c_string. */
    char *truncated = tagma_compile("a or b\0 and c");
    assert(truncated != NULL);
    assert(strcmp(truncated, "a/b/or") == 0);
    tagma_str_free(truncated);

    /* The index is still usable after all of the above. */
    char *still_good = tagma_query(idx, "note");
    assert(still_good != NULL);
    assert(strcmp(still_good, "c") == 0);
    tagma_str_free(still_good);

    assert(tagma_index_free(idx) == 0);

    handle_safety();

    return 0;
}
