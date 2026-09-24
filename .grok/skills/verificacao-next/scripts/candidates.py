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
STORE = ROOT / "crates/pedradb-store/src/lib_kernel.rs"
RAFT = ROOT / "crates/pedradb-raft/src/lib_kernel.rs"
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
            or p.name.startswith("0187")
            or p.name.startswith("0188")
            or p.name.startswith("0191")
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
    store = load("crates/pedradb-store/src/lib_kernel.rs")
    raft = load("crates/pedradb-raft/src/lib_kernel.rs")

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
    unpaid_script, unpaid_compose = script_compose_board()
    unpaid_concurrency = concurrency_board()
    unpaid_scale = scale_board()
    unpaid_product, product_next = product_board()
    # Every catalog kernel, not only data_fate. twin==kernel +
    # single_artifact with a cfg/verus stand-in is still unpaid cartoon
    # (prefix.rs Seq vs rustc &[u8] hid here).
    cartoons = sa_unpaid_board(pairs)
    unpaid_tramp = trampoline_unpaid_data_fate()
    print_leftover_next(
        unpaid_script,
        unpaid_compose,
        unpaid_concurrency,
        unpaid_scale,
        unpaid_product,
        product_next,
        cartoons,
        unpaid_tramp,
    )
    return capacity_board(cat, res)


