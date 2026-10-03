# pedradb-world (FDB-parity P1 + P2)

Deterministic **World** runtime over Montanha-Store + **Net** + **per-peer disk** + **logical clock**.

> Not full FDB Simulation — see [`../DST-VS-FDB-SIM.md`](../DST-VS-FDB-SIM.md)  
> and [`../FDB-PARITY-ROADMAP.md`](../FDB-PARITY-ROADMAP.md).

## What this is

| Piece | Role |
|-------|------|
| `net::Net` / `InProcessNet` | send/poll/tick; drop; delay; bipartition |
| `schedule::Action` | clock, put/get/DCS, part/heal, remove/add member, disk arm, net |
| `World::run(seed)` | `StoreCluster<FailingEnv>` Queued RPC + Net; stable `trace_hash` |

Raft AE/RV/**InstallSnapshot** are **`PeerMsg`** bytes through Net.

## Run

```bash
cd world
cargo test
cargo run --release --bin world_smoke -- 42
../scripts/world_campaign.sh

# P3 soak (UCB1 over schedule arms: disk/membership/dcs_ttl/…)
cargo run --release --bin world_soak -- 32 1 12
../scripts/world_soak.sh 32 1 12

# P3.5 metamorphic compact (harness)
cd ../harness && cargo run --release --bin dst_metamorphic_compact -- 42 80
```

## Replay

```rust
pedradb_world::assert_seed_replayable(seed, cfg)?;
```

## Store seams

| API | Role |
|-----|------|
| `RpcMode::{Direct,Queued}` | unit tests vs World |
| `PeerMsg` | shared codec (P2.1) |
| `advance_time` / `logical_now` | P1.5 |
| `remove_member` / `add_member` | P2.2 |
| `InstallSnapshot` | P2.3 catch-up |
| `open_with_envs_rng` | per-peer Env |

## Next (P3)

Coverage-guided schedules, soak CI — not claimed done here.
