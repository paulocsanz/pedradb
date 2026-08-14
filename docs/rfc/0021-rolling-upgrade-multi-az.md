# RFC-0021 P2.4 — Rolling upgrade + multi-AZ lab

**Status:** runbook  
**Parent:** [0021](0021-montanha-fdb-tikv-parity-gaps.md)

## Rolling upgrade (3-node majority)

1. Confirm `/v1/cluster` shows one leader per range; majority healthy.  
2. Upgrade **one follower** first (new image); wait `status` + `/ready` 200.  
3. Upgrade second follower.  
4. Step-down / upgrade former leader last.  
5. Run `montanha-tcp smoke` or perf gate sample after each step.

## Multi-AZ lab

- Prefer odd voters across ≥2 failure domains (e.g. 2+1 hosts).  
- WireGuard/env hub already provides L3; do **not** place all 3 voters on one metal for “HA” claims.  
- Test: stop one AZ’s node → put still majority-commits (2/3).

## Gate artifacts

- Findings note after each drill: `findings/upgrade-drill-*/report.json` (manual or script later).
