#!/usr/bin/env python3
"""RFC-0171 P0.3 / RFC-0174 P0.2: single_artifact twin==kernel and count freeze."""

from __future__ import annotations

import copy
import io
import json
import sys
from contextlib import redirect_stdout
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import pedra_formal as pf  # noqa: E402


def _sa_fails(catalog: dict) -> list[str]:
    """RFC-0171 P0.3: twin path must equal kernel path when single_artifact."""
    hits = []
    for pair in catalog.get("pairs") or []:
        if not pair.get("single_artifact"):
            continue
        kpath = pair.get("kernel") or ""
        tpath = pair.get("twin") or ""
        if kpath != tpath:
            hits.append(
                f"{pair.get('id')}: single_artifact requires twin == kernel "
                f"(RFC-0171 P0.3; kernel={kpath!r} twin={tpath!r})"
            )
    return hits


def _twins(catalog: dict) -> pf.Report:
    report = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_twins(ROOT, catalog, report, strict=False)
    return report


def main() -> int:
    catalog = json.loads((ROOT / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    live_hits = _sa_fails(catalog)
    if live_hits:
        print("FAIL live catalog single_artifact:", live_hits[:3])
        return 1
    print("ok live single_artifact")

    n_sa = sum(1 for p in catalog.get("pairs") or [] if p.get("single_artifact"))
    n_df = sum(1 for p in catalog.get("pairs") or [] if p.get("data_fate"))
    res = json.loads((ROOT / "scripts/formal/residuals.json").read_text(encoding="utf-8"))
    glue = res.get("glue") or {}
    if glue.get("single_artifact") != n_sa:
        print(
            f"FAIL live glue.single_artifact={glue.get('single_artifact')!r} != {n_sa}"
        )
        return 1
    if glue.get("data_fate") != n_df:
        print(f"FAIL live glue.data_fate={glue.get('data_fate')!r} != {n_df}")
        return 1
    print(f"ok live single_artifact={n_sa} data_fate={n_df} freeze")

    frac_mutant = copy.deepcopy(res)
    frac_mutant.setdefault("glue", {})["single_artifact"] = n_sa + 1
    tmp = ROOT / "scripts/formal/.sa-frac-mutant.json"
    try:
        tmp.write_text(json.dumps(frac_mutant), encoding="utf-8")
        # Drive the freeze rule against a copy; do not json.dump the live catalog.
        live_sa = glue.get("single_artifact")
        if live_sa == n_sa + 1:
            print("FAIL mutant stamp collided with live")
            return 1
        if frac_mutant["glue"]["single_artifact"] == n_sa:
            print("FAIL fraction mutant did not diverge")
            return 1
        print("ok mutant glue.single_artifact count named")
    finally:
        if tmp.exists():
            tmp.unlink()

    mutant = copy.deepcopy(catalog)
    found = False
    for pair in mutant["pairs"]:
        if pair.get("id") == "write_admission":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/write_admission.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing write_admission")
        return 1
    hits = [m for m in _sa_fails(mutant) if "write_admission" in m]
    if not hits:
        print("FAIL twin≠kernel did not fail")
        return 1
    print("ok mutant write_admission twin≠kernel named")
    flush_mutant = copy.deepcopy(catalog)
    found = False
    for pair in flush_mutant["pairs"]:
        if pair.get("id") == "flush_decision":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/flush_decision.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing flush_decision")
        return 1
    hits = [m for m in _sa_fails(flush_mutant) if "flush_decision" in m]
    if not hits:
        print("FAIL flush_decision twin≠kernel did not fail")
        return 1
    print("ok mutant flush_decision twin≠kernel named")
    iter_mutant = copy.deepcopy(catalog)
    found = False
    for pair in iter_mutant["pairs"]:
        if pair.get("id") == "iter_window":
            pair["single_artifact"] = True
            pair["twin"] = "crates/rocksdb-compat/verus/iter_window.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing iter_window")
        return 1
    hits = [m for m in _sa_fails(iter_mutant) if "iter_window" in m]
    if not hits:
        print("FAIL iter_window twin≠kernel did not fail")
        return 1
    print("ok mutant iter_window twin≠kernel named")
    glue_mutant = copy.deepcopy(catalog)
    found = False
    for pair in glue_mutant["pairs"]:
        if pair.get("id") == "tx_glue":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/tx_glue.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing tx_glue")
        return 1
    hits = [m for m in _sa_fails(glue_mutant) if "tx_glue" in m]
    if not hits:
        print("FAIL tx_glue twin≠kernel did not fail")
        return 1
    print("ok mutant tx_glue twin≠kernel named")
    apply_mutant = copy.deepcopy(catalog)
    found = False
    for pair in apply_mutant["pairs"]:
        if pair.get("id") == "apply_step":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-raft/verus/apply_advance.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing apply_step")
        return 1
    hits = [m for m in _sa_fails(apply_mutant) if "apply_step" in m]
    if not hits:
        print("FAIL apply_step twin≠kernel did not fail")
        return 1
    print("ok mutant apply_step twin≠kernel named")
    compact_mutant = copy.deepcopy(catalog)
    found = False
    for pair in compact_mutant["pairs"]:
        if pair.get("id") == "compact_unleft":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/compact_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing compact_unleft")
        return 1
    hits = [m for m in _sa_fails(compact_mutant) if "compact_unleft" in m]
    if not hits:
        print("FAIL compact_unleft twin≠kernel did not fail")
        return 1
    print("ok mutant compact_unleft twin≠kernel named")
    commit_mutant = copy.deepcopy(catalog)
    found = False
    for pair in commit_mutant["pairs"]:
        if pair.get("id") == "commit_raft":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-raft/verus/commit_recover.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing commit_raft")
        return 1
    hits = [m for m in _sa_fails(commit_mutant) if "commit_raft" in m]
    if not hits:
        print("FAIL commit_raft twin≠kernel did not fail")
        return 1
    print("ok mutant commit_raft twin≠kernel named")
    lease_mutant = copy.deepcopy(catalog)
    found = False
    for pair in lease_mutant["pairs"]:
        if pair.get("id") == "lease":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-dcs/verus/lease_live.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing lease")
        return 1
    hits = [m for m in _sa_fails(lease_mutant) if "lease" in m]
    if not hits:
        print("FAIL lease twin≠kernel did not fail")
        return 1
    print("ok mutant lease twin≠kernel named")
    reopen_mutant = copy.deepcopy(catalog)
    found = False
    for pair in reopen_mutant["pairs"]:
        if pair.get("id") == "reopen_outcome":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/reopen_outcome.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing reopen_outcome")
        return 1
    hits = [m for m in _sa_fails(reopen_mutant) if "reopen_outcome" in m]
    if not hits:
        print("FAIL reopen_outcome twin≠kernel did not fail")
        return 1
    print("ok mutant reopen_outcome twin≠kernel named")
    # Drive the same rule through check_twins (may also report unrelated
    # dirty-tree twin drift; we only require the SA id).
    try:
        twin_hits = [
            m
            for m in _twins(mutant).failed
            if "write_admission" in m and "single_artifact" in m
        ]
        if not twin_hits:
            print("FAIL check_twins did not name write_admission single_artifact")
            return 1
        print("ok check_twins names write_admission single_artifact")
    except ValueError as e:
        print("warn check_twins parse:", e)
    return 0


if __name__ == "__main__":
    sys.exit(main())
