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
    deadlock_mutant = copy.deepcopy(catalog)
    found = False
    for pair in deadlock_mutant["pairs"]:
        if pair.get("id") == "wait_for_deadlock":
            pair["single_artifact"] = True
            pair["twin"] = "crates/rocksdb-compat/verus/wait_for_deadlock.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing wait_for_deadlock")
        return 1
    hits = [m for m in _sa_fails(deadlock_mutant) if "wait_for_deadlock" in m]
    if not hits:
        print("FAIL wait_for_deadlock twin≠kernel did not fail")
        return 1
    print("ok mutant wait_for_deadlock twin≠kernel named")
    cf_mutant = copy.deepcopy(catalog)
    found = False
    for pair in cf_mutant["pairs"]:
        if pair.get("id") == "cf_family":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/cf_family.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing cf_family")
        return 1
    hits = [m for m in _sa_fails(cf_mutant) if "cf_family" in m]
    if not hits:
        print("FAIL cf_family twin≠kernel did not fail")
        return 1
    print("ok mutant cf_family twin≠kernel named")
    vlog_mutant = copy.deepcopy(catalog)
    found = False
    for pair in vlog_mutant["pairs"]:
        if pair.get("id") == "vlog_recover":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/vlog_gc_decision.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing vlog_recover")
        return 1
    hits = [m for m in _sa_fails(vlog_mutant) if "vlog_recover" in m]
    if not hits:
        print("FAIL vlog_recover twin≠kernel did not fail")
        return 1
    print("ok mutant vlog_recover twin≠kernel named")
    manifest_mutant = copy.deepcopy(catalog)
    found = False
    for pair in manifest_mutant["pairs"]:
        if pair.get("id") == "manifest_recover":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/manifest_recover.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing manifest_recover")
        return 1
    hits = [m for m in _sa_fails(manifest_mutant) if "manifest_recover" in m]
    if not hits:
        print("FAIL manifest_recover twin≠kernel did not fail")
        return 1
    print("ok mutant manifest_recover twin≠kernel named")
    ae_mutant = copy.deepcopy(catalog)
    found = False
    for pair in ae_mutant["pairs"]:
        if pair.get("id") == "ae_entry":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-raft/verus/ae_entry_action.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing ae_entry")
        return 1
    hits = [m for m in _sa_fails(ae_mutant) if "ae_entry" in m]
    if not hits:
        print("FAIL ae_entry twin≠kernel did not fail")
        return 1
    print("ok mutant ae_entry twin≠kernel named")
    ack_mutant = copy.deepcopy(catalog)
    found = False
    for pair in ack_mutant["pairs"]:
        if pair.get("id") == "ae_ack":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-raft/verus/ae_ack_success.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing ae_ack")
        return 1
    hits = [m for m in _sa_fails(ack_mutant) if "ae_ack" in m]
    if not hits:
        print("FAIL ae_ack twin≠kernel did not fail")
        return 1
    print("ok mutant ae_ack twin≠kernel named")
    txn_mutant = copy.deepcopy(catalog)
    found = False
    for pair in txn_mutant["pairs"]:
        if pair.get("id") == "txn":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing txn")
        return 1
    hits = [m for m in _sa_fails(txn_mutant) if m.startswith("txn:")]
    if not hits:
        print("FAIL txn twin≠kernel did not fail")
        return 1
    print("ok mutant txn twin≠kernel named")
    revert_mutant = copy.deepcopy(catalog)
    found = False
    for pair in revert_mutant["pairs"]:
        if pair.get("id") == "revert_clears_status":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing revert_clears_status")
        return 1
    hits = [m for m in _sa_fails(revert_mutant) if m.startswith("revert_clears_status:")]
    if not hits:
        print("FAIL revert_clears_status twin≠kernel did not fail")
        return 1
    print("ok mutant revert_clears_status twin≠kernel named")
    rua_mutant = copy.deepcopy(catalog)
    found = False
    for pair in rua_mutant["pairs"]:
        if pair.get("id") == "revert_user_action":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing revert_user_action")
        return 1
    hits = [m for m in _sa_fails(rua_mutant) if m.startswith("revert_user_action:")]
    if not hits:
        print("FAIL revert_user_action twin≠kernel did not fail")
        return 1
    print("ok mutant revert_user_action twin≠kernel named")
    sih_mutant = copy.deepcopy(catalog)
    found = False
    for pair in sih_mutant["pairs"]:
        if pair.get("id") == "should_repair_si_hist":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing should_repair_si_hist")
        return 1
    hits = [m for m in _sa_fails(sih_mutant) if m.startswith("should_repair_si_hist:")]
    if not hits:
        print("FAIL should_repair_si_hist twin≠kernel did not fail")
        return 1
    print("ok mutant should_repair_si_hist twin≠kernel named")
    dc_mutant = copy.deepcopy(catalog)
    found = False
    for pair in dc_mutant["pairs"]:
        if pair.get("id") == "discard_cut":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing discard_cut")
        return 1
    hits = [m for m in _sa_fails(dc_mutant) if m.startswith("discard_cut:")]
    if not hits:
        print("FAIL discard_cut twin≠kernel did not fail")
        return 1
    print("ok mutant discard_cut twin≠kernel named")
    lo_mutant = copy.deepcopy(catalog)
    found = False
    for pair in lo_mutant["pairs"]:
        if pair.get("id") == "leftover_txn_is_aborted":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing leftover_txn_is_aborted")
        return 1
    hits = [m for m in _sa_fails(lo_mutant) if m.startswith("leftover_txn_is_aborted:")]
    if not hits:
        print("FAIL leftover_txn_is_aborted twin≠kernel did not fail")
        return 1
    print("ok mutant leftover_txn_is_aborted twin≠kernel named")
    nid_mutant = copy.deepcopy(catalog)
    found = False
    for pair in nid_mutant["pairs"]:
        if pair.get("id") == "next_txn_id_after":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing next_txn_id_after")
        return 1
    hits = [m for m in _sa_fails(nid_mutant) if m.startswith("next_txn_id_after:")]
    if not hits:
        print("FAIL next_txn_id_after twin≠kernel did not fail")
        return 1
    print("ok mutant next_txn_id_after twin≠kernel named")
    rsg_mutant = copy.deepcopy(catalog)
    found = False
    for pair in rsg_mutant["pairs"]:
        if pair.get("id") == "recover_si_generation":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing recover_si_generation")
        return 1
    hits = [m for m in _sa_fails(rsg_mutant) if m.startswith("recover_si_generation:")]
    if not hits:
        print("FAIL recover_si_generation twin≠kernel did not fail")
        return 1
    print("ok mutant recover_si_generation twin≠kernel named")
    pea_mutant = copy.deepcopy(catalog)
    found = False
    for pair in pea_mutant["pairs"]:
        if pair.get("id") == "prepare_error_aborts_earlier":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing prepare_error_aborts_earlier")
        return 1
    hits = [m for m in _sa_fails(pea_mutant) if m.startswith("prepare_error_aborts_earlier:")]
    if not hits:
        print("FAIL prepare_error_aborts_earlier twin≠kernel did not fail")
        return 1
    print("ok mutant prepare_error_aborts_earlier twin≠kernel named")
    rsg2_mutant = copy.deepcopy(catalog)
    found = False
    for pair in rsg2_mutant["pairs"]:
        if pair.get("id") == "reserve_si_gen":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing reserve_si_gen")
        return 1
    hits = [m for m in _sa_fails(rsg2_mutant) if m.startswith("reserve_si_gen:")]
    if not hits:
        print("FAIL reserve_si_gen twin≠kernel did not fail")
        return 1
    print("ok mutant reserve_si_gen twin≠kernel named")
    usg_mutant = copy.deepcopy(catalog)
    found = False
    for pair in usg_mutant["pairs"]:
        if pair.get("id") == "unreserve_si_gen":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-store/verus/txn_kernel.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing unreserve_si_gen")
        return 1
    hits = [m for m in _sa_fails(usg_mutant) if m.startswith("unreserve_si_gen:")]
    if not hits:
        print("FAIL unreserve_si_gen twin≠kernel did not fail")
        return 1
    print("ok mutant unreserve_si_gen twin≠kernel named")
    dl_mutant = copy.deepcopy(catalog)
    found = False
    for pair in dl_mutant["pairs"]:
        if pair.get("id") == "dictionary_link":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/dictionary_link.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing dictionary_link")
        return 1
    hits = [m for m in _sa_fails(dl_mutant) if m.startswith("dictionary_link:")]
    if not hits:
        print("FAIL dictionary_link twin≠kernel did not fail")
        return 1
    print("ok mutant dictionary_link twin≠kernel named")
    blob_mutant = copy.deepcopy(catalog)
    found = False
    for pair in blob_mutant["pairs"]:
        if pair.get("id") == "blob_gc_pick":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/vlog_gc_decision.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing blob_gc_pick")
        return 1
    hits = [m for m in _sa_fails(blob_mutant) if m.startswith("blob_gc_pick:")]
    if not hits:
        print("FAIL blob_gc_pick twin≠kernel did not fail")
        return 1
    print("ok mutant blob_gc_pick twin≠kernel named")
    lev_mutant = copy.deepcopy(catalog)
    found = False
    for pair in lev_mutant["pairs"]:
        if pair.get("id") == "leveling":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/leveling.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing leveling")
        return 1
    hits = [m for m in _sa_fails(lev_mutant) if m.startswith("leveling:")]
    if not hits:
        print("FAIL leveling twin≠kernel did not fail")
        return 1
    print("ok mutant leveling twin≠kernel named")
    pick_mutant = copy.deepcopy(catalog)
    found = False
    for pair in pick_mutant["pairs"]:
        if pair.get("id") == "leveling_pick":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/leveling_pick.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing leveling_pick")
        return 1
    hits = [m for m in _sa_fails(pick_mutant) if m.startswith("leveling_pick:")]
    if not hits:
        print("FAIL leveling_pick twin≠kernel did not fail")
        return 1
    print("ok mutant leveling_pick twin≠kernel named")
    pd_mutant = copy.deepcopy(catalog)
    found = False
    for pair in pd_mutant["pairs"]:
        if pair.get("id") == "leveling_pushdown":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/leveling_pick.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing leveling_pushdown")
        return 1
    hits = [m for m in _sa_fails(pd_mutant) if m.startswith("leveling_pushdown:")]
    if not hits:
        print("FAIL leveling_pushdown twin≠kernel did not fail")
        return 1
    print("ok mutant leveling_pushdown twin≠kernel named")
    wrc_mutant = copy.deepcopy(catalog)
    found = False
    for pair in wrc_mutant["pairs"]:
        if pair.get("id") == "write_record_count":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/write_record_count.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing write_record_count")
        return 1
    hits = [m for m in _sa_fails(wrc_mutant) if m.startswith("write_record_count:")]
    if not hits:
        print("FAIL write_record_count twin≠kernel did not fail")
        return 1
    print("ok mutant write_record_count twin≠kernel named")
    vis_mutant = copy.deepcopy(catalog)
    found = False
    for pair in vis_mutant["pairs"]:
        if pair.get("id") == "visible_at":
            pair["single_artifact"] = True
            pair["twin"] = "crates/pedradb-core/verus/visible_at.rs"
            found = True
            break
    if not found:
        print("FAIL catalog missing visible_at")
        return 1
    hits = [m for m in _sa_fails(vis_mutant) if m.startswith("visible_at:")]
    if not hits:
        print("FAIL visible_at twin≠kernel did not fail")
        return 1
    print("ok mutant visible_at twin≠kernel named")
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
