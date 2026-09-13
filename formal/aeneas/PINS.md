# Toolchain pins (Aeneas / Charon / Lean)

Recorded 2026-08-15. Empty `rev` means **not installed on this host**.
Fill the hashes the day `./scripts/aeneas_vote.sh` first succeeds, from
the clones that produced the `.llbc` / Lean.

| Tool | Source | How to pin | rev (this host, 2026-08-15) |
|------|--------|------------|------------------------------|
| Aeneas | `$(AENEAS_CHECKOUT)` → `~/.local/bin/aeneas` | `aeneas -version` / git HEAD | `daa85d7e89400fa978be83fedbc7e475a83f0889` |
| Charon | `aeneas/charon` (`charon-pin`) → `~/.local/bin/charon` | `charon version` | `0.1.232` / git `340b1af4df92608d0911fc2ba26eef3fd3a30ab4` |
| Lean | `formal/aeneas/lean/lean-toolchain` | `elan toolchain install leanprover/lean4:v4.31.0` | `leanprover/lean4:v4.31.0` (`lake build Vote` green 2026-08-15) |
| rustc for Charon | `charon toolchain-version` | rustup channel | `nightly-2026-06-01` |

Install (upstream README, 2026):

```
opam switch create 5.3.0
# opam install … (see Aeneas README)
git clone https://github.com/AeneasVerif/aeneas
cd aeneas && make setup-charon && make
# binaries: aeneas/bin/aeneas  and  aeneas/charon/bin/charon
```

Nix (no local pin until a successful run):

```
nix run github:aeneasverif/aeneas#charon -L -- cargo --preset=aeneas
```

`lake build Vote` on this host (2026-08-15) accepted `vote_decision_matches_spec`.
CI without elan still must not say “Lean proved vote” unless `./scripts/lean_vote.sh --required` is green.