# Glue handlers. Two payments — do not OR them (that hid compose unpaid
# behind `_plan(` in the body and sent the grind into an SA factory).
# Glue method names are never Lean defs; `plan` is the extractable caller.
# tuple: glue_fn, file, tokens, callee, plan
GLUE_SCRIPTS = [
    (
        "commit_ops_with",
        "crates/pedradb-core/src/db_put_kernel.rs",
        ("append_write_ops", "sync_data", "apply_ops_to_mem", "fence_on_sync_fail"),
        "wal_sync_required",
        "wal_commit_plan",
    ),
    (
        "wal_sync_group",
        "crates/pedradb-core/src/db_kernel.rs",
        ("sync_data", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "lone_commit",
        "crates/pedradb-core/src/concurrent_kernel.rs",
        ("occ_conflict", "lone_sync_commit"),
        "occ_conflict",
        "occ_member_fate",
    ),
    (
        "finish_group_off_lock",
        "crates/pedradb-core/src/concurrent_kernel.rs",
        ("write_pending_frame", "sync_data", "fence_on_sync_fail"),
        "may_publish_group",
        "wal_commit_plan",
    ),
    (
        "validate_occ_batch",
        "crates/pedradb-core/src/concurrent_kernel.rs",
        ("occ_batch_plan", "key_has_write_after"),
        "occ_conflict",
        "occ_batch_plan",
    ),
    (
        "lone_sync_commit",
        "crates/pedradb-core/src/db_kernel.rs",
        ("sync_data", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "sync",
        "crates/pedradb-core/src/db_put_kernel.rs",
        ("sync_data", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "open_with_env_sourced",
        "crates/pedradb-core/src/db_open_kernel.rs",
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
        "crates/pedradb-core/src/db_kernel.rs",
        ("wal_sync_group", "write_pending_frame", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "vlog_prepare_wal",
        "crates/pedradb-core/src/db_kernel.rs",
        ("vlog_sync_pending", "vlog_flush_pending", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "fsync_sst_paths",
        "crates/pedradb-core/src/db_kernel.rs",
        ("sync_data", "fence_on_sync_fail", "dir_sync_required"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "write_checkpoint_meta",
        "crates/pedradb-core/src/db_kernel.rs",
        ("sync_all", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "close",
        "crates/pedradb-core/src/db_kernel.rs",
        ("vlog_prepare_wal", "flush", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "rotate_wal_now",
        "crates/pedradb-core/src/db_kernel.rs",
        ("persist_manifest_durable", "flush", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "try_rotate_wal",
        "crates/pedradb-core/src/db_kernel.rs",
        ("wal_rotate_decision", "wal_segment_is_empty", "rotate_wal_now"),
        "wal_segment_is_empty",
        "wal_rotate_decision",
    ),
    (
        "group_start",
        "crates/pedradb-core/src/db_kernel.rs",
        ("batch_is_empty", "vlog_prepare_wal", "fence_on_sync_fail"),
        "fence_on_sync_fail",
        "wal_commit_plan",
    ),
    (
        "group_absorb",
        "crates/pedradb-core/src/db_kernel.rs",
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


# RFC-0191 fire order. P1.6 is per-land hygiene (not a Fire).
# P2.4 is 0187 inherited / user-gated (not this grind).
PRODUCT_FIRE_ORDER = (
    "P0.1",
    "P0.2",
    "P0.3",
    "P1.1",
    "P1.2",
    "P1.3",
    "P1.4",
    "P2.1",
    "P2.2",
    "P1.5",
    "P2.3",
)
PRODUCT_RFC = RFC_DIR / "0191-pacote-garantias-produto.md"
PRODUCT_TSV = ROOT / "scripts/ratchet/product_guarantees.tsv"
PRODUCT_CHECKER = ROOT / "scripts/check_product_floor.py"


def rfc0191_open() -> list[str]:
    if not PRODUCT_RFC.is_file():
        return ["P0.1"]
    text = PRODUCT_RFC.read_text(encoding="utf-8")
    return re.findall(r"- \[ \] \*\*(P[012]\.\d+)\*\*", text)


def product_board() -> tuple[int, str | None]:
    """RFC-0191 product rows. Missing ratchet = P0.1 unpaid."""
    print("== product (RFC-0191: D1/R1/T1/C1 over rustc fn) ==")
    opens = rfc0191_open()
    fireable = [s for s in PRODUCT_FIRE_ORDER if s in opens]
    for s in fireable:
        print(f"  OPEN {s}")
    tsv_ok = PRODUCT_TSV.is_file()
    chk_ok = PRODUCT_CHECKER.is_file()
    print(f"  tsv={str(tsv_ok).lower()} checker={str(chk_ok).lower()}")
    if not tsv_ok or not chk_ok:
        print("  unpaid_product P0.1 ratchet missing")
        print(f"  unpaid_product={max(len(fireable), 1)} next=P0.1")
        return max(len(fireable), 1), "P0.1"
    nxt = fireable[0] if fireable else None
    unpaid = len(fireable)
    print(f"  unpaid_product={unpaid} next={nxt or 'none'}")
    return unpaid, nxt


def skip_montanha_path(path: str) -> bool:
    """Rank 13: leftover_next must not name store/montanha until the user lifts it."""
    return path.startswith("crates/pedradb-store/") or path.startswith(
        "crates/montanha"
    )


def trampoline_unpaid_data_fate() -> int:
    """Unpaid data-fate `if`s on the rustc put/open/write-group path.

    Same fns as `put_ok_and_recover_path_data_fate_ifs_call_kernels` plus
    ConcurrentDb put/open/lead. Env glue and kernel predicates are paid.
    """
    env = (
        "env.", "exists(", "metadata_len", "cfg!", "debug_assert", "opts.",
        "exclusive", "source", "sst_payload", "buggify", "per_cf",
        "write_stall_drain", "defer_auto_compact", "physical_cfs", "let Some(",
        "stage_flush_imm", "keep_wal_archives", "wal_archives", "resync_origin",
        "max_sequence", "large_value_threshold", "auto_blob_gc",
        "deadline", "collect_mode", "herd_only", "CATCHUP", "batch.len()",
        "batch_ops", "Instant", "now >=", "sealed_async",
    )
    kern = (
        "_kernel::", "write_admission_idle(", "write_admit(", "wal_sync_required(",
        "seq_exhausted(", "batch_is_empty(", "fence_on_sync_fail(",
        "wal_commit_plan(", "dir_sync_required(", "torn_head_is_empty_log(",
        "torn_tail_needs_cut(", "seq_after_feed(", "pit_resync_needs_rewrite(",
        "cas_absent_put(", "cas_eq_put(", "range_inverted(", "reopen_outcome(",
        "feed_is_lazy(", "skip_auto_flush(", "auto_flush_due(",
        "herd_collect_us(", "post_group_grace_us(", "merge_eligible(",
        "herd_full(",
    )
    sites = [
        ("crates/pedradb-core/src/db_kernel.rs", (
            "alloc_seq",
            "wal_sync_group", "sync_dir_if_required", "ensure_write_admitted_for",
            "maybe_auto_flush",
        )),
        ("crates/pedradb-core/src/db_put_kernel.rs", (
            "put_with", "apply_batch_with", "commit_ops_with",
        )),
        ("crates/pedradb-core/src/db_open_kernel.rs", (
            "open_with_env_sourced",
        )),
        ("crates/pedradb-core/src/concurrent_kernel.rs", (
            "lead",
        )),
        ("crates/pedradb-core/src/concurrent_put_kernel.rs", (
            "put_with_seq",
        )),
        ("crates/pedradb-core/src/concurrent_open_kernel.rs", (
            "open_with_env",
        )),
    ]
    n = 0
    for rel, names in sites:
        src = load(rel)
        for name in names:
            body = test_body(src, name)
            for cond in _if_conds(body):
                if any(k in cond for k in env) or any(k in cond for k in kern):
                    continue
                n += 1
    print(f"  unpaid_trampoline_data_fate_ifs={n}")
    return n


def _if_conds(body: str) -> list[str]:
    body = re.sub(r"//.*?$", "", body, flags=re.M)
    out: list[str] = []
    i = 0
    n = len(body)
    while i + 3 < n:
        at = (
            body[i : i + 2] == "if"
            and (i == 0 or not (body[i - 1].isalnum() or body[i - 1] == "_"))
            and body[i + 2] in " (\n"
        )
        if at:
            rest = body[i + 2 :]
            end = rest.find("{")
            if end >= 0:
                out.append(rest[:end].strip())
                i += 2 + end
                continue
        i += 1
    return out


def print_leftover_next(
    unpaid_script: int,
    unpaid_compose: int,
    unpaid_concurrency: int,
    unpaid_scale: int,
    unpaid_product: int,
    product_next: str | None,
    cartoons: list[tuple[str, str]],
    unpaid_tramp: int = 0,
) -> None:
    """Do not replace this with a production fn name. That is the factory."""
    if unpaid_script or unpaid_compose or unpaid_concurrency or unpaid_scale:
        print(
            "  leftover_next unpaid board remains "
            f"script={unpaid_script} compose={unpaid_compose} "
            f"concurrency={unpaid_concurrency} scale={unpaid_scale}; "
            "first UNPAID compose then script then rank 6 then rank 10; "
            "never leftover is_empty wrap; never compact_refuse spray; skip Montanha"
        )
        return
    # RFC-0191: product remaining beats cartoon (Montanha frozen) and
    # beats a random trampoline if while P0/P1.1–P1.4/P2.1–P2.2 are open.
    trampoline_ids = {"P1.5", "P2.3"}
    if unpaid_product and product_next and product_next not in trampoline_ids:
        print(
            f"  leftover_next product remaining RFC-0191 {product_next}; "
            "references/product.md; ∀ credit (not rfl concrete); "
            "layer model→atom→close only up; skip Montanha"
        )
        print(f"  leftover_next_first RFC-0191 {product_next}")
        return
    payable = [(i, f) for i, f in cartoons if not skip_montanha_path(f)]
    if payable:
        cid, cfile = payable[0]
        print(
            "  leftover_next cartoon remaining; "
            f"delete verus! stand-in from {cfile}; rustc body stays; "
            "Aeneas of handler types; Verus only same types; "
            "never mint; never _body! over u64-vs-bytes; "
            "never leftover is_empty wrap; never compact_refuse spray; skip Montanha"
        )
        print(f"  leftover_next_first {cid} {cfile}")
        return
    if unpaid_tramp:
        slice = product_next if product_next in {"P1.5", "P2.3"} else "P1.5"
        print(
            f"  leftover_next trampoline data-fate if remaining RFC-0191 {slice} "
            f"({unpaid_tramp} unpaid); pull one if into a named kernel "
            "rustc links with handler types; Aeneas extract of that body; "
            "cap_data_fate down + atom same commit; "
            "Verus only same types; never leftover is_empty wrap; "
            "never compact_refuse spray"
        )
        print(f"  leftover_next_first RFC-0191 {slice}")
        return
    print(
        "  leftover_next none — write-path data-fate ifs match named kernels "
        "(Env glue remains); db_rs_extracted=true (split open/put files + plans); "
        "never_floor intact"
    )
    print("  leftover_next_first none")


def script_compose_board() -> tuple[int, int]:
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
    return unpaid_script, unpaid_compose


# Rank 6: named total fn the live handler calls + Lean unfold of that
# caller AND a callee. Not dump of concurrent.rs / db.rs. Not SA wrap.
# tuple: label, file, handler, caller, callee
CONCURRENCY = [
    (
        "write-lock client",
        "crates/pedradb-core/src/concurrent_kernel.rs",
        "occ_snapshot",
        "occ_snap_lock_order",
        "occ_snap_uses_published",
    ),
    (
        "lost-update",
        "crates/pedradb-core/src/concurrent_kernel.rs",
        "validate_occ_batch",
        "occ_batch_plan",
        "occ_conflict",
    ),
    (
        "deadlock 2PL",
        "crates/rocksdb-compat/src/locktab_kernel.rs",
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
        "crates/pedradb-core/src/concurrent_kernel.rs",
        "occ_snapshot",
        "rwlock_client_may_read",
        "rwlock_client_may_mutate",
    ),
]


def concurrency_board() -> int:
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
    return unpaid


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


def scale_board() -> int:
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
    return unpaid


def verus_block(src: str) -> str:
    m = re.search(r"verus!\s*\{(.*)\}\s*// verus!", src, re.S)
    return m.group(1) if m else ""


def verus_token_kind(src: str) -> str:
    """Last-wins = same types both compilers see. Toy enum/u64/Seq vs rustc bytes = cartoon."""
    block = verus_block(src)
    rustc = src
    if "verus!" in src and "} // verus!" in src:
        rustc = src[: src.find("verus!")] + src[src.find("} // verus!") :]
    standin = bool(re.search(r"\benum\s+ValueType\b", block))
    if not standin:
        for m in re.finditer(
            r"(?:pub\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\((.*?)\)",
            block,
            re.S,
        ):
            name, params = m.group(1), m.group(2)
            rm = re.search(
                r"(?:pub\s+)?fn\s+" + re.escape(name) + r"\s*\((.*?)\)",
                rustc,
                re.S,
            )
            if not rm:
                continue
            rp = rm.group(1)
            if ("u64" in params or "Seq<" in params) and (
                "&[u8]" in rp or "Bound<" in rp
            ):
                standin = True
                break
    if standin:
        return "cartoon"
    if "macro_rules!" in src and re.search(r"_body!\s*\(", src):
        return "macro"
    # Comment-only mentions (headers narrating a deleted stand-in) are not
    # cartoons; only code-visible cfg splits are.
    code = "\n".join(
        l for l in src.splitlines() if not l.lstrip().startswith("//")
    )
    if "verus_keep_ghost" in code or block:
        return "cartoon"
    return "none"


def sa_unpaid_board(fate: list) -> list[tuple[str, str]]:
    print(
        "== single_artifact (rank 7: rustc body extract; "
        "Verus cartoon ≠ last-wins; all catalog kernels, not only data_fate) =="
    )
    unpaid = []
    skip_extracted = []
    skip_verus = []
    cartoon: list[tuple[str, str]] = []
    paid = []
    for p in fate:
        k = p.get("kernel") or ""
        t = p.get("twin") or ""
        entry = p.get("entry") or ""
        kp = ROOT / k
        src = (
            kp.read_text(encoding="utf-8", errors="replace") if kp.is_file() else ""
        )
        kind = verus_token_kind(src)
        if kind == "cartoon":
            cartoon.append((p["id"], k))
            continue
        if t and k and t == k and p.get("single_artifact"):
            paid.append(p["id"])
            continue
        if t == k:
            continue
        if kind == "macro":
            skip_verus.append(p["id"])
            continue
        if entry and lean_has_def(entry):
            skip_extracted.append(p["id"])
            continue
        unpaid.append(p["id"])
    print("  unpaid_no_extract", " ".join(unpaid) if unpaid else "none")
    print(
        f"  skip_already_extracted={len(skip_extracted)} "
        "(Lean def of rustc entry exists — SA wrap is not a slice)"
    )
    print(
        f"  skip_verus_same_tokens={len(skip_verus)} "
        "(macro_rules! last-wins RFC-0171; same types rustc links)"
    )
    print(
        f"  cartoon_twin={len(cartoon)} "
        "(verus! u64/toy enum/Seq ≠ rustc types — unpaid; "
        "delete the stand-in; Aeneas of rustc types; never mint)"
    )
    if cartoon:
        payable = [(i, f) for i, f in cartoon if not skip_montanha_path(f)]
        lead = payable[0] if payable else cartoon[0]
        print(f"  cartoon_first {lead[0]} {lead[1]}")
        seen: set[str] = set()
        for cid, cfile in cartoon:
            if cfile in seen:
                continue
            seen.add(cfile)
            print(f"    cartoon_file {cfile}")
    print(
        f"  catalog_only_skip={len(paid)} "
        "(twin==kernel already; not a slice)"
    )
    return cartoon


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
