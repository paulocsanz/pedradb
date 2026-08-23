# RFC-0021 P2.3 — Security / TLS baseline

**Status:** lab implementation shipped (RFC-0050 P0.5); not production default  
**Parent:** [0021](0021-montanha-fdb-tikv-parity-gaps.md)
**Updated:** 2026-08-23

## Baseline (lab → prod profile)

| Surface | Lab today | Prod profile (required before GA claim) |
|---------|-----------|----------------------------------------|
| Peer Raft TCP | Cleartext MTCP | TLS 1.3 mutual auth (mTLS) or private mesh only |
| Client TCP | Cleartext | TLS + client auth token/mTLS |
| Health HTTP | Cleartext :9780 | Localhost-only or TLS |
| Disk | Host FS | Encrypt at rest (volume/LUKS) ops-owned |

## Implementation (RFC-0050 P0.5)

- Feature `tls` on `pedradb-store` (`rustls` 0.21).
- `montanha-tcp --tls-cert --tls-key --tls-ca [--require-tls] [--tls-server-name localhost]`.
- mTLS when CA is present (the only supported TLS profile).
- Health HTTP defaults to `127.0.0.1` (`bind_port+79`).
- Cleartext remains the lab default when flags are omitted.

## Non-goals here

- Claiming secure-by-default / GA.  
- In-process key rotation (restart + replace PEMs).  
- Encrypt-at-rest in the engine (LUKS/volume, ops).

## Gate

- [x] CI profile `montanha-secure` builds with TLS features  
- [x] Doc + runbook for key rotation (manual restart)  

Without `--tls-*` / `--require-tls`: **cleartext lab only**.
