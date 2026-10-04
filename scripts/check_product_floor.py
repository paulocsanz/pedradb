#!/usr/bin/env python3
"""RFC-0191 P0.1 — product-guarantee ratchet (blocking, self-verifying).

Four product rows (D1/R1/T1/C1) live in
``scripts/ratchet/product_guarantees.tsv``. Layer only moves UP
(model → atom → close). ``floor_promoted N`` is the minimum number of
atom|close rows — dropping R1 from atom back to model at the day-1
freeze (N=1) is red.

atom/close rows reuse the RFC-0188 credit rule: theorem exists, statement
carries a forall-binder (named property over all inputs, not ``rfl`` on
concrete inputs), Lean file has zero ``sorry``, catalog id resolves.

``--selftest`` proves redness in memory: layer descent, theorem without
forall, dangling catalog id must each be caught; the honest freeze must
pass (an always-red checker is also a bug).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TSV = REPO / "scripts" / "ratchet" / "product_guarantees.tsv"
CATALOG = REPO / "scripts" / "formal" / "catalog.json"

REQUIRED_IDS = ("D1", "R1", "T1", "C1")
LAYERS = ("model", "atom", "close")
WRAP_FACTORY_SUBJECTS = {
    "batch_is_empty",
    "dir_sync_required",
    "put_ok",
    "d1_modelo",
    "r1_modelo",
    "t1_modelo",
    "c1_modelo",
}


def parse_tsv(text: str) -> tuple[int, list[dict[str, str]]]:
    floor = None
    rows: list[dict[str, str]] = []
    for no, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("floor_promoted "):
            floor = int(line.split()[1])
            continue
        parts = line.split()
        if len(parts) != 8:
            raise SystemExit(
                f"GATE product-floor: FAIL — line {no} needs 8 fields "
                f"(id layer catalog theorem lean entry subject subject_kind): {raw!r}"
            )
        pid, layer, catalog_id, theorem, lean_file, entry, subject, subject_kind = parts
        if pid not in REQUIRED_IDS:
            raise SystemExit(f"GATE product-floor: FAIL — unknown id {pid!r} (want D1|R1|T1|C1)")
        if layer not in LAYERS:
            raise SystemExit(f"GATE product-floor: FAIL — {pid} layer must be model|atom|close, got {layer!r}")
        if subject_kind not in ("plan", "handler"):
            raise SystemExit(
                f"GATE product-floor: FAIL — {pid} subject_kind must be plan|handler, got {subject_kind!r}"
            )
        rows.append(
            {
                "id": pid,
                "layer": layer,
                "catalog_id": catalog_id,
                "theorem": theorem,
                "lean_file": lean_file,
                "entry": entry,
                "subject": subject,
                "subject_kind": subject_kind,
            }
        )
    if floor is None:
        raise SystemExit("GATE product-floor: FAIL — missing `floor_promoted N`")
    seen = [r["id"] for r in rows]
    if sorted(seen) != sorted(REQUIRED_IDS):
        raise SystemExit(f"GATE product-floor: FAIL — need exactly {REQUIRED_IDS}, got {seen}")
    return floor, rows


def theorem_errors(row: dict[str, str], catalog_ids: set[str]) -> list[str]:
    """Credit rule for atom/close. model rows skip (no close credit)."""
    tag = f"{row['id']} {row['layer']}"
    errs: list[str] = []
    cid = row["catalog_id"]
    if not cid.startswith("catalog:"):
        return [f"{tag}: catalog_id must be `catalog:<id>`, got {cid!r}"]
    cid = cid[len("catalog:") :]
    if cid not in catalog_ids:
        errs.append(f"{tag}: catalog:{cid} does not exist — dangling product claim")
    if row["layer"] == "model":
        return errs
    if row["theorem"] == "-" or row["lean_file"] == "-":
        errs.append(f"{tag}: atom/close row needs a theorem and lean_file, not `-`")
        return errs
    path = REPO / row["lean_file"]
    if not path.is_file():
        errs.append(f"{tag}: lean file {row['lean_file']} does not exist")
        return errs
    text = path.read_text(encoding="utf-8")
    if "sorry" in text:
        errs.append(f"{tag}: {row['lean_file']} contains `sorry` — no product credit on a sorry file")
    m = re.search(rf"\btheorem\s+{re.escape(row['theorem'])}\b(.*?):=", text, re.DOTALL)
    if not m:
        errs.append(f"{tag}: no `theorem {row['theorem']}` in {row['lean_file']}")
        return errs
    stmt = m.group(1)
    if "∀" not in stmt and "\\forall" not in stmt:
        errs.append(
            f"{tag}: statement has no forall-binder — product atom/close requires a named "
            "property over all inputs, not a definitional equality over concrete inputs"
        )
    if row["subject"] in WRAP_FACTORY_SUBJECTS:
        errs.append(
            f"{tag}: subject {row['subject']!r} is wrap-factory / 0166 modelo — "
            "RFC-0227 P2.7 refuses product credit"
        )
    if row["layer"] == "close" and row["subject_kind"] == "handler":
        proof = text[m.end() :]
        nxt = re.search(r"\ntheorem\s+", proof)
        body = proof[: nxt.start()] if nxt else proof
        subj = row["subject"]
        if not re.search(rf"\bunfold\b[^\n]*\b{re.escape(subj)}\b", body) and not re.search(
            rf"\bunfold\s+{re.escape(subj)}\b", stmt + "\n" + body
        ):
            # allow unfold of dotted `merge.get_live` etc.
            if not re.search(rf"\bunfold\b[^\n.]*\.{re.escape(subj)}\b", body):
                errs.append(
                    f"{tag}: close of handler subject {subj!r} must unfold that fn "
                    "(RFC-0227 P0.3)"
                )
    if row["id"] == "R1" and row["layer"] == "close":
        if not r1_close_stmt_ok(stmt, text):
            errs.append(
                f"{tag}: R1 close must state `get_live = ok true ↔` first covering "
                "Value ∧ ¬hidden (not walk=loop / visible_at iff / decided-keeps)"
            )
    if row["id"] == "D1" and row["layer"] == "close":
        if not d1_close_stmt_ok(stmt):
            errs.append(
                f"{tag}: D1 close must have an `env_honest = true` arm concluding "
                "`put_crash_reopen_survives = ok true` (honest acked put survives)"
            )
        mk = REPO / "crates/pedradb-core/src/merge_kernel.rs"
        rust = mk.read_text(encoding="utf-8") if mk.is_file() else ""
        if not d1_recover_threads_wal(rust):
            errs.append(
                f"{tag}: recover_collect_act still uses literals `1, false, 0, false` "
                "— prefix_n must be the put's n_records"
            )
    return errs


def r1_close_stmt_ok(stmt: str, lean_text: str = "") -> bool:
    """Left of ↔ is `get_live = ok true`; RHS names Value or first_covering_live."""
    if not re.search(
        r"get_live\s+(?:[A-Za-z0-9_.]+\s+){0,6}=\s*ok true\s*↔",
        stmt,
        re.S,
    ):
        return False
    if "Value" in stmt:
        return True
    if "first_covering_live" in stmt:
        return bool(
            re.search(r"def first_covering_live[\s\S]*?Value", lean_text)
        )
    return False


def d1_close_stmt_ok(stmt: str) -> bool:
    """Honest-Env arm concludes survival, not only lying→true / fence→false."""
    return bool(
        re.search(
            r"env_honest\s*=\s*true(?:(?!theorem).)*put_crash_reopen_survives"
            r"(?:(?!theorem).)*?=\s*ok true",
            stmt,
            re.S,
        )
    )


def d1_recover_threads_wal(rust: str) -> bool:
    """Only the production composer — tests may quote the banned literal."""
    m = re.search(
        r"pub fn put_crash_reopen_survives\b(.*?)pub fn put_crash_reopen_survives_as_is",
        rust,
        re.S,
    )
    body = m.group(1) if m else rust
    return not re.search(
        r"recover_collect_act\(\s*RecoverKind::Record\s*,\s*1\s*,\s*false\s*,"
        r"\s*0\s*,\s*false\s*\)",
        body,
    )


# RFC-0227 skeptic bags — hardcoded so S7/S8 stay red after the live theorems move.
_BAG_R1_STMT = r"""
    ∀ (kinds : Slice key.ValueType) (hiddens : Slice Bool),
      merge.get_live kinds hiddens
        = (do
            let i := Slice.len kinds
            let i1 := Slice.len hiddens
            let n ← core.cmp.Ord.min.trait_default core.cmp.OrdUsize i i1
            merge.get_live_loop kinds hiddens n 0#usize false false)
      ∧ (∀ (kind : key.ValueType) (range_hidden : Bool),
          merge.visible_at kind range_hidden = ok true
            ↔ (kind = key.ValueType.Value ∧ range_hidden = false))
      ∧ (∀ (n i : Usize) (live : Bool),
          merge.get_live_loop.body kinds hiddens n i live true
            = (if i < n then
                (do
                  let i1 ← i + 1#usize
                  ok (ControlFlow.cont (i1, live, true)))
               else ok (ControlFlow.done live)))
"""

_BAG_D1_STMT = r"""
    ∀ (n_records : U64) (commit_failed need_sync sync_fail env_honest : Bool)
      (synced written cut : U64)
      (kinds : Slice key.ValueType) (hiddens : Slice Bool),
      (env_honest = false →
        merge.put_crash_reopen_survives n_records commit_failed need_sync
          sync_fail env_honest synced written cut kinds hiddens = ok true)
      ∧ (env_honest = true → need_sync = true → sync_fail = true →
          commit_failed = false → n_records ≠ 0#u64 →
          merge.put_crash_reopen_survives n_records commit_failed need_sync
            sync_fail env_honest synced written cut kinds hiddens
            = ok false)
"""

_BAG_D1_RECOVER = (
    "let kept = recover_collect_act(RecoverKind::Record, 1, false, 0, false)"
)


def check(floor: int, rows: list[dict[str, str]], catalog_ids: set[str]) -> list[str]:
    errs: list[str] = []
    promoted = sum(1 for r in rows if r["layer"] in ("atom", "close"))
    if promoted < floor:
        errs.append(
            f"promoted {promoted} < floor_promoted {floor} — a product row descended "
            "to model (layer only moves UP; freeze day-1 needs ≥1 atom|close)"
        )
    for row in rows:
        errs.extend(theorem_errors(row, catalog_ids))
    return errs


def selftest() -> int:
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    catalog_ids = {p["id"] for p in catalog["pairs"]}
    floor, rows = parse_tsv(TSV.read_text(encoding="utf-8"))
    base = check(floor, rows, catalog_ids)
    if base:
        print("SELFTEST product-floor: state already inconsistent — fix first:")
        for e in base:
            print(f"  {e}")
        return 1

    caught = 0
    total = 8

    # S1: layer descent (R1 atom → model) drops promoted below floor.
    descended = [dict(r) for r in rows]
    for r in descended:
        if r["id"] == "R1":
            r["layer"] = "model"
            r["theorem"] = "-"
            r["lean_file"] = "-"
    if check(floor, descended, catalog_ids):
        print("SELFTEST product-floor: caught=layer-descent")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED layer-descent")

    # S2: atom/close theorem without a forall-binder.
    no_forall = [dict(r) for r in rows]
    for r in no_forall:
        if r["id"] == "R1":
            r["theorem"] = "visible_at_deletion"  # concrete rfl, no ∀
    if check(floor, no_forall, catalog_ids):
        print("SELFTEST product-floor: caught=no-forall")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED no-forall")

    # S3: dangling catalog id.
    dangling = [dict(r) for r in rows]
    for r in dangling:
        if r["id"] == "D1":
            r["catalog_id"] = "catalog:no_such_pair"
    if check(floor, dangling, catalog_ids):
        print("SELFTEST product-floor: caught=dangling-catalog")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED dangling-catalog")

    # S4: close of handler whose theorem does not unfold the subject.
    no_unfold = [dict(r) for r in rows]
    for r in no_unfold:
        if r["layer"] == "close" and r["subject_kind"] == "handler":
            r["theorem"] = "visible_at_deletion_never_live"  # ∀ but no unfold of subject
            r["lean_file"] = "formal/aeneas/lean/Merge.lean"
            r["subject"] = "get_live"
            break
    if any(r["layer"] == "close" and r["subject_kind"] == "handler" for r in no_unfold):
        if check(floor, no_unfold, catalog_ids):
            print("SELFTEST product-floor: caught=close-without-subject-unfold")
            caught += 1
        else:
            print("SELFTEST product-floor: MISSED close-without-subject-unfold")
    else:
        # no handler-close row yet — synthesise one
        synth = [dict(r) for r in rows]
        synth[0] = dict(synth[0])
        synth[0]["layer"] = "close"
        synth[0]["subject_kind"] = "handler"
        synth[0]["subject"] = "get_live"
        synth[0]["theorem"] = "visible_at_deletion_never_live"
        synth[0]["lean_file"] = "formal/aeneas/lean/Merge.lean"
        if check(floor, synth, catalog_ids):
            print("SELFTEST product-floor: caught=close-without-subject-unfold")
            caught += 1
        else:
            print("SELFTEST product-floor: MISSED close-without-subject-unfold")

    # S5: wrap-factory / 0166 modelo as subject.
    wrapped = [dict(r) for r in rows]
    for r in wrapped:
        if r["id"] == "D1":
            r["subject"] = "batch_is_empty"
            r["layer"] = "close"
            break
    if check(floor, wrapped, catalog_ids):
        print("SELFTEST product-floor: caught=wrap-factory-subject")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED wrap-factory-subject")

    # S6: honest freeze must PASS (checker not always-red).
    if check(floor, rows, catalog_ids) == []:
        print("SELFTEST product-floor: passed=honest-freeze (oracle not always-red)")
        caught += 1
    else:
        print("SELFTEST product-floor: honest freeze REJECTED — oracle always-red")

    # S7: bag R1 (walk=loop ∧ visible_at iff ∧ decided-keeps) is not a close.
    if not r1_close_stmt_ok(_BAG_R1_STMT, ""):
        print("SELFTEST product-floor: caught=r1-no-result-iff")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED r1-no-result-iff")

    # S8: bag D1 (lying→true ∧ fence→false, recover literals) is not a close.
    if not d1_close_stmt_ok(_BAG_D1_STMT) and not d1_recover_threads_wal(_BAG_D1_RECOVER):
        print("SELFTEST product-floor: caught=d1-no-honest-survive")
        caught += 1
    else:
        print("SELFTEST product-floor: MISSED d1-no-honest-survive")

    print(f"SELFTEST product-floor: {caught}/{total} checks caught")
    return 0 if caught == total else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--selftest", action="store_true", help="prove all red directions")
    args = ap.parse_args()
    if args.selftest:
        return selftest()

    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    catalog_ids = {p["id"] for p in catalog["pairs"]}
    floor, rows = parse_tsv(TSV.read_text(encoding="utf-8"))
    errs = check(floor, rows, catalog_ids)
    if errs:
        for e in errs:
            print(f"GATE product-floor: FAIL — {e}")
        print("GATE product-floor: RED")
        return 1
    layers = " ".join(f"{r['id']}={r['layer']}" for r in rows)
    promoted = sum(1 for r in rows if r["layer"] in ("atom", "close"))
    print(
        f"GATE product-floor: GREEN — {layers}, promoted={promoted}>=floor {floor}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
