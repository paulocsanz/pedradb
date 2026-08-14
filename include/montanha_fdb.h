/**
 * Montanha FDB-shaped C ABI (RFC-0023 P2.2) — plug/test only.
 *
 * Build Rust with: cargo build -p pedradb-store --features c-api
 * Link the resulting libpedradb_store (or cdylib if configured).
 *
 * This is NOT the FoundationDB client library / fdbcli.
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

/** Consumes tr (do not destroy after). */
int montanha_fdb_transaction_commit(MontanhaFdbDatabase *db, MontanhaFdbTransaction *tr);

void montanha_fdb_free(uint8_t *p, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* MONTAHA_FDB_H */
