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
FN_HEAD = re.compile(r"\n\s*fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")


def load_json(rel: str) -> dict:
    return json.loads((ROOT / rel).read_text(encoding="utf-8"))


def mentions(src: str, name: str) -> bool:
    return re.search(r"\b" + re.escape(name) + r"\s*\(", src) is not None


def load(rel: str) -> str:
    p = ROOT / rel
    return p.read_text(encoding="utf-8") if p.is_file() else ""


def test_body(src: str, name: str) -> str:
    m = re.search(r"\n\s*fn\s+" + re.escape(name) + r"\s*\(", src)
    if not m:
        return ""
    rest = src[m.start() :]
    nxt = FN_HEAD.search(rest, 1)
    return rest if nxt is None else rest[: nxt.start()]


def rfc_open_slices() -> list[str]:
    """Open P-slices on formal RFCs (015x / 0061 / 0056). Skip bench RFCs."""
    out: list[str] = []
    for p in sorted(RFC_DIR.glob("*.md")):
        if not (
            p.name.startswith("015")
            or p.name.startswith("0061")
            or p.name.startswith("0056")
            or p.name.startswith("0166")
            or p.name.startswith("0170")
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
    return capacity_board(cat, res)


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
