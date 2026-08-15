# sim-volume-dst-20260815 — continuous DST soak

**Date:** 2026-08-15  
**Command:**
```bash
PEDRA_SIM_VOLUME_ROUNDS=1 \
PEDRA_SIM_VOLUME_SEEDS='0x17CAD571,0x17CA5EED,0x17CAAF01,0x00200A17,0xA11CE001,0xBEEF0001,0xCAFE0002,0xDEAD0003' \
./scripts/montanha_sim_volume_v0.sh findings/sim-volume-dst-20260815
```

## Result

| Field | Value |
|-------|--------|
| gate | rfc0021-p0.4-sim-volume-v0 |
| seeds | 8 |
| runs | 8 × montanha_fdb_path |
| suites | fdb_path, multiprocess_tx, chaos_soak (×3), **scale_gate** |
| wall_secs | ~726 |
| silent_wrong | **0** |

## Script change

`montanha_sim_volume_v0.sh` and `montanha_chaos_soak.sh` now run `montanha_scale_gate_v0` so option-A hygiene is part of continuous soak.
