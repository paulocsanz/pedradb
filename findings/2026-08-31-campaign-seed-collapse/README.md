# RFC-0157 campaign seed collapse — every "distinct seed" ran one world

- date: 2026-08-31, found while re-staging the nightly TCP campaign
- severity: evidence-claim bug (fail-closed violation in the harness binary);
  no data corruption, no false-green kernel bit

## Symptom

Every fingerprint row of every registered campaign — 113 rows across
`findings/rfc0157-tcp-campaign/` and `findings/rfc0157-nightly/` — echoes the
same inner world seed:

```
$ grep -rhoE 'seed=[0-9a-f]+' findings/rfc0157-tcp-campaign/ findings/rfc0157-nightly/ | sort | uniq -c
    113 seed=641e28
      2 seed=671        # manual decimal-seed runs (parsed fine)
```

The READMEs tabulate "distinct seeds" (0x0157_C001+, 0x015A_N001..N008) taken
from the *strings passed*, never cross-checked against the *seed echoed back*.

## Mechanism

`cluster_real` parsed its positional seed as:

```rust
t.strip_prefix("0x").map_or_else(
    || t.parse().ok().or_else(|| u64::from_str_radix(t, 16).ok()),
    |h| u64::from_str_radix(h, 16).ok(),
).unwrap_or(0x0064_1E28)   // silent fallback = 0x641e28
```

Campaign seeds are mnemonics (`0x015A_N` + `%02x`): `"015A_N01"` fails
`from_str_radix` (`_` and `N` are not hex digits), every seed fell to the
silent default. The RFC-0156-era seeds named in the campaign script comment
(`0x0156_1E28` etc.) fail for the same reason. The only numeric seeds ever
exercised are 0x641e28 (the default) and decimal 671.

## What it did and did not break

- Each run was still a REAL 3-process TCP cluster (kill/restart/leave/remove
  legs genuinely exercised); per-run checks were real, and dirs are
  PID-qualified so the 8 same-world clusters never trampled each other.
- What is false: the *diversity* claim. "K=8 seeds" was 8 simultaneous copies
  of one seeded world (same KV `l28-641e28`/`v-641e28`, same cluster id, same
  seed-derived kill choice), differing only by port, stagger and OS timing.
  The `s=117..120` spread across rows is timing nondeterminism, not seed
  diversity.
- R-swarm-real is unaffected as a residual (K seeds was never ∀ TCP); the
  multiplier's evidence value was overstated, not its claim class.

## Fix (same day)

1. `cluster_real`: strict `parse_seed` — an unparseable seed now refuses to
   run (`exit 2`, message pointing here) instead of silently picking a world.
   Unit tests pin both the accepted forms and the historical mnemonic strings
   (all must be `None`).
2. `scripts/rfc0157_tcp_campaign.sh`: `RFC0157_SEED_PREFIX` is now a numeric
   u64 base (default `0x157c0`; tonight `0x15b00` → seeds `0x15b01..0x15b08`),
   validated by regex — and each attempt cross-checks the fingerprint's
   echoed `seed=` against the requested seed; a mismatch fails the attempt
   ("SEED COLLAPSE" in the seed log) instead of registering a row.
3. RFC-0157 P0.3/P2.3 status lines, table rows, and the "nenhuma seed
   reutilizada" acceptance bullet record the correction.

Verified: unit tests (mnemonics → `None`); live `cluster_real 0x015A_N01`
refuses with exit 2; live full `--remove-member` run with `0x15b001` echoes
`seed=15b001`. A registered nightly with genuinely distinct worlds is pending
a clean tree.

## Reading

- The collapse was visible in the published evidence all along — the
  fingerprints did not lie; the tabulation never looked at them.
- Root cause class: same as the aeneas stamp drift and the clone gaps —
  a silent default where a refusal belonged.
