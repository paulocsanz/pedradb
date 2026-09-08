#!/usr/bin/env python3
"""Search catalog + live code + RFCs for formal next-step candidates.

Does not pick a winner. Prints a board the agent must read against source.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
STORE = ROOT / "crates/pedradb-store/src/lib.rs"
RAFT = ROOT / "crates/pedradb-raft/src/lib.rs"
RFC_DIR = ROOT / "docs/rfc"
FN_OPEN = r"(?:pub(?:\([^)]+\))?\s+)?(?:async\s+)?fn\s+"
FN_HEAD = re.compile(
    r"\n\s*" + FN_OPEN + r"([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>]*>)?\s*\("
)


def load_json(rel: str) -> dict:
    return json.loads((ROOT / rel).read_text(encoding="utf-8"))


def mentions(src: str, name: str) -> bool:
    return re.search(r"\b" + re.escape(name) + r"\s*\(", src) is not None


def load(rel: str) -> str:
    p = ROOT / rel
    return p.read_text(encoding="utf-8") if p.is_file() else ""


def test_body(src: str, name: str) -> str:
    matches = list(
        re.finditer(
            r"\n\s*" + FN_OPEN + re.escape(name) + r"\s*(?:<[^>]*>)?\s*\(",
            src,
        )
    )
    if not matches:
        return ""

    def body_at(m: re.Match) -> str:
        nxt = FN_HEAD.search(src, m.end())
        return src[m.start() : nxt.start()] if nxt else src[m.start() :]

    for m in matches:
        after = src[m.end() : m.end() + 48]
        if after.lstrip().startswith("&mut self"):
            return body_at(m)
    return body_at(matches[0])


def rfc_open_slices() -> list[str]:
    """Open P-slices on formal RFCs. Skip bench RFCs."""
    out: list[str] = []
    for p in sorted(RFC_DIR.glob("*.md")):
        if not (
            p.name.startswith("015")
            or p.name.startswith("016")
            or p.name.startswith("017")
            or p.name.startswith("0061")
            or p.name.startswith("0051")
            or p.name.startswith("0056")
            or p.name.startswith("0070")
        ):
            continue
        text = p.read_text(encoding="utf-8")
        for i, line in enumerate(text.splitlines(), 1):
            if re.search(r"- \[ \] \*\*P[012]", line):
                out.append(f"{p.name}:{i}:{line.strip()}")
    return out


def main() -> int:
    cat = load_json("scripts/formal/catalog.json")
    res = load_json("scripts/formal/residuals.json")
    pairs = cat.get("pairs") or []
    fate = [p for p in pairs if p.get("data_fate")]
    other = [p for p in pairs if not p.get("data_fate")]
    store = load("crates/pedradb-store/src/lib.rs")
    raft = load("crates/pedradb-raft/src/lib.rs")

    print("== board (search; not a winner) ==")
    print(f"pairs={len(pairs)} data_fate={len(fate)} not_data_fate={len(other)}")
    glue = res.get("glue") or {}
    print(
        "glue "
        f"kernel_files={glue.get('kernel_files')} "
        f"kernel_loc={glue.get('kernel_loc')} "
        f"handler_loc={glue.get('handler_loc')} "
        f"db_rs_extracted={glue.get('db_rs_extracted')}"
    )
    print("never_floor", " ".join(res.get("never_floor") or []))

    print("== RFC open P0/P1/P2 ==")
    opens = rfc_open_slices()
    if not opens:
        print("(none)")
    else:
        for row in opens:
            print(row)

    print("== A absent twins ==")
    absent = [p["id"] for p in pairs if p.get("status") == "absent"]
    print(" ".join(absent) if absent else "none")

    print("== B data_fate freeze holes ==")
    no_as = [p["id"] for p in fate if not p.get("as_is")]
    no_pl = [p["id"] for p in fate if not p.get("dst_plant")]
    print("missing_as_is", " ".join(no_as) if no_as else "none")
    print("missing_dst_plant", " ".join(no_pl) if no_pl else "none")

    print("== C live_callers vs store/raft mentions ==")
    live_ids = []
    for p in fate:
        entry = p.get("entry") or ""
        lcs = p.get("live_callers") or []
        in_store = mentions(store, entry) if entry else False
        in_raft = mentions(raft, entry) if entry else False
        if lcs:
            live_ids.append(p["id"])
            print(f"  live_callers {p['id']} entry={entry} store={in_store} raft={in_raft} {lcs}")
        elif in_store and p.get("kernel", "").startswith("crates/pedradb-raft"):
            print(f"  store_mentions_no_live_callers {p['id']} {entry}")

    print("== D protocol bits (persist/ack) in store ==")
    print("  grant_after_persist", "yes" if mentions(store, "grant_after_persist") else "NO")
    print("  ae_ack_success", "yes" if mentions(store, "ae_ack_success") else "NO")
    print("  persist_hard_db", "yes" if mentions(store, "persist_hard_db") else "NO")
    gp = next((p for p in pairs if p.get("id") == "grant_persist"), None)
    if gp:
        print("  grant_persist live_callers", gp.get("live_callers") or "missing")

    print("== D/C plants: inbound vs pure-fn cartoon ==")
    cartoon = []
    inbound = []
    live_core = []
    missing_test = []
    for p in fate:
        plant = p.get("dst_plant") or {}
        pfile, ptest, entry = plant.get("file"), plant.get("test"), p.get("entry") or ""
        if not pfile or not ptest:
            continue
        src = load(pfile)
        body = test_body(src, ptest)
        if not body:
            missing_test.append(p["id"])
            print(f"  MISSING_TEST {p['id']} {pfile} {ptest}")
            continue
        has_entry = mentions(body, entry) if entry else False
        has_in = "handle_inbound" in body or "PeerMsg::" in body
        uses_queued = (
            "LiveQueued::open" in body
            or "pin_dst_queued" in body
            or "l28_campaign" in body
        )
        # Cartoon = Queued open + entry( without inbound (skill). Core Db
        # plants (flush/scan) are live, not cartoon.
        if has_in:
            kind = "inbound"
        elif uses_queued:
            kind = "pure_fn"
        else:
            kind = "live_core"
        if kind == "pure_fn":
            cartoon.append(p["id"])
        elif kind == "inbound":
            inbound.append(p["id"])
        else:
            live_core.append(p["id"])
        if p["id"] in ("vote", "ae_entry", "grant_persist", "ae_ack") or kind != "inbound":
            print(
                f"  {kind} {p['id']} test={ptest} entry={has_entry} "
                f"inbound={has_in}"
            )
    print(
        f"  cartoon_pure_fn_count={len(cartoon)} inbound_count={len(inbound)} "
        f"live_core_count={len(live_core)}"
    )
    print("  cartoon_ids", " ".join(cartoon) if cartoon else "(none)")
    print("  live_core_ids", " ".join(live_core) if live_core else "(none)")

    print("== E non-data_fate three-teeth ==")
    nd_as = sum(1 for p in other if p.get("as_is"))
    nd_pl = sum(1 for p in other if p.get("dst_plant"))
    print(f"  as_is={nd_as}/{len(other)} dst_plant={nd_pl}/{len(other)}")
    print("  ids", " ".join(p["id"] for p in other))

    print("== RFC-0170 atom→close (above F/E when D/C/B/A empty) ==")
    atoms = [p for p in pairs if p.get("twin_kind") == "atom"]
    fate_atoms = [p for p in atoms if p.get("data_fate")]
    print(f"  atom={len(atoms)} data_fate_atom={len(fate_atoms)}")
    if not fate_atoms:
        print("  atom_to_close none")
    else:
        lead = fate_atoms[0]
        print(f"  atom_to_close {lead['id']} {lead.get('entry')}")
        for p in fate_atoms:
            print(f"    remaining {p['id']} entry={p.get('entry')} atom={p.get('atom')}")

    print("== F clones ==")
    print(" ", " ".join(c.get("id", "") for c in cat.get("clones") or []))

    print("== G/H/I ==")
    print("  G never_floor + db_rs_extracted must stay false — not a next proof")
    print("  H L28 TCP / PCT / lock interleavings — campaign not forall")
    print("  I benches/0149/crates.io — not verification")
    script_compose_board()
    sa_unpaid_board(fate)
    return capacity_board(cat, res)


# Glue handlers. Two payments — do not OR them (that hid compose unpaid
# behind `_plan(` in the body and sent the grind into an SA factory).
# Glue method names are never Lean defs; `plan` is the extractable caller.
# tuple: glue_fn, file, tokens, callee, plan
GLUE_SCRIPTS = [
    (
        "commit_ops_with",
        "crates/pedradb-core/src/db.rs",
        ("append_write_ops", "sync_data", "apply_ops_to_mem", "fence_on_sync_fail"),
        "wal_sync_required",
        "wal_commit_plan",
    ),
    (
        "wal_sync_group",
        "crates/pedradb-core/src/db.rs",
        ("sync_data", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "lone_commit",
        "crates/pedradb-core/src/concurrent.rs",
        ("occ_conflict", "lone_sync_commit"),
        "occ_conflict",
        "occ_member_fate",
    ),
    (
        "finish_group_off_lock",
        "crates/pedradb-core/src/concurrent.rs",
        ("write_pending_frame", "sync_data", "fence_on_sync_fail"),
        "may_publish_group",
        "wal_commit_plan",
    ),
    (
        "validate_occ_batch",
        "crates/pedradb-core/src/concurrent.rs",
        ("occ_batch_plan", "key_has_write_after"),
        "occ_conflict",
        "occ_batch_plan",
    ),
    (
        "lone_sync_commit",
        "crates/pedradb-core/src/db.rs",
        ("sync_data", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "sync",
        "crates/pedradb-core/src/db.rs",
        ("sync_data", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "open_with_env_sourced",
        "crates/pedradb-core/src/db.rs",
        (
            "pit_resync_needs_rewrite",
            "torn_tail_needs_cut",
            "wal_commit_plan",
            "fence_on_sync_fail",
        ),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "group_finish",
        "crates/pedradb-core/src/db.rs",
        ("wal_sync_group", "write_pending_frame", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "vlog_prepare_wal",
        "crates/pedradb-core/src/db.rs",
        ("vlog_sync_pending", "vlog_flush_pending", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "fsync_sst_paths",
        "crates/pedradb-core/src/db.rs",
        ("sync_data", "fence_on_sync_fail", "dir_sync_required"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "write_checkpoint_meta",
        "crates/pedradb-core/src/db.rs",
        ("sync_all", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "close",
        "crates/pedradb-core/src/db.rs",
        ("vlog_prepare_wal", "flush", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "rotate_wal_now",
        "crates/pedradb-core/src/db.rs",
        ("persist_manifest_durable", "flush", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "try_rotate_wal",
        "crates/pedradb-core/src/db.rs",
        ("wal_rotate_decision", "wal_segment_is_empty", "rotate_wal_now"),
        "wal_segment_is_empty",
        "wal_rotate_decision",
    ),
    (
        "group_start",
        "crates/pedradb-core/src/db.rs",
        ("batch_is_empty", "vlog_prepare_wal", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "group_absorb",
        "crates/pedradb-core/src/db.rs",
        ("batch_is_empty", "vlog_prepare_wal", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
]


def lean_unfolds(name: str) -> bool:
    lean_dir = ROOT / "formal/aeneas/lean"
    if not lean_dir.is_dir():
        return False
    pat = re.compile(r"\bunfold\s+" + re.escape(name) + r"\b")
    for p in lean_dir.glob("*.lean"):
        if pat.search(p.read_text(encoding="utf-8", errors="replace")):
            return True
    return False


def lean_has_def(name: str) -> bool:
    lean_dir = ROOT / "formal/aeneas/lean"
    out_dir = ROOT / "formal/aeneas/out/lean"
    pat = re.compile(r"\bdef\s+(?:[A-Za-z0-9_]+\.)*" + re.escape(name) + r"\b")
    for d in (lean_dir, out_dir):
        if not d.is_dir():
            continue
        for p in d.glob("*.lean"):
            if pat.search(p.read_text(encoding="utf-8", errors="replace")):
                return True
    return False


def script_compose_board() -> None:
    print("== script (rank 4: handler calls the named plan) ==")
    unpaid_script = 0
    unpaid_compose = 0
    for glue, rel, tokens, callee, plan in GLUE_SCRIPTS:
        src = load(rel)
        body = test_body(src, glue)
        if not body:
            print(f"  MISSING_FN {glue} {rel}")
            unpaid_script += 1
            unpaid_compose += 1
            continue
        missing = [t for t in tokens if t not in body]
        calls_plan = plan + "(" in body or "_plan(" in body
        extra = (" tokens_missing=" + ",".join(missing)) if missing else ""
        if calls_plan and not missing:
            print(f"  {glue} plan={plan} calls_plan extra=ok")
        elif calls_plan and missing:
            unpaid_script += 1
            print(f"  {glue} plan={plan} UNPAID tokens_missing={','.join(missing)}")
        else:
            unpaid_script += 1
            print(f"  {glue} plan={plan} UNPAID order still inline{extra}")
    print(f"  unpaid_script={unpaid_script}/{len(GLUE_SCRIPTS)}")
    print(
        "  leftover_next store client put_batch pairs.is_empty leftover if; named kernel not SA"
    )
    print("== compose glue callers (rank 5: unfold plan AND callee) ==")
    for glue, rel, tokens, callee, plan in GLUE_SCRIPTS:
        unfold_plan = lean_unfolds(plan)
        unfold_callee = lean_unfolds(callee)
        if unfold_plan and unfold_callee:
            status = "lean_unfold_plan_and_callee"
        elif unfold_callee and not unfold_plan:
            status = "UNPAID callee-only unfold"
            unpaid_compose += 1
        else:
            status = "UNPAID no unfold of plan"
            unpaid_compose += 1
        print(
            f"  {glue} plan={plan} callee={callee} "
            f"unfold_plan={str(unfold_plan).lower()} "
            f"unfold_callee={str(unfold_callee).lower()} {status}"
        )
    print(f"  unpaid_compose={unpaid_compose}/{len(GLUE_SCRIPTS)}")
    concurrency_board()


# Rank 6: named total fn the live handler calls + Lean unfold of that
# caller AND a callee. Not dump of concurrent.rs / db.rs. Not SA wrap.
# tuple: label, file, handler, caller, callee
CONCURRENCY = [
    (
        "write-lock client",
        "crates/pedradb-core/src/concurrent.rs",
        "occ_snapshot",
        "occ_snap_lock_order",
        "occ_snap_uses_published",
    ),
    (
        "lost-update",
        "crates/pedradb-core/src/concurrent.rs",
        "validate_occ_batch",
        "occ_batch_plan",
        "occ_conflict",
    ),
    (
        "deadlock 2PL",
        "crates/rocksdb-compat/src/locktab.rs",
        "lock",
        "wait_for_deadlock",
        "wait_for_deadlock",
    ),
    (
        "N-way OCC",
        "crates/pedradb-core/src/group_commit_kernel.rs",
        "group_validate",
        "group_validate",
        "occ_conflict",
    ),
    (
        "rwlock reader token",
        "crates/pedradb-core/src/concurrent.rs",
        "occ_snapshot",
        "rwlock_client_may_read",
        "rwlock_client_may_mutate",
    ),
]


def concurrency_board() -> None:
    print("== concurrency (rank 6: named fn + unfold caller AND callee) ==")
    unpaid = 0
    for label, rel, handler, caller, callee in CONCURRENCY:
        src = load(rel)
        body = test_body(src, handler)
        calls = bool(body) and (caller + "(" in body)
        unfold_caller = lean_unfolds(caller)
        unfold_callee = lean_unfolds(callee)
        if not body:
            status = "UNPAID missing handler"
            unpaid += 1
        elif not calls:
            status = "UNPAID handler does not call " + caller
            unpaid += 1
        elif not (unfold_caller and unfold_callee):
            status = "UNPAID no dual-unfold"
            unpaid += 1
        else:
            status = "lean_unfold_caller_and_callee"
        print(
            f"  {label} handler={handler} caller={caller} callee={callee} "
            f"calls={str(calls).lower()} "
            f"unfold_caller={str(unfold_caller).lower()} "
            f"unfold_callee={str(unfold_callee).lower()} {status}"
        )
    print(f"  unpaid_concurrency={unpaid}/{len(CONCURRENCY)}")
    scale_board()


# Rank 10: enrolled scale_kernel on a concrete N. Handler scale_forecast
# must call the named clock; Lean unfolds that clock AND a callee.
SCALE_CLOCKS = [
    (
        "best clock",
        "crates/pedradb-core/src/scale_kernel.rs",
        "scale_forecast",
        "best_get_ns",
        "point_get_probes",
    ),
    (
        "happy clock",
        "crates/pedradb-core/src/scale_kernel.rs",
        "scale_forecast",
        "happy_get_ns",
        "point_get_probes",
    ),
    (
        "worst clock",
        "crates/pedradb-core/src/scale_kernel.rs",
        "scale_forecast",
        "worst_get_ns",
        "probes_worst",
    ),
]


def scale_board() -> None:
    print("== scale (rank 10: named clock + unfold clock AND callee) ==")
    unpaid = 0
    for label, rel, handler, caller, callee in SCALE_CLOCKS:
        src = load(rel)
        body = test_body(src, handler)
        calls = bool(body) and (caller + "(" in body)
        unfold_caller = lean_unfolds(caller)
        unfold_callee = lean_unfolds(callee)
        if not body:
            status = "UNPAID missing handler"
            unpaid += 1
        elif not calls:
            status = "UNPAID handler does not call " + caller
            unpaid += 1
        elif not (unfold_caller and unfold_callee):
            status = "UNPAID no dual-unfold"
            unpaid += 1
        else:
            status = "lean_unfold_caller_and_callee"
        print(
            f"  {label} handler={handler} caller={caller} callee={callee} "
            f"calls={str(calls).lower()} "
            f"unfold_caller={str(unfold_caller).lower()} "
            f"unfold_callee={str(unfold_callee).lower()} {status}"
        )
    print(f"  unpaid_scale={unpaid}/{len(SCALE_CLOCKS)}")


def sa_unpaid_board(fate: list) -> None:
    print("== single_artifact (rank 7: skip if already extracted or Verus last-wins) ==")
    unpaid = []
    skip_extracted = []
    skip_verus = []
    paid = []
    for p in fate:
        k = p.get("kernel") or ""
        t = p.get("twin") or ""
        entry = p.get("entry") or ""
        if t and k and t == k and p.get("single_artifact"):
            paid.append(p["id"])
            continue
        if t == k:
            continue
        kp = ROOT / k
        has_verus = kp.is_file() and "verus_keep_ghost" in kp.read_text(
            encoding="utf-8", errors="replace"
        )
        if has_verus:
            skip_verus.append(p["id"])
            continue
        if entry and lean_has_def(entry):
            skip_extracted.append(p["id"])
            continue
        unpaid.append(p["id"])
    print("  unpaid_no_extract", " ".join(unpaid) if unpaid else "none")
    print(
        f"  skip_already_extracted={len(skip_extracted)} "
        "(Lean def of entry exists — SA wrap is not a slice)"
    )
    print(
        f"  skip_verus_last_wins={len(skip_verus)} "
        "(production file already has cfg(verus_keep_ghost))"
    )
    print(
        f"  catalog_only_skip={len(paid)} "
        "(twin==kernel already; not a slice)"
    )


# RFC-0157 P2.2 — capacity per residual. The mapping below is id ->
# evidence anchors ONLY; every anchor is verified against the repo (guard
# test exists, catalog twin + runner exist, findings path exists) and a
# broken reference fails the board. Nothing else is hand-written per id.
GUARD_TESTS = {
    "R-unsafe-posix": ["posix_unsafe_rc_sites_all_gated"],
    "R-unsafe-capi": ["capi_len_boundary_sweep_on_live_tx"],
    "R-unsafe-uring": ["cqe_leftover_sequence_never_false_ok"],
    "R-group-glue": ["planted_chain3_found_by_pct_d3"],
}
TWIN_PAIRS = {
    "R-unsafe-posix": ["fdatasync_rc"],
    "R-unsafe-capi": ["c_len"],
    "R-unsafe-uring": ["cqe_res"],
    "R-group-glue": [
        "group_commit", "group_fence", "group_publish", "forall_schedules",
        "fsync_promote", "media_durable", "lock_interleavings",
    ],
    "R-pct": ["forall_schedules"],
    "R-glue": ["zero_glue"],
    "R-crc": ["crc_match", "sst_crc"],
    "R-fsync-lie": ["fsync_promote", "media_durable"],
    "R-swarm-real": ["l28_durability", "l28_tcp_apply", "l28_tcp_napply", "l28_napply_retry"],
    "R-es": ["liveness_claim"],
    "R-tcg-guest": ["tcg_guest"],
    "R-direct-rpc": ["rpc_mode"],
    "R-joint": [
        "joint_election", "joint_leave", "pending_joint_node", "joint_leave_ok",
        "election_grant_from", "joint_target", "joint_add_target",
        "queued_leave_finish", "disk_membership",
    ],
}
CAMPAIGN_DEPTH = {
    "R-pct": "PCT d=3 16384 seeds + d=4 16384 seeds (0157 P2.3) + exaustivo N<=3 (P1.3)",
    "R-group-glue": "exaustivo N<=3 completo (66 scheds) + disk-fence lower-bound",
    "R-swarm-real": "TCP REAL K=8 (0156: 3 seeds; 0157: C-campanha + campanhas noturnas)",
}
REAL_ANCHORS = {
    "R-swarm-real": "findings/rfc0157-tcp-campaign/,findings/rfc0157-nightly/",
    "R-fsync-lie": "findings/2026-08-24-tcg-world-smoke/,findings/2026-08-27-upstream-fullfsync/",
    "R-tcg-guest": "findings/2026-08-27-tcg-caixote-guest/",
    "R-direct-rpc": "findings/2026-08-30-direct-rpc-lab/",
}


def capacity_board(cat: dict, res: dict) -> int:
    print()
    print("== capacity per residual (RFC-0157 P2.2; verified refs) ==")
    pairs = {p["id"]: p for p in cat.get("pairs") or []}
    errors: list[str] = []
    rows = res.get("residuals") or []

    def grep_repo_test(name: str) -> bool:
        pat = re.compile(r"fn\s+" + re.escape(name) + r"\s*\(")
        for p in ROOT.glob("crates/*/src/**/*.rs"):
            if pat.search(p.read_text(encoding="utf-8", errors="replace")):
                return True
        return False

    for r in rows:
        rid = r["id"]
        guards = GUARD_TESTS.get(rid) or []
        for g in guards:
            if not grep_repo_test(g):
                errors.append(f"{rid}: guard test {g} not found in crates/")
        guard = "sim (" + ",".join(guards) + ")" if guards else "nao"

        tids = TWIN_PAIRS.get(rid) or []
        for t in tids:
            p = pairs.get(t)
            if p is None:
                errors.append(f"{rid}: twin pair {t} not in catalog")
                continue
            twin_file = ROOT / p.get("twin", "")
            runner = ROOT / p.get("verus", "")
            if not twin_file.is_file():
                errors.append(f"{rid}: twin file missing {p.get('twin')}")
            if not runner.is_file():
                errors.append(f"{rid}: runner missing {p.get('verus')}")
        if rid == "R-verus":
            n_runners = len(list((ROOT / "scripts").glob("verus_*.sh")))
            twin = (
                "sim (corpus verus_check.sh --all "
                f"{n_runners} runners no checker pinado, RFC-0157 P2.1; "
                "checker/Z3 seguem TCB no never_floor)"
            )
        elif tids:
            twin = "sim (" + ",".join(tids) + ")"
        else:
            twin = "nao" + (
                " (never_floor: ferramenta/meio)" if r.get("class") == "never" else ""
            )

        depth = CAMPAIGN_DEPTH.get(rid, "nenhuma")

        anchor = REAL_ANCHORS.get(rid, "")
        for a in filter(None, anchor.split(",")):
            if not (ROOT / a.strip()).is_dir():
                errors.append(f"{rid}: REAL anchor missing {a.strip()}")
        real = anchor if anchor else "nao"

        print(f"{rid} | {r.get('class')} | guard={guard} | gemeo={twin} | campanha={depth} | real={real}")

    print(f"capacity rows: {len(rows)}")
    if errors:
        print("capacity ERRORS:")
        for e in errors:
            print("  " + e)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
