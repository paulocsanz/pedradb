/**
 * Montanha FDB-shaped C ABI (RFC-0023 P2.2) — plug/test only.
 *
 * Build: cargo build -p pedradb-capi
 * Link:  libpedradb_capi (cdylib / staticlib / rlib).
 *
 * Handles are opaque IDs (slot + generation packed into the pointer), not
 * heap pointers. Double-free and use-after-destroy return NULL / ERROR;
 * they are not allocator UB. Tables are thread-local: a handle used on
 * another thread returns NULL / ERROR (not a data race). Do not share
 * handles across threads.
 *
 * Still required of the C caller (inherent): `path` is a NUL-terminated
 * string; `key`/`value` are readable for the given lengths.
 *
 * This is NOT a supported product ABI and NOT the FoundationDB client /
 * fdbcli.
 */
#ifndef MONTAHA_FDB_H
#define MONTAHA_FDB_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MONTAHA_FDB_OK 0
#define MONTAHA_FDB_NOT_COMMITTED 1
#define MONTAHA_FDB_TOO_OLD 2
#define MONTAHA_FDB_LIMIT 3
#define MONTAHA_FDB_UNAVAILABLE 4
#define MONTAHA_FDB_ERROR 5

typedef struct MontanhaFdbDatabase MontanhaFdbDatabase;
typedef struct MontanhaFdbTransaction MontanhaFdbTransaction;

/** Open in-process cluster; n_nodes/n_ranges >= 1. NULL on failure. */
MontanhaFdbDatabase *montanha_fdb_database_create(const char *path, uint64_t n_nodes,
                                                  uint64_t n_ranges);
void montanha_fdb_database_destroy(MontanhaFdbDatabase *db);

MontanhaFdbTransaction *montanha_fdb_transaction_create(MontanhaFdbDatabase *db);
void montanha_fdb_transaction_destroy(MontanhaFdbTransaction *tr);

int montanha_fdb_transaction_set(MontanhaFdbTransaction *tr, const uint8_t *key, size_t key_len,
                                 const uint8_t *value, size_t value_len);

/** out_ptr set to buffer; free with montanha_fdb_free(p, len). */
int montanha_fdb_transaction_get(MontanhaFdbDatabase *db, MontanhaFdbTransaction *tr,
                                 const uint8_t *key, size_t key_len, uint8_t **out_ptr,
                                 size_t *out_len);

/** Consumes tr (destroy afterwards is a no-op). */
int montanha_fdb_transaction_commit(MontanhaFdbDatabase *db, MontanhaFdbTransaction *tr);

void montanha_fdb_free(uint8_t *p, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* MONTAHA_FDB_H */
