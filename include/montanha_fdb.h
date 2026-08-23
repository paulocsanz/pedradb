/**
 * Montanha FDB-shaped C ABI (RFC-0023 P2.2) — in-process product face.
 *
 * Build: cargo build -p pedradb-capi
 * Link:  libpedradb_capi (cdylib / staticlib / rlib).
 * Gate:  bash scripts/capi-asan.sh
 *        PASS = honest C + rotten handles + oversize → LIMIT
 *        malicious key/value/path = ASan-red (required)
 *
 * Handles are opaque IDs (slot + generation packed into the pointer), not
 * heap pointers. Double-free and use-after-destroy return NULL / ERROR;
 * they are not allocator UB. Tables are thread-local: a handle used on
 * another thread returns NULL / ERROR (not a data race). Do not share
 * handles across threads.
 *
 * Length caps (reject without unbounded read):
 *   path: first 4096 bytes must contain a NUL (else create → NULL)
 *   key_len   ≤ 10MiB  (MAX_TX_BYTES)     → else LIMIT
 *   value_len ≤ 100KiB (MAX_VALUE_BYTES)  → else LIMIT
 *
 * C contract (UB if lied about, under the cap): `path` bytes are readable
 * until NUL; `key`/`value` are readable for the given (capped) lengths.
 * Wild pointers are not validatable. The malicious ASan binary is the
 * regression oracle for a short-buffer lie, not a runtime validator.
 *
 * This is NOT the FoundationDB client / fdbcli / libfdb_c.
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

#define MONTAHA_FDB_MAX_PATH_BYTES 4096
#define MONTAHA_FDB_MAX_KEY_BYTES (10 * 1024 * 1024)
#define MONTAHA_FDB_MAX_VALUE_BYTES (100 * 1024)

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
