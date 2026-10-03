/* Well-behaved C caller + rotten handles. Must PASS under ASan.
 *
 * Covers: set/get/commit, stale/double-destroy, oversize lens → LIMIT
 * (F215, no unbounded read), 4096-byte no-NUL path → NULL.
 *
 * Does NOT cover a caller that lies about a *capped* length — that is
 * `capi_asan_malicious.c` (expected ASan FAIL).
 *
 * Build/run: bash scripts/capi-asan.sh
 */
#include "montanha_fdb.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int fails;

#define CHECK(cond, msg)                                                                          \
    do {                                                                                          \
        if (!(cond)) {                                                                            \
            fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, (msg));                       \
            fails++;                                                                              \
        }                                                                                         \
    } while (0)

static char *make_dir(void) {
    char *dir = malloc(128);
    if (!dir) {
        return NULL;
    }
    snprintf(dir, 128, "/tmp/montanha-fdb-c-asan-%d-%u", (int)getpid(),
             (unsigned)rand());
    if (mkdir(dir, 0700) != 0) {
        free(dir);
        return NULL;
    }
    return dir;
}

static int test_roundtrip(void) {
    char *dir = make_dir();
    CHECK(dir, "mkdtemp");
    if (!dir) {
        return -1;
    }

    MontanhaFdbDatabase *db = montanha_fdb_database_create(dir, 3, 1);
    CHECK(db != NULL, "database_create");
    if (!db) {
        free(dir);
        return -1;
    }

    MontanhaFdbTransaction *tr = montanha_fdb_transaction_create(db);
    CHECK(tr != NULL, "transaction_create");
    const uint8_t key[] = {'c', '/', 'k'};
    const uint8_t val[] = {'v', '1'};
    CHECK(montanha_fdb_transaction_set(tr, key, sizeof key, val, sizeof val) == MONTAHA_FDB_OK,
          "set");
    CHECK(montanha_fdb_transaction_commit(db, tr) == MONTAHA_FDB_OK, "commit");

    MontanhaFdbTransaction *tr2 = montanha_fdb_transaction_create(db);
    uint8_t *out = NULL;
    size_t len = 0;
    CHECK(montanha_fdb_transaction_get(db, tr2, key, sizeof key, &out, &len) == MONTAHA_FDB_OK,
          "get");
    CHECK(len == 2 && out && memcmp(out, "v1", 2) == 0, "get bytes");
    montanha_fdb_free(out, len);
    montanha_fdb_transaction_destroy(tr2);
    montanha_fdb_database_destroy(db);
    free(dir);
    return 0;
}

static int test_stale_handles(void) {
    char *dir = make_dir();
    CHECK(dir, "mkdtemp");
    if (!dir) {
        return -1;
    }
    MontanhaFdbDatabase *db = montanha_fdb_database_create(dir, 3, 1);
    CHECK(db != NULL, "create");
    MontanhaFdbTransaction *tr = montanha_fdb_transaction_create(db);
    montanha_fdb_database_destroy(db);
    montanha_fdb_database_destroy(db); /* double destroy is a no-op */
    const uint8_t k = 'k';
    const uint8_t v = 'v';
    CHECK(montanha_fdb_transaction_set(tr, &k, 1, &v, 1) == MONTAHA_FDB_ERROR,
          "set after db destroy");
    montanha_fdb_transaction_destroy(tr);
    montanha_fdb_transaction_destroy(tr);
    CHECK(montanha_fdb_transaction_create(db) == NULL, "create tx on stale db");
    montanha_fdb_free(NULL, 0);
    montanha_fdb_free((uint8_t *)(uintptr_t)0x1, 4);
    free(dir);
    return 0;
}

static int test_oversize_is_limit(void) {
    /* Dangling pointer + huge len: LIMIT, no ASan (F215). */
    uint8_t *dangling = (uint8_t *)(uintptr_t)0x1;
    int rc = montanha_fdb_transaction_set(NULL, dangling, (size_t)MONTAHA_FDB_MAX_KEY_BYTES + 1,
                                          dangling, 1);
    CHECK(rc == MONTAHA_FDB_LIMIT, "oversize key_len");
    if (rc == MONTAHA_FDB_LIMIT) {
        fprintf(stderr, "capi_asan: LIMIT key\n");
    }
    rc = montanha_fdb_transaction_set(NULL, dangling, 1, dangling,
                                      (size_t)MONTAHA_FDB_MAX_VALUE_BYTES + 1);
    CHECK(rc == MONTAHA_FDB_LIMIT, "oversize value_len");
    if (rc == MONTAHA_FDB_LIMIT) {
        fprintf(stderr, "capi_asan: LIMIT value\n");
    }
    uint8_t *out = (uint8_t *)(uintptr_t)0xdead;
    size_t len = 7;
    rc = montanha_fdb_transaction_get(NULL, NULL, dangling, (size_t)MONTAHA_FDB_MAX_KEY_BYTES + 1,
                                      &out, &len);
    CHECK(rc == MONTAHA_FDB_LIMIT, "oversize get key_len");
    CHECK(out == NULL && len == 0, "get oversize zeros outputs");
    if (rc == MONTAHA_FDB_LIMIT) {
        fprintf(stderr, "capi_asan: LIMIT get\n");
    }
    return 0;
}

/* RFC-0075 P1.2: same LIMIT on a live create+tx (not a null handle). */
static int test_live_oversize_is_limit(void) {
    char *dir = make_dir();
    CHECK(dir, "mkdtemp");
    if (!dir) {
        return -1;
    }
    MontanhaFdbDatabase *db = montanha_fdb_database_create(dir, 3, 1);
    CHECK(db != NULL, "live database_create");
    if (!db) {
        free(dir);
        return -1;
    }
    MontanhaFdbTransaction *tr = montanha_fdb_transaction_create(db);
    CHECK(tr != NULL, "live transaction_create");
    uint8_t tiny = 1;
    int rc = montanha_fdb_transaction_set(tr, &tiny, (size_t)MONTAHA_FDB_MAX_KEY_BYTES + 1, &tiny,
                                          1);
    CHECK(rc == MONTAHA_FDB_LIMIT, "live oversize key_len");
    if (rc == MONTAHA_FDB_LIMIT) {
        fprintf(stderr, "capi_asan: LIMIT live-key\n");
    }
    montanha_fdb_transaction_destroy(tr);
    montanha_fdb_database_destroy(db);
    free(dir);
    return 0;
}

static int test_path_no_nul_in_window(void) {
    char *buf = malloc(MONTAHA_FDB_MAX_PATH_BYTES);
    CHECK(buf, "malloc path window");
    if (!buf) {
        return -1;
    }
    memset(buf, 'a', MONTAHA_FDB_MAX_PATH_BYTES);
    CHECK(montanha_fdb_database_create(buf, 3, 1) == NULL, "no-NUL 4096 path");
    free(buf);
    return 0;
}

int main(void) {
    test_roundtrip();
    test_stale_handles();
    test_oversize_is_limit();
    test_live_oversize_is_limit();
    test_path_no_nul_in_window();
    if (fails) {
        fprintf(stderr, "capi_asan: %d check(s) failed\n", fails);
        return 1;
    }
    fprintf(stderr, "capi_asan: PASS\n");
    return 0;
}
