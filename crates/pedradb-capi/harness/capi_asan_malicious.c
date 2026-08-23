/* Malicious C caller on slices. Must DIE under ASan (expected FAIL).
 *
 * Cases (argv[1]):
 *   key   — 8-byte heap key, key_len=256 (under the 10MiB cap, past the
 *           allocation). Hits memcpy in marshalling.
 *   value — 8-byte heap value, value_len=256 (under the 100KiB cap).
 *   path  — 8-byte heap path with no NUL. Hits memchr in create.
 *
 * A green run of this binary is a harness bug: ASan must see the lie.
 * Wild pointers remain unvalidatable; this only covers short buffers.
 *
 * Build/run: bash scripts/capi-asan.sh  (expects non-zero + ASan report)
 */
#include "montanha_fdb.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static char *make_dir(void) {
    char *dir = malloc(128);
    if (!dir) {
        return NULL;
    }
    snprintf(dir, 128, "/tmp/montanha-fdb-c-asan-m-%d-%u", (int)getpid(),
             (unsigned)rand());
    if (mkdir(dir, 0700) != 0) {
        free(dir);
        return NULL;
    }
    return dir;
}

static void case_key(void) {
    char *dir = make_dir();
    if (!dir) {
        fprintf(stderr, "mkdtemp failed\n");
        exit(2);
    }
    MontanhaFdbDatabase *db = montanha_fdb_database_create(dir, 3, 1);
    if (!db) {
        fprintf(stderr, "database_create failed\n");
        exit(2);
    }
    MontanhaFdbTransaction *tr = montanha_fdb_transaction_create(db);
    if (!tr) {
        fprintf(stderr, "transaction_create failed\n");
        exit(2);
    }
    uint8_t *key = malloc(8);
    if (!key) {
        exit(2);
    }
    memset(key, 'k', 8);
    const uint8_t v = 'v';
    /* key_len is under MAX_KEY_BYTES so the cap does not save us. */
    (void)montanha_fdb_transaction_set(tr, key, 256, &v, 1);
    fprintf(stderr, "capi_asan_malicious: key lie did not trap\n");
    exit(1);
}

static void case_value(void) {
    char *dir = make_dir();
    if (!dir) {
        exit(2);
    }
    MontanhaFdbDatabase *db = montanha_fdb_database_create(dir, 3, 1);
    MontanhaFdbTransaction *tr = montanha_fdb_transaction_create(db);
    const uint8_t k = 'k';
    uint8_t *val = malloc(8);
    if (!val) {
        exit(2);
    }
    memset(val, 'v', 8);
    (void)montanha_fdb_transaction_set(tr, &k, 1, val, 256);
    fprintf(stderr, "capi_asan_malicious: value lie did not trap\n");
    exit(1);
}

static void case_path(void) {
    /* Heap, not stack: ASan redzones around malloc(8) so memchr(…, 4096)
     * cannot skate across adjacent live stack slots. Darwin ASan intercepts
     * memchr, not strnlen. */
    char *buf = malloc(8);
    if (!buf) {
        exit(2);
    }
    memset(buf, 'p', 8); /* no NUL */
    (void)montanha_fdb_database_create(buf, 3, 1);
    fprintf(stderr, "capi_asan_malicious: path lie did not trap\n");
    exit(1);
}

int main(int argc, char **argv) {
    const char *which = argc > 1 ? argv[1] : "key";
    if (strcmp(which, "key") == 0) {
        case_key();
    } else if (strcmp(which, "value") == 0) {
        case_value();
    } else if (strcmp(which, "path") == 0) {
        case_path();
    } else {
        fprintf(stderr, "usage: capi_asan_malicious [key|value|path]\n");
        return 2;
    }
    return 1;
}
