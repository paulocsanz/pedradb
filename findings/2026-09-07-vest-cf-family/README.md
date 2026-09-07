# Vest: verified parse/serialize of `cf\0user` (CF isolation)

**Date:** 2026-09-07
**Primary source:** Cai, Singh, Lin, Bosamiya, Gancher, Surbatovich, Parno, *Vest: Verified, Secure, High-Performance Parsing and Serialization for Rust*, USENIX Security 2025. PDF: `usenixsecurity25-cai-yi.pdf`.

## What the paper actually says

Vest generates Verus-verified parsers and serializers from an RFC-like DSL. The security theorems include **non-ambiguity** and **non-malleability**: a byte string must not parse as two different messages. Case studies: Bitcoin blocks, TLS 1.3 handshake, WebAssembly. Vest is not Pedra's CF codec; it is the class of guarantee for a binary prefix format.

## Used this turn

Catalog pair `cf_family` (`data_fate`, entry `key_in_cf_family`): a `lock\0k` encoding is not in family `default`. AS-IS admits every key (scan leak). Production `cf_kernel.rs` is now the Verus term (`single_artifact`). Lemma `lemma_as_is_admits_foreign_cf` is the second possibility (ambiguous parse). Other pairs on the file (`encode_cf_key`, `decode_cf_key`, …) keep their twins until their turn.

## Not claimed

Vest combinators in Pedra. Dump of `db.rs` CF scan glue. “somos seL4”. Non-malleability of the whole SST.
