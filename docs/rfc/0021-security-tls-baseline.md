# RFC-0021 P2.3 — Security / TLS baseline

**Status:** baseline recorded (not production default yet)  
**Parent:** [0021](0021-montanha-fdb-tikv-parity-gaps.md)

## Baseline (lab → prod profile)

| Surface | Lab today | Prod profile (required before GA claim) |
|---------|-----------|----------------------------------------|
| Peer Raft TCP | Cleartext MTCP | TLS 1.3 mutual auth (mTLS) or private mesh only |
| Client TCP | Cleartext | TLS + client auth token/mTLS |
| Health HTTP | Cleartext :9780 | Localhost-only or TLS |
| Disk | Host FS | Encrypt at rest (volume/LUKS) ops-owned |

## Non-goals here

- Implementing rustls stack in this slice (tracked as implementation follow-on).  
- Claiming secure-by-default until peer+client TLS land in code.

## Gate

- [ ] CI profile `montanha-secure` builds with TLS features  
- [ ] Doc + runbook for key rotation  

Until then: **cleartext lab only**.
