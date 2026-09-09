#!/usr/bin/env python3
"""Pedra formal glue: caller lint, twin-token diff, clone drift, models, Verus.

The production kernel is the source of truth. A Verus file is a twin, not a
second implementation. This script does not prove anything — it refuses
silent drift between those copies, refuses a close twin that lacks the
entry fn, and refuses a kernel nobody in production calls. See
scripts/formal/README.md.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

FN_HEAD = re.compile(
    r"(?P<pre>(?:pub\s+)?(?:open\s+)?(?:spec\s+)?(?:proof\s+)?)fn\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*[<(]",
)
TOKEN_RE = re.compile(
    r"0x[0-9a-fA-F]+|\b\d+u(?:8|16|32|64)?\b|\b\d+\b|\btrue\b|\bfalse\b|"
    r"==|!=|<=|>=|&&|\|\||<(?!<)|>(?!>)|"
    r"::[A-Z][A-Za-z0-9_]*"
)
SKIP_FN = re.compile(r"(_as_is|_spec)$")

PUB_FN_HEAD = re.compile(
    r"\bpub(?:\([^)]*\))?\s+(?:unsafe\s+)?(?:const\s+)?fn\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)


def strip_comments(src: str) -> str:
    """Drop // and /* */ comments without touching // inside strings."""
    out: list[str] = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c in "\"'":
            quote = c
            out.append(c)
            i += 1
            while i < n:
                out.append(src[i])
                if src[i] == "\\" and i + 1 < n:
                    out.append(src[i + 1])
                    i += 2
                    continue
                if src[i] == quote:
                    i += 1
                    break
                i += 1
            continue
        if c == "/" and nxt == "/":
            i += 2
            while i < n and src[i] != "\n":
                i += 1
            continue
        if c == "/" and nxt == "*":
            i += 2
            while i + 1 < n and not (src[i] == "*" and src[i + 1] == "/"):
                i += 1
            i = min(n, i + 2)
            continue
        out.append(c)
        i += 1
    return "".join(out)


def match_braces(src: str, open_at: int) -> int:
    depth = 0
    i = open_at
    n = len(src)
    while i < n:
        c = src[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        elif c == '"':
            i += 1
            while i < n and src[i] != '"':
                if src[i] == "\\":
                    i += 1
                i += 1
        i += 1
    raise ValueError("unbalanced braces")


def strip_cfg_test_mods(src: str) -> str:
    """Drop `#[cfg(test)]` modules so test helpers are not proved exec.

    RFC-0171 single-artifact kernels keep plants in the production file;
    char literals / format `{}` in those helpers are not the twin term.
    Unparsable trailing tests are truncated from the marker (tests last).
    """
    text = src
    marker = "#[cfg(test)]"
    while True:
        i = text.find(marker)
        if i < 0:
            return text
        brace = text.find("{", i)
        if brace < 0:
            return text[:i]
        try:
            end = match_braces(text, brace)
        except ValueError:
            return text[:i]
        text = text[:i] + text[end + 1 :]


def iter_fns(src: str):
    text = strip_cfg_test_mods(strip_comments(src))
    for m in FN_HEAD.finditer(text):
        pre = m.group("pre") or ""
        name = m.group("name")
        kind = "exec"
        if "spec" in pre.split():
            kind = "spec"
        elif "proof" in pre.split():
            kind = "proof"
        brace = text.find("{", m.end() - 1)
        if brace < 0:
            continue
        end = match_braces(text, brace)
        yield kind, name, text[brace + 1 : end]


def exec_fns(src: str) -> dict[str, str]:
    out = {}
    for kind, name, body in iter_fns(src):
        if kind != "exec" or SKIP_FN.search(name):
            continue
        out[name] = body
    return out


def spec_fns(src: str) -> dict[str, str]:
    out = {}
    for kind, name, body in iter_fns(src):
        if kind != "spec":
            continue
        out[name] = body
    return out


def tokens(body: str) -> list[str]:
    out = []
    for t in TOKEN_RE.findall(body):
        m = re.match(r"(\d+)u(?:8|16|32|64)?$", t)
        out.append(m.group(1) if m else t)
    return out


def prod_fn_offsets(src: str) -> tuple[dict[str, int], int]:
    """Exec-fn head offsets + offset of the first #[cfg(test)] module."""
    text = strip_comments(src)
    cfg = text.find("#[cfg(test)]")
    if cfg < 0:
        cfg = len(text)
    offs: dict[str, int] = {}
    for m in FN_HEAD.finditer(text):
        pre = (m.group("pre") or "").split()
        if "spec" in pre or "proof" in pre:
            continue
        offs.setdefault(m.group("name"), m.start())
    return offs, cfg


def load_text(root: Path, rel: str) -> str | None:
    p = root / rel
    if not p.is_file():
        return None
    return p.read_text(encoding="utf-8")


def mentions(src: str, symbol: str) -> bool:
    return (
        re.search(r"\b" + re.escape(symbol) + r"\s*\(", strip_comments(src))
        is not None
    )


def find_verus(root: Path) -> str | None:
    env = os.environ.get("VERUS")
    if env and os.path.isfile(env) and os.access(env, os.X_OK):
        return env
    home = Path.home() / ".local/verus/verus-arm64-macos/verus"
    if home.is_file() and os.access(home, os.X_OK):
        return str(home)
    which = subprocess.run(
        "command -v verus", shell=True, capture_output=True, text=True
    )
    path = which.stdout.strip()
    return path or None


class Report:
    def __init__(self) -> None:
        self.failed: list[str] = []
        self.gaps: list[str] = []
        self.ok: list[str] = []

    def fail(self, msg: str) -> None:
        self.failed.append(msg)
        print(f"FAIL  {msg}")

    def gap(self, msg: str) -> None:
        self.gaps.append(msg)
        print(f"GAP   {msg}")

    def good(self, msg: str) -> None:
        self.ok.append(msg)
        print(f"ok    {msg}")


def check_lint(root: Path, catalog: dict, r: Report) -> None:
    print("== lint (production must call the kernel) ==")
    for pair in catalog["pairs"]:
        entry = pair.get("entry")
        if not entry:
            continue
        kernel = pair["kernel"]
        for caller in pair.get("callers", []):
            src = load_text(root, caller)
            if src is None:
                r.fail(f"{pair['id']}: missing caller {caller}")
                continue
            if not mentions(src, entry):
                r.fail(f"{pair['id']}: {caller} does not call {entry}()")
            elif caller == kernel:
                r.good(f"{pair['id']}: {entry} lives and is used in {caller}")
            else:
                r.good(f"{pair['id']}: {caller} calls {entry}")
            if pair.get("data_fate"):
                for handler in pair.get("handlers", []):
                    if not mentions(src, handler):
                        r.fail(
                            f"{pair['id']}: data_fate handler {handler} missing in {caller} "
                            f"(RFC-0053 P0.4)"
                        )
                    else:
                        r.good(f"{pair['id']}: data_fate handler {handler} in {caller}")
        # RFC-0152: store live path must invoke the catalog entry without
        # inheriting raft handler names (those stay on `callers`).
        for lc in pair.get("live_callers") or []:
            if not isinstance(lc, dict):
                r.fail(f"{pair['id']}: live_callers entry must be {{file, handler}}")
                continue
            live_file = lc.get("file") or ""
            live_handler = lc.get("handler") or ""
            lsrc = load_text(root, live_file)
            if lsrc is None:
                r.fail(f"{pair['id']}: missing live_caller {live_file}")
                continue
            if not mentions(lsrc, entry):
                r.fail(f"{pair['id']}: {live_file} does not call {entry}()")
            else:
                r.good(f"{pair['id']}: live_caller {live_file} calls {entry}")
            if live_handler:
                if not mentions(lsrc, live_handler):
                    r.fail(
                        f"{pair['id']}: live handler {live_handler} missing in {live_file}"
                    )
                else:
                    r.good(f"{pair['id']}: live handler {live_handler} in {live_file}")
    check_store_live(root, catalog, r)
    check_raft_store_live(root, catalog, r)


# RFC-0152: these catalog kernels must be the store live RV/AE path.
STORE_LIVE_KERNELS = {
    "vote": ("crates/pedradb-store/src/lib.rs", "on_request_vote"),
    "ae_entry": ("crates/pedradb-store/src/lib.rs", "on_append_entries"),
    "grant_persist": ("crates/pedradb-store/src/lib.rs", "on_request_vote"),
    "ae_ack": ("crates/pedradb-store/src/lib.rs", "on_append_entries"),
    "commit_raft": ("crates/pedradb-store/src/lib.rs", "broadcast_append_after_propose"),
    "joint_election": ("crates/pedradb-store/src/lib.rs", "election_has_joint_quorum"),
    "joint_leave": ("crates/pedradb-store/src/lib.rs", "pending_joint_on"),
    "pending_joint_node": ("crates/pedradb-store/src/lib.rs", "pending_joint"),
    "joint_leave_ok": ("crates/pedradb-store/src/lib.rs", "leave_joint"),
    "election_grant_from": ("crates/pedradb-store/src/lib.rs", "on_request_vote_reply"),
    "joint_target": ("crates/pedradb-store/src/lib.rs", "remove_member_joint"),
    "joint_add_target": ("crates/pedradb-store/src/lib.rs", "add_member_joint"),
    "queued_leave_finish": ("crates/pedradb-store/src/lib.rs", "finish_uncommitted_leave"),
    "disk_membership": ("crates/pedradb-store/src/lib.rs", "bind_cluster_identity"),
    "high_water": ("crates/pedradb-store/src/lib.rs", "open_single_node_with_rng_opts"),
    "participating_member": ("crates/pedradb-store/src/lib.rs", "is_participating"),
    "identity_before_applied": ("crates/pedradb-store/src/lib.rs", "apply_range"),
    "recover_apply": ("crates/pedradb-store/src/lib.rs", "recover_apply_committed"),
    "recover_apply_node": ("crates/pedradb-store/src/lib.rs", "recover_apply_committed"),
    "recover_truncate": ("crates/pedradb-store/src/lib.rs", "persist_truncated_logs"),
    "recover_drop_orphan": ("crates/pedradb-store/src/lib.rs", "persist_log_db"),
    "recover_abort": ("crates/pedradb-store/src/lib.rs", "abort_leftover_intents"),
    "persist_meta": ("crates/pedradb-store/src/lib.rs", "persist_u64_meta_all"),
    "persist_hist": ("crates/pedradb-store/src/lib.rs", "persist_si_keys"),
    "persist_fence": ("crates/pedradb-store/src/lib.rs", "fence_txn_aborted"),
    "force_clear": ("crates/pedradb-store/src/lib.rs", "force_local_clear_keys"),
    "drop_preimages": ("crates/pedradb-store/src/lib.rs", "drop_preimages"),
    "open_peer_disk": ("crates/pedradb-store/src/lib.rs", "open_with_envs_rng_opts"),
    "local_id_member": ("crates/pedradb-store/src/lib.rs", "local_node_id"),
    "reader_local": ("crates/pedradb-store/src/lib.rs", "ids_first_if_local"),
    "discard_uncommitted": ("crates/pedradb-store/src/lib.rs", "discard_uncommitted_from"),
    "discard_leader": ("crates/pedradb-store/src/lib.rs", "finish_queued_propose"),
    "removed_step_down": ("crates/pedradb-store/src/lib.rs", "install_applied_membership"),
    "hint_member": ("crates/pedradb-store/src/lib.rs", "leader_hint"),
    "drop_repl_slot": ("crates/pedradb-store/src/lib.rs", "install_applied_membership"),
    "drop_sent_through": ("crates/pedradb-store/src/lib.rs", "remove_member"),
    "apply_step": ("crates/pedradb-store/src/lib.rs", "apply_range"),
}

STORE_LIVE_PATH = "crates/pedradb-store/src/lib.rs"


def check_store_live(_root: Path, catalog: dict, r: Report) -> None:
    """Refuse a vote/ae_entry catalog that is not wired through store live RPC."""
    print("== store live (RFC-0152: queued RV/AE is the catalog kernel) ==")
    ids = {p["id"]: p for p in catalog["pairs"]}
    for pid, (path, handler) in STORE_LIVE_KERNELS.items():
        pair = ids.get(pid)
        if pair is None:
            r.fail(f"{pid}: catalog pair missing (RFC-0152)")
            continue
        lcs = pair.get("live_callers") or []
        hit = next(
            (
                lc
                for lc in lcs
                if isinstance(lc, dict) and lc.get("file") == path
            ),
            None,
        )
        if hit is None:
            r.fail(f"{pid}: missing live_callers {path}")
            continue
        if hit.get("handler") != handler:
            r.fail(f"{pid}: live handler must be {handler}")
        else:
            r.good(f"{pid}: live_callers {path} / {handler}")


def check_raft_store_live(root: Path, catalog: dict, r: Report) -> None:
    """Raft-kernel data_fate pair that store lib.rs calls must list live_callers."""
    print("== raft→store live_callers (RFC-0152 C) ==")
    store = load_text(root, STORE_LIVE_PATH)
    if store is None:
        r.fail(f"missing {STORE_LIVE_PATH}")
        return
    for pair in catalog["pairs"]:
        kernel = pair.get("kernel") or ""
        if not kernel.startswith("crates/pedradb-raft"):
            continue
        if not pair.get("data_fate"):
            continue
        entry = pair.get("entry") or ""
        if not entry or not mentions(store, entry):
            continue
        pid = pair["id"]
        lcs = pair.get("live_callers") or []
        hit = next(
            (
                lc
                for lc in lcs
                if isinstance(lc, dict) and lc.get("file") == STORE_LIVE_PATH
            ),
            None,
        )
        if hit is None:
            r.fail(f"{pid}: missing live_callers {STORE_LIVE_PATH}")
        else:
            r.good(f"{pid}: live_callers {STORE_LIVE_PATH} / {hit.get('handler')}")


def check_three_teeth(root: Path, catalog: dict, r: Report) -> None:
    """RFC-0151: every data_fate pair has AS-IS + twin + named DST plant.
    RFC-0152 P2: pairs with three_teeth=true (non-data_fate) too."""
    print("== three teeth (RFC-0151: AS-IS + twin + named DST plant) ==")
    before = len(r.failed)
    for pair in catalog["pairs"]:
        if not pair.get("data_fate") and not pair.get("three_teeth"):
            continue
        pid = pair["id"]
        entry = pair.get("entry") or ""
        ksrc = load_text(root, pair.get("kernel") or "")
        as_is = pair.get("as_is")
        if not isinstance(as_is, str) or not as_is.strip():
            r.fail(f"three teeth: {pid} missing as_is")
        elif ksrc is None:
            r.fail(f"three teeth: {pid} missing kernel for as_is")
        elif re.search(r"\bfn\s+" + re.escape(as_is) + r"(\s*<[^>]*>)?\s*\(", ksrc) is None:
            r.fail(f"three teeth: {pid} as_is {as_is} not in kernel")
        else:
            r.good(f"three teeth: {pid} as_is {as_is}")
        plant = pair.get("dst_plant")
        if not isinstance(plant, dict):
            r.fail(f"three teeth: {pid} missing dst_plant")
            continue
        pfile = plant.get("file")
        ptest = plant.get("test")
        if not pfile or not ptest:
            r.fail(f"three teeth: {pid} dst_plant needs file+test")
            continue
        psrc = load_text(root, pfile)
        if psrc is None:
            r.fail(f"three teeth: {pid} dst_plant file missing {pfile}")
            continue
        if re.search(r"\bfn\s+" + re.escape(ptest) + r"\s*\(", psrc) is None:
            r.fail(f"three teeth: {pid} dst_plant test {ptest} missing in {pfile}")
        elif entry and not mentions(psrc, entry):
            r.fail(f"three teeth: {pid} dst_plant does not mention {entry}()")
        else:
            r.good(f"three teeth: {pid} plant {ptest}")
        kernel = pair.get("kernel") or ""
        is_raft = "pedradb-raft" in kernel or str(pid).startswith("l28_")
        if is_raft and psrc is not None:
            if "pin_dst_queued" not in psrc and "RpcMode::Queued" not in psrc:
                r.fail(f"three teeth: {pid} raft plant must pin Queued RPC")
    if len(r.failed) == before:
        n = sum(
            1
            for p in catalog["pairs"]
            if p.get("data_fate") or p.get("three_teeth")
        )
        r.good(f"three teeth: {n} pairs")


def check_clones(root: Path, catalog: dict, r: Report) -> None:
    print("== clones (duplicated production kernels) ==")
    for clone in catalog.get("clones", []):
        a = load_text(root, clone["a"])
        b = load_text(root, clone["b"])
        if a is None or b is None:
            r.fail(f"{clone['id']}: missing {clone['a'] if a is None else clone['b']}")
            continue
        af, bf = exec_fns(a), exec_fns(b)
        for name in clone["fns"]:
            if name not in af or name not in bf:
                r.fail(f"{clone['id']}: {name} missing in one side")
                continue
            ta, tb = tokens(af[name]), tokens(bf[name])
            if ta != tb:
                r.fail(
                    f"{clone['id']}: {name} drifted ({ta} vs {tb})"
                )
            else:
                r.good(f"{clone['id']}: {name} identical tokens")
        # Anti-silence: a token-identical production fn duplicated across
        # the pair must be registered above (or deliberately diverged);
        # test-module fns are out of scope.
        oa, ca = prod_fn_offsets(a)
        ob, cb = prod_fn_offsets(b)
        hidden = sorted(
            name
            for name in (set(af) & set(bf)) - set(clone["fns"])
            if oa.get(name, ca) < ca and ob.get(name, cb) < cb
            and tokens(af[name]) == tokens(bf[name])
        )
        if hidden:
            r.fail(
                f"{clone['id']}: unregistered identical production clone "
                f"fn(s) {hidden} — add to catalog clones or diverge them"
            )
        else:
            r.good(f"{clone['id']}: no unregistered identical production fn")
        # Anti-silence (name level, 2026-08-31): a production fn NAME shared
        # across the pair must be registered as identical above or recorded
        # in "diverged" with a reason. Exact-token equality alone fails open
        # on near-clones — a new fn added to both sides with slightly
        # different bodies would stay silent forever.
        diverged = clone.get("diverged") or {}
        if not isinstance(diverged, dict):
            r.fail(f"{clone['id']}: diverged must be an object {{fn: reason}}")
            diverged = {}
        shared_prod = {
            name
            for name in set(af) & set(bf)
            if oa.get(name, ca) < ca and ob.get(name, cb) < cb
        }
        unlisted = sorted((shared_prod - set(clone["fns"])) - set(diverged))
        if unlisted:
            r.fail(
                f"{clone['id']}: shared production fn(s) {unlisted} neither "
                "registered identical nor recorded diverged — add to fns or "
                "to diverged with a reason"
            )
        for name in sorted(set(diverged) - shared_prod):
            r.fail(f"{clone['id']}: stale diverged entry {name} (not a shared production fn)")
        for name in sorted(set(diverged) & set(clone["fns"])):
            r.fail(f"{clone['id']}: {name} listed in both fns and diverged")
        if diverged and unlisted == [] and not (set(diverged) - shared_prod):
            r.good(f"{clone['id']}: {len(diverged)} recorded divergence(s)")
    # Cross-pair completeness: the clone catalog must cover EVERY
    # token-identical production fn duplicated across the frozen kernels
    # (plus the catalog's own non-kernel sides). A new duplicate must be
    # registered or deliberately diverged, never silently added.
    files = [str(p.relative_to(root)) for p in decision_kernel_paths(root)]
    sides = {clone["a"] for clone in catalog.get("clones", [])} | {
        clone["b"] for clone in catalog.get("clones", [])
    }
    files += sorted(sides - set(files))
    info: dict[tuple[str, str], list[str]] = {}
    by_name: dict[str, list[str]] = {}
    for rel in files:
        src = load_text(root, rel)
        if src is None:
            continue
        offs, cfg = prod_fn_offsets(src)
        for name, body in exec_fns(src).items():
            if offs.get(name, cfg) >= cfg:
                continue
            info[(rel, name)] = tokens(body)
            by_name.setdefault(name, []).append(rel)
    covered = {
        (cl["a"], cl["b"], n)
        for cl in catalog.get("clones", [])
        for n in cl["fns"]
    } | {
        (cl["b"], cl["a"], n)
        for cl in catalog.get("clones", [])
        for n in cl["fns"]
    }
    missed = [
        f"{name}: {fs[i]} <-> {fs[j]}"
        for name, fs in sorted(by_name.items())
        for i in range(len(fs))
        for j in range(i + 1, len(fs))
        if info[(fs[i], name)] == info[(fs[j], name)]
        and (fs[i], fs[j], name) not in covered
    ]
    for m in missed:
        r.fail(
            f"clones catalog incomplete: unregistered identical "
            f"production pair {m} — register or diverge"
        )
    if not missed:
        r.good(
            f"clone catalog complete: {len(covered) // 2} registered fns "
            f"cover every identical production pair across {len(files)} files"
        )


# ---------------------------------------------------------------------------
# RFC-0056 P2.5 — TCB freeze.
# ---------------------------------------------------------------------------

# Explicit exceptions: production decision kernels that are NOT catalog
# pairs. Adding a line here is a visible diff a reviewer must justify;
# anything not listed and not registered turns CI red. TCB cannot grow in
# silence.
#
# RFC-0074 P2.1 registered `cqe_kernel.rs` as catalog pair `cqe_res`
# (entry `cqe_res_ok`). That is not a ring model (P2.2 / R-uring):
# `cqe_ring_model_admitted` stays false; do not add `verus/ring_model.rs`.
#
# 2026-08-31: leveling.rs graduated from this allowlist into catalog pairs
# `leveling` (close) + `leveling_pick` (atom) — twins
# verus/leveling{,_pick}.rs, plant in src/leveling.rs (findings/
# 2026-08-31-leveling-kernel-unenrolled). The allowlist is empty; keep it
# that way (transitional states get a comment, not a permanent row).
TCB_FREEZE_ALLOWLIST: dict[str, str] = {
    "crates/pedradb-core/src/disk_pressure_kernel.rs":
        "RFC-0179 Aeneas extract SOURCE.disk_pressure (catalog twin pending)",
}

# RFC-0166 P2.4: catalog accounting. `l28_*` is a campaign gate (named
# plant / seed replay, not ∀ traces). Everything else defaults to a
# proof object (machine-checked twin). Pair may override with `"object"`.
OBJECT_KINDS = {"proof", "campaign"}
PROPERTY_PROOF_IDS = frozenset(
    {
        "d1_durability",
        "d1_modelo",
        "r1_no_resurrection",
        "r1_modelo",
        "t1_atomicity",
        "t1_modelo",
        "c1_quorum",
        "c1_modelo",
    }
)


def pair_object(pair: dict, prefixes: list[str]) -> str:
    """proof | campaign for one catalog pair (explicit, else prefix, else proof)."""
    explicit = pair.get("object")
    if isinstance(explicit, str) and explicit.strip():
        return explicit.strip()
    pid = pair.get("id") or ""
    if any(pid.startswith(p) for p in prefixes):
        return "campaign"
    return "proof"


def check_proof_vs_campaign(_root: Path, catalog: dict, r: Report) -> None:
    """RFC-0166 P2.4: every pair is a proof object or a campaign gate."""
    print("== proof vs campaign (RFC-0166 P2.4) ==")
    kinds = catalog.get("object_kinds")
    if not isinstance(kinds, dict) or set(kinds) != OBJECT_KINDS:
        r.fail("catalog object_kinds must map both 'proof' and 'campaign'")
        return
    prefixes = catalog.get("campaign_prefixes")
    if not isinstance(prefixes, list) or "l28_" not in prefixes:
        r.fail("catalog campaign_prefixes must be a list including 'l28_'")
        return
    prefixes = [p for p in prefixes if isinstance(p, str) and p]
    n_proof = 0
    n_campaign = 0
    before = len(r.failed)
    for pair in catalog.get("pairs") or []:
        pid = pair.get("id") or "<missing-id>"
        obj = pair_object(pair, prefixes)
        if obj not in OBJECT_KINDS:
            r.fail(f"{pid}: object must be proof|campaign (got {obj!r})")
            continue
        if pid.startswith("l28_") and obj != "campaign":
            r.fail(f"{pid}: l28_* is a campaign gate, not a proof object")
            continue
        if pid in PROPERTY_PROOF_IDS and obj != "proof":
            r.fail(f"{pid}: D1/R1/T1/C1 refinement is a proof object")
            continue
        if obj == "campaign":
            n_campaign += 1
        else:
            n_proof += 1
    if len(r.failed) == before:
        r.good(
            f"proof vs campaign: {n_proof} proof objects, {n_campaign} campaign gates"
        )


ISLAND_CRATES = ("pedradb-posix", "pedradb-io-uring", "pedradb-capi")
RFC_0061 = "docs/rfc/0061-residuals-sel4-ironfleet.md"


def enrolled_kernel_registry(root: Path) -> set[str]:
    """Explicit kernel enrollment (residuals.json glue.kernel_paths).

    The *_kernel.rs glob is a naming convention, and conventions fail open:
    leveling.rs (2026-08-31) shipped as a pure decision kernel invisible to
    the glob. Discovery is therefore glob UNION registry, and the residuals
    freeze enforces the tooth in both directions (check_kernel_enrollment).
    A missing/invalid residuals.json yields an empty registry; that hole is
    closed by check_residuals, which fails on the same file.
    """
    try:
        data = json.loads((root / "scripts/formal/residuals.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return set()
    glue = data.get("glue")
    paths = glue.get("kernel_paths") if isinstance(glue, dict) else None
    if not isinstance(paths, list):
        return set()
    return {p for p in paths if isinstance(p, str)}


def kernel_marker(path: Path) -> str | None:
    """First `//! kernel:` marker line in the file, if any."""
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    for line in text.splitlines():
        if line.startswith("//! kernel:"):
            return line
    return None


def marked_kernel_files(root: Path) -> set[str]:
    """Non-verus crate sources carrying a `//! kernel:` marker."""
    return {
        str(p.relative_to(root))
        for p in (root / "crates").glob("*/src/**/*.rs")
        if p.is_file() and "verus" not in p.parts and kernel_marker(p) is not None
    }


def decision_kernel_paths(root: Path) -> list[Path]:
    globbed = {
        str(p.relative_to(root))
        for p in (root / "crates").glob("*/src/**/*_kernel.rs")
        if p.is_file() and "verus" not in p.parts
    }
    enrolled = {rel for rel in enrolled_kernel_registry(root) if (root / rel).is_file()}
    return [root / rel for rel in sorted(globbed | enrolled)]


def check_kernel_enrollment(root: Path, glue: dict, r: Report) -> None:
    """Marker<->registry tooth (findings/2026-08-31-leveling-kernel-unenrolled).

    The registry is the explicit side; the `//! kernel:` marker is the
    in-file side. Both directions fail closed:
      - every glob-discovered kernel must be enrolled;
      - every enrolled path must exist and sit in non-verus crates/*/src;
      - suffix-less enrolled paths must carry the marker;
      - every marked file must be enrolled.
    """
    before = len(r.failed)
    raw_paths = glue.get("kernel_paths")
    if not isinstance(raw_paths, list) or not raw_paths:
        r.fail(
            "residuals freeze: glue.kernel_paths list required (explicit "
            "kernel enrollment — findings/2026-08-31-leveling-kernel-unenrolled)"
        )
        return
    bad = [p for p in raw_paths if not isinstance(p, str) or not p.strip()]
    if bad:
        r.fail(f"residuals freeze: glue.kernel_paths entries must be non-empty strings: {bad[:3]}")
    raw = [p for p in raw_paths if isinstance(p, str) and p.strip()]
    if len(set(raw)) != len(raw):
        r.fail(f"residuals freeze: glue.kernel_paths duplicates {sorted({p for p in raw if raw.count(p) > 1})}")
    missing = [p for p in raw if not (root / p).is_file()]
    for p in missing:
        r.fail(f"residuals freeze: enrolled kernel {p} is not a file")
    existing = {p for p in raw if (root / p).is_file()}
    globbed = {
        str(p.relative_to(root))
        for p in (root / "crates").glob("*/src/**/*_kernel.rs")
        if p.is_file() and "verus" not in p.parts
    }
    for p in sorted(globbed - existing):
        r.fail(
            f"residuals freeze: kernel {p} matches the *_kernel.rs glob but "
            "is not enrolled in glue.kernel_paths (register it)"
        )
    stray = sorted(
        p for p in existing
        if not p.startswith("crates/") or "verus" in Path(p).parts
    )
    for p in stray:
        r.fail(f"residuals freeze: enrolled kernel {p} is outside non-verus crates/*/src")
    suffix_less = sorted(existing - globbed)
    for p in suffix_less:
        if kernel_marker(root / p) is None:
            r.fail(
                f"residuals freeze: enrolled kernel {p} is suffix-less "
                "(*_kernel.rs) and carries no `//! kernel:` marker line"
            )
    for p in sorted(marked_kernel_files(root) - existing):
        r.fail(f"residuals freeze: {p} carries `//! kernel:` but is not enrolled in glue.kernel_paths")
    if len(r.failed) == before:
        r.good(
            f"kernel enrollment: {len(existing)} kernels, "
            f"{len(suffix_less)} suffix-less via `//! kernel:` marker"
        )


def kernel_pub_fns(path: Path) -> set[str]:
    """pub / pub(crate) fn names in a kernel file (comments stripped)."""
    try:
        text = strip_comments(path.read_text(encoding="utf-8", errors="replace"))
    except OSError:
        return set()
    return {m.group("name") for m in PUB_FN_HEAD.finditer(text)}


def check_kernel_fn_surface(root: Path, catalog: dict, glue: dict, r: Report) -> None:
    """Fn-level enrollment tooth (findings/2026-08-31-leveling-kernel-unenrolled).

    Enrollment is FILE-level (glob union registry), so a new pub decision fn
    added inside an already-enrolled kernel file is invisible to the sweep
    and to the teeth (they check only cataloged entries). Fail-closed at the
    fn level: every pub/pub(crate) fn in an enrolled kernel must be
      - a catalog entry (pair whose kernel is this file), or
      - a catalog as_is mutant (exact name or the `_as_is` convention), or
      - a `_spec` kernel-side spec twin, or
      - a catalog clone fn for a clone side on this file, or
      - in glue.kernel_fn_allowlist (path -> fn names): the date-stamped
        baseline of not-yet-classified fns; every entry must eventually
        graduate to a real class or a catalog gap.
    Stale and auto-classifiable baseline entries fail, so the baseline can
    only shrink honestly.
    """
    before = len(r.failed)
    entries: dict[str, set[str]] = {}
    as_is: set[str] = set()
    for pair in catalog.get("pairs", []):
        if pair.get("status") == "absent" or not pair.get("kernel"):
            continue
        entries.setdefault(pair["kernel"], set()).add(pair["entry"])
        if pair.get("as_is"):
            as_is.add(pair["as_is"])
    clones: dict[str, set[str]] = {}
    for clone in catalog.get("clones", []):
        for side in ("a", "b"):
            clones.setdefault(clone[side], set()).update(clone.get("fns", []))
    raw_allow = glue.get("kernel_fn_allowlist")
    if not isinstance(raw_allow, dict):
        r.fail(
            "residuals freeze: glue.kernel_fn_allowlist object required "
            "(fn-level enrollment tooth — findings/2026-08-31-leveling-kernel-unenrolled)"
        )
        raw_allow = {}
    allow: dict[str, set[str]] = {}
    for k, names in raw_allow.items():
        if not isinstance(names, list) or not all(
            isinstance(n, str) and n.strip() for n in names
        ):
            r.fail(f"residuals freeze: kernel_fn_allowlist[{k}] must be a list of fn names")
            continue
        dupes = sorted({n for n in names if names.count(n) > 1})
        if dupes:
            r.fail(f"residuals freeze: kernel_fn_allowlist[{k}] duplicates {dupes}")
        allow[k] = set(names)
    enrolled = {str(p.relative_to(root)) for p in decision_kernel_paths(root)}
    for k in sorted(allow):
        if k not in enrolled:
            r.fail(f"residuals freeze: kernel_fn_allowlist key {k} is not an enrolled kernel")
    n_classified = n_baseline = 0
    for rel in sorted(enrolled):
        names = kernel_pub_fns(root / rel)
        auto = entries.get(rel, set()) | clones.get(rel, set()) | as_is
        for name in sorted(names):
            if name in auto or "_as_is" in name or name.endswith("_spec"):
                n_classified += 1
                if name in allow.get(rel, set()):
                    r.fail(
                        f"residuals freeze: {rel}: baseline entry {name} is "
                        "auto-classified (entry/as_is/clone) — remove it"
                    )
            elif name in allow.get(rel, set()):
                n_baseline += 1
            else:
                r.fail(
                    f"residuals freeze: {rel}: pub fn {name} is outside the fn "
                    "surface (no entry/as_is/_as_is/_spec/clone, not in "
                    "glue.kernel_fn_allowlist) — classify it "
                    "(findings/2026-08-31-leveling-kernel-unenrolled)"
                )
        for name in sorted(allow.get(rel, set()) - names):
            r.fail(f"residuals freeze: {rel}: stale kernel_fn_allowlist entry {name} (fn gone)")
    if len(r.failed) == before:
        r.good(
            f"kernel fn surface: {n_classified + n_baseline} pub fns in "
            f"{len(enrolled)} kernels — {n_classified} classified "
            f"(entry/as_is/clone), {n_baseline} baseline allowlisted"
        )


def file_loc(path: Path) -> int:
    return len(path.read_text(encoding="utf-8", errors="replace").splitlines())


def glue_loc(root: Path, catalog: dict) -> tuple[int, int, int]:
    """Kernel-file count, kernel LOC, unique-caller handler LOC."""
    kernels = decision_kernel_paths(root)
    kernel_loc = sum(file_loc(p) for p in kernels)
    handlers: set[str] = set()
    for pair in catalog.get("pairs", []):
        for c in pair.get("callers") or []:
            handlers.add(c)
    handler_loc = 0
    for rel in handlers:
        p = root / rel
        if p.is_file():
            handler_loc += file_loc(p)
    return len(kernels), kernel_loc, handler_loc


def check_tcb_freeze(root: Path, catalog: dict, r: Report) -> None:
    print("== tcb freeze (RFC-0056 P2.5) ==")
    registered = {p["kernel"] for p in catalog["pairs"] if p.get("status") != "absent"}
    for clone in catalog.get("clones", []):
        registered.add(clone["a"])
        registered.add(clone["b"])
    frozen = [str(p.relative_to(root)) for p in decision_kernel_paths(root)]
    before = len(r.failed)
    for k in frozen:
        if k in registered:
            continue
        if k in TCB_FREEZE_ALLOWLIST:
            r.good(f"tcb freeze: allowlisted {k} — {TCB_FREEZE_ALLOWLIST[k]}")
            continue
        r.fail(
            f"tcb freeze: kernel {k} is neither a catalog pair, a catalog "
            "clone, nor allowlisted — new TCB must register kernel+twin "
            "(RFC-0056 P2.5)"
        )
    for k in TCB_FREEZE_ALLOWLIST:
        if k not in frozen:
            r.fail(f"tcb freeze: stale allowlist entry {k} (file gone)")
    # data_fate pairs are the data-destination TCB: each must carry the
    # full kernel+twin+script triple.
    for pair in catalog["pairs"]:
        if not pair.get("data_fate"):
            continue
        # Payment is machine-checked by a Verus twin OR an Aeneas extract of
        # the linked rustc body (RFC-0171 P0.3: a cfg-split Verus stand-in
        # whose body can drift from the rustc body is not a payment).
        paid = pair.get("verus") or pair.get("aeneas")
        missing = [
            what
            for what, val in (("twin", pair.get("twin")), ("proof script (verus|aeneas)", paid))
            if not val or not (root / val).is_file()
        ]
        if missing:
            r.fail(f"tcb freeze: data_fate pair {pair['id']} lacks {', '.join(missing)}")
    if len(r.failed) == before:
        r.good(f"tcb freeze: {len(frozen)} decision kernels accounted for")


RESIDUAL_CLASSES = {"never", "continuous", "parked", "open"}


def check_residuals(
    root: Path,
    r: Report,
    catalog: dict | None = None,
    residuals_path: Path | None = None,
    rfc_path: Path | None = None,
) -> None:
    """RFC-0061: residual catalog is well-formed; scripts exist; never-floor holds."""
    print("== residuals freeze (RFC-0061) ==")
    path = residuals_path or (root / "scripts/formal/residuals.json")
    rfc = rfc_path or (root / RFC_0061)
    before = len(r.failed)
    if not path.is_file():
        r.fail("residuals freeze: missing scripts/formal/residuals.json")
        return
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        r.fail(f"residuals freeze: invalid JSON: {e}")
        return
    rows = data.get("residuals")
    if not isinstance(rows, list) or not rows:
        r.fail("residuals freeze: residuals must be a non-empty list")
        return
    seen: set[str] = set()
    classes_seen: set[str] = set()
    never_ids: set[str] = set()
    island_crates: set[str] = set()
    rfc_dir = root / "docs/rfc"
    for i, row in enumerate(rows):
        if not isinstance(row, dict):
            r.fail(f"residuals freeze: row {i} is not an object")
            continue
        rid = row.get("id")
        title = row.get("title")
        klass = row.get("class")
        owner = row.get("owner")
        close = row.get("close")
        if not all(isinstance(x, str) and x.strip() for x in (rid, title, klass, owner, close)):
            r.fail(f"residuals freeze: row {i} missing id/title/class/owner/close")
            continue
        if rid in seen:
            r.fail(f"residuals freeze: duplicate id {rid}")
        seen.add(rid)
        if klass not in RESIDUAL_CLASSES:
            r.fail(f"residuals freeze: {rid} class {klass!r} not in {sorted(RESIDUAL_CLASSES)}")
            continue
        classes_seen.add(klass)
        if klass == "never":
            never_ids.add(rid)
        matches = list(rfc_dir.glob(f"{owner}-*.md")) + list(rfc_dir.glob(f"{owner}*.md"))
        if not matches:
            r.fail(f"residuals freeze: {rid} owner RFC {owner} has no docs/rfc/{owner}*.md")
        for key in ("script", "safety"):
            rel = row.get(key)
            if rel is None:
                continue
            if not isinstance(rel, str) or not (root / rel).is_file():
                r.fail(f"residuals freeze: {rid} {key} {rel!r} is not a file")
        crate = row.get("crate")
        if crate in ISLAND_CRATES:
            island_crates.add(crate)
            blob = f"{title} {close}".lower()
            if "safety.md" not in blob or not any(w in blob for w in ("forall", "∀", "for all")):
                r.fail(
                    f"residuals freeze: {rid} must name SAFETY.md and that it is not a forall proof"
                )
            if not row.get("script") or not row.get("safety"):
                r.fail(f"residuals freeze: {rid} island row needs script+safety")
        if rid == "R-tcg-guest":
            script = row.get("script") or ""
            blob = f"{title} {close} {row.get('mechanism') or ''}"
            if Path(script).name != "tcg_guest_status.sh":
                r.fail(f"residuals freeze: {rid} script must be tcg_guest_status.sh (got {script!r})")
            if "C2.2" not in blob or "residual_no_guest" not in blob:
                r.fail(f"residuals freeze: {rid} must name C2.2=residual_no_guest")
            tcg = (root / script) if script else None
            if tcg and tcg.is_file():
                src = tcg.read_text(encoding="utf-8", errors="replace")
                if "TCG_REQUIRED" not in src or "FAIL_no_guest" not in src:
                    r.fail(f"residuals freeze: {script} must fail-close when TCG_REQUIRED=1")

    for need in ("never", "continuous"):
        if need not in classes_seen:
            r.fail(f"residuals freeze: catalog missing any {need!r} row")
    for crate in ISLAND_CRATES:
        if crate not in island_crates:
            r.fail(f"residuals freeze: missing unsafe island row for {crate}")

    floor = data.get("never_floor")
    if not isinstance(floor, list) or not all(isinstance(x, str) for x in floor):
        r.fail("residuals freeze: never_floor must be a list of ids")
    else:
        floor_set = set(floor)
        if floor_set != never_ids:
            missing = sorted(floor_set - never_ids)
            extra = sorted(never_ids - floor_set)
            if missing:
                r.fail(
                    f"residuals freeze: never id(s) disappeared {missing} "
                    "(edit never_floor and RFC-0061 together — RFC-0061 P2.1)"
                )
            if extra:
                r.fail(f"residuals freeze: never id(s) not in never_floor {extra}")
        rfc_text = rfc.read_text(encoding="utf-8") if rfc.is_file() else ""
        if not rfc.is_file():
            r.fail(f"residuals freeze: missing {RFC_0061}")
        else:
            for nid in sorted(floor_set):
                if nid not in rfc_text:
                    r.fail(
                        f"residuals freeze: never id {nid} missing from {RFC_0061} "
                        "(P2.1: catalog and RFC must both change)"
                    )

    glue = data.get("glue")
    if not isinstance(glue, dict):
        r.fail("residuals freeze: glue object required (RFC-0061 P1.3)")
    else:
        if glue.get("db_rs_extracted") is not False:
            r.fail("residuals freeze: glue.db_rs_extracted must be false (do not extract db.rs)")
        check_kernel_enrollment(root, glue, r)
        cat = catalog
        if cat is None:
            cat_path = root / "scripts/formal/catalog.json"
            cat = json.loads(cat_path.read_text(encoding="utf-8")) if cat_path.is_file() else {"pairs": []}
        check_kernel_fn_surface(root, cat, glue, r)
        n_files, k_loc, h_loc = glue_loc(root, cat)
        for key, live in (
            ("kernel_files", n_files),
            ("kernel_loc", k_loc),
            ("handler_loc", h_loc),
        ):
            got = glue.get(key)
            if got != live:
                r.fail(
                    f"residuals freeze: glue.{key}={got!r} != live {live} "
                    "(recompute; do not extract db.rs)"
                )
        for pair in cat.get("pairs", []):
            k = pair.get("kernel") or ""
            if Path(k).name == "db.rs":
                r.fail(f"residuals freeze: kernel {k} extracts db.rs — refused")

    if len(r.failed) == before:
        r.good(f"residuals freeze: {len(seen)} rows")


TWIN_KINDS = {"close", "atom", "model"}

PROOF_DEPTHS = {"atom", "close", "extract", "model"}
ATOM_REASON_DATE = re.compile(r"^(\d{4}-\d{2}-\d{2})$")
# RFC-0170 P0.1: close + Aeneas SOURCE stamp ⇒ proof_depth=extract.
AENEAS_EXTRACTS = (
    ("crates/pedradb-raft/src/vote_kernel.rs", "formal/aeneas/out/SOURCE"),
    ("crates/pedradb-fold/src/isolated_kernel.rs", "formal/aeneas/out/SOURCE.isolated"),
    ("crates/pedradb-core/src/bloom.rs", "formal/aeneas/out/SOURCE.bloom"),
    ("crates/pedradb-raft/src/ae_kernel.rs", "formal/aeneas/out/SOURCE.ae"),
    ("crates/pedradb-raft/src/commit_kernel.rs", "formal/aeneas/out/SOURCE.commit"),
    ("crates/pedradb-core/src/group_commit_kernel.rs", "formal/aeneas/out/SOURCE.group_commit"),
    ("crates/pedradb-core/src/wal/reopen_kernel.rs", "formal/aeneas/out/SOURCE.reopen"),
    ("crates/pedradb-core/src/wal/recover_kernel.rs", "formal/aeneas/out/SOURCE.wal_recover"),
    ("crates/pedradb-raft/src/apply_kernel.rs", "formal/aeneas/out/SOURCE.apply"),
    ("crates/pedradb-core/src/prefix.rs", "formal/aeneas/out/SOURCE.prefix"),
    (
        "crates/pedradb-core/src/write_admission_kernel.rs",
        "formal/aeneas/out/SOURCE.write_admission",
    ),
    (
        "crates/pedradb-core/src/flush_kernel.rs",
        "formal/aeneas/out/SOURCE.flush",
    ),
    ("crates/pedradb-core/src/lookup_kernel.rs", "formal/aeneas/out/SOURCE.lookup"),
    ("crates/pedradb-store/src/rpc_mode_kernel.rs", "formal/aeneas/out/SOURCE.rpc_mode"),
    (
        "crates/pedradb-store/src/compact_kernel.rs",
        "formal/aeneas/out/SOURCE.store_compact",
    ),
    (
        "crates/pedradb-store/src/snapshot_kernel.rs",
        "formal/aeneas/out/SOURCE.snapshot",
    ),
    ("crates/pedradb-store/src/si_kernel.rs", "formal/aeneas/out/SOURCE.si"),
    (
        "crates/pedradb-store/src/index_val_kernel.rs",
        "formal/aeneas/out/SOURCE.index_val",
    ),
    (
        "crates/pedradb-core/src/changelog_kernel.rs",
        "formal/aeneas/out/SOURCE.changelog",
    ),
    ("crates/pedradb-stream/src/cursor_kernel.rs", "formal/aeneas/out/SOURCE.cursor"),
    ("crates/pedradb-http/src/cl_kernel.rs", "formal/aeneas/out/SOURCE.cl"),
    (
        "crates/montanha-fdb-recipes/src/children_kernel.rs",
        "formal/aeneas/out/SOURCE.children",
    ),
    ("crates/pedradb-journal/src/pin_kernel.rs", "formal/aeneas/out/SOURCE.pin"),
    (
        "crates/montanha-fdb-recipes/src/pack_kernel.rs",
        "formal/aeneas/out/SOURCE.pack",
    ),
    ("crates/pedradb-replicate/src/ship_kernel.rs", "formal/aeneas/out/SOURCE.ship"),
    ("crates/pedradb-fold/src/fold_kernel.rs", "formal/aeneas/out/SOURCE.fold"),
    (
        "crates/pedradb-core/src/manifest_kernel.rs",
        "formal/aeneas/out/SOURCE.manifest",
    ),
    ("crates/pedradb-core/src/compact_kernel.rs", "formal/aeneas/out/SOURCE.compact"),
    ("crates/pedradb-core/src/vlog_gc_kernel.rs", "formal/aeneas/out/SOURCE.vlog_gc"),
    ("crates/pedradb-store/src/tx_glue_kernel.rs", "formal/aeneas/out/SOURCE.tx_glue"),
    ("crates/pedradb-store/src/l28.rs", "formal/aeneas/out/SOURCE.l28"),
    ("crates/pedradb-world/src/tcg.rs", "formal/aeneas/out/SOURCE.tcg"),
    ("crates/pedradb-io-uring/src/cqe_kernel.rs", "formal/aeneas/out/SOURCE.cqe"),
    ("crates/rocksdb-compat/src/iter_kernel.rs", "formal/aeneas/out/SOURCE.iter"),
    (
        "crates/pedradb-spec/src/properties_kernel.rs",
        "formal/aeneas/out/SOURCE.properties",
    ),
    (
        "crates/pedradb-core/src/scale_kernel.rs",
        "formal/aeneas/out/SOURCE.scale",
    ),
    (
        "crates/pedradb-core/src/disk_pressure_kernel.rs",
        "formal/aeneas/out/SOURCE.disk_pressure",
    ),
    ("crates/pedradb-core/src/wal/crc.rs", "formal/aeneas/out/SOURCE.crc"),
    (
        "crates/pedradb-core/src/env_crash_kernel.rs",
        "formal/aeneas/out/SOURCE.env_crash",
    ),
    (
        "crates/pedradb-core/src/wal/wal_state_kernel.rs",
        "formal/aeneas/out/SOURCE.wal_state",
    ),
    (
        "crates/pedradb-core/src/d1_modelo_kernel.rs",
        "formal/aeneas/out/SOURCE.d1_modelo",
    ),
    (
        "crates/pedradb-core/src/write_ack_kernel.rs",
        "formal/aeneas/out/SOURCE.write_ack",
    ),
    (
        "crates/pedradb-dcs/src/apply_kernel.rs",
        "formal/aeneas/out/SOURCE.dcs_apply",
    ),
    (
        "crates/pedradb-store/src/apply_kernel.rs",
        "formal/aeneas/out/SOURCE.store_apply",
    ),
    (
        "crates/pedradb-store/src/commit_kernel.rs",
        "formal/aeneas/out/SOURCE.store_commit",
    ),
    (
        "crates/pedradb-store/src/ae_ack_kernel.rs",
        "formal/aeneas/out/SOURCE.store_ae_ack",
    ),
    (
        "crates/pedradb-store/src/vote_kernel.rs",
        "formal/aeneas/out/SOURCE.store_vote",
    ),
    ("crates/pedradb-core/src/key.rs", "formal/aeneas/out/SOURCE.key"),
    (
        "crates/pedradb-dcs/src/lease_kernel.rs",
        "formal/aeneas/out/SOURCE.lease",
    ),
    (
        "crates/pedradb-store/src/txn_kernel.rs",
        "formal/aeneas/out/SOURCE.txn",
    ),
    (
        "crates/pedradb-store/src/t1_modelo_kernel.rs",
        "formal/aeneas/out/SOURCE.t1_modelo",
    ),
    (
        "crates/pedradb-raft/src/membership_kernel.rs",
        "formal/aeneas/out/SOURCE.membership",
    ),
    (
        "crates/pedradb-store/src/membership_kernel.rs",
        "formal/aeneas/out/SOURCE.store_membership",
    ),
    (
        "crates/pedradb-raft/src/c1_modelo_kernel.rs",
        "formal/aeneas/out/SOURCE.c1_modelo",
    ),
    (
        "crates/pedradb-capi/src/handles.rs",
        "formal/aeneas/out/SOURCE.capi_handles",
    ),
    ("crates/pedradb-core/src/batch.rs", "formal/aeneas/out/SOURCE.batch"),
    ("crates/pedradb-core/src/merge.rs", "formal/aeneas/out/SOURCE.merge"),
    (
        "crates/pedradb-http/src/fail_closed.rs",
        "formal/aeneas/out/SOURCE.fail_closed",
    ),
    (
        "crates/pedradb-core/src/probe_order_kernel.rs",
        "formal/aeneas/out/SOURCE.probe_order",
    ),
    (
        "crates/rocksdb-compat/src/locktab.rs",
        "formal/aeneas/out/SOURCE.locktab",
    ),
    (
        "crates/pedradb-core/src/sst/scan_kernel.rs",
        "formal/aeneas/out/SOURCE.scan",
    ),
    (
        "crates/pedradb-core/src/cf_kernel.rs",
        "formal/aeneas/out/SOURCE.cf",
    ),
    (
        "crates/montanha-fdb-recipes/src/fields_kernel.rs",
        "formal/aeneas/out/SOURCE.fields",
    ),
    (
        "crates/pedradb-core/src/lsm_r1_kernel.rs",
        "formal/aeneas/out/SOURCE.lsm_r1",
    ),
    (
        "crates/pedradb-core/src/leveling.rs",
        "formal/aeneas/out/SOURCE.leveling",
    ),
    (
        "crates/pedradb-posix/src/lib.rs",
        "formal/aeneas/out/SOURCE.posix",
    ),
    (
        "crates/pedradb-http/src/form_kernel.rs",
        "formal/aeneas/out/SOURCE.form",
    ),
    (
        "crates/pedradb-http/src/auth_kernel.rs",
        "formal/aeneas/out/SOURCE.auth",
    ),
    (
        "crates/pedradb-http/src/path_kernel.rs",
        "formal/aeneas/out/SOURCE.path",
    ),
    (
        "crates/pedradb-world/src/world_kernel.rs",
        "formal/aeneas/out/SOURCE.world",
    ),
)


def proof_depth_of(pair: dict) -> str:
    kind = pair.get("twin_kind")
    if kind == "atom":
        return "atom"
    if kind == "model":
        return "model"
    if kind == "close":
        kernel = pair.get("kernel") or ""
        if any(kernel == k for k, _ in AENEAS_EXTRACTS):
            return "extract"
        return "close"
    return "unknown"


def check_proof_depth(root: Path, catalog: dict, r: Report) -> None:
    """RFC-0170 P0.1 + P1.6: atom needs dated atom_reason; >30d needs child_rfc."""
    print("== proof_depth (RFC-0170: atom|close|extract; model stand-in until P2.2) ==")
    from datetime import date, datetime

    today = date(2026, 9, 6)
    n_atom = n_close = n_extract = n_model = 0
    for pair in catalog["pairs"]:
        pid = pair["id"]
        depth = proof_depth_of(pair)
        if depth == "atom":
            n_atom += 1
            reason = pair.get("atom_reason")
            if not isinstance(reason, dict):
                r.fail(
                    f"{pid}: atom twin needs atom_reason "
                    "{{date: YYYY-MM-DD, why: ...}} (RFC-0170 P0.1)"
                )
                continue
            ds = str(reason.get("date") or "")
            if not ATOM_REASON_DATE.match(ds):
                r.fail(f"{pid}: atom_reason.date must be YYYY-MM-DD")
                continue
            try:
                d = datetime.strptime(ds, "%Y-%m-%d").date()
            except ValueError:
                r.fail(f"{pid}: atom_reason.date not a real day")
                continue
            child = reason.get("child_rfc")
            age = (today - d).days
            if age > 30 and not child:
                r.fail(
                    f"{pid}: atom_reason older than 30 days without child_rfc "
                    "(RFC-0170 P1.6)"
                )
                continue
            if not str(reason.get("why") or "").strip():
                r.fail(f"{pid}: atom_reason.why empty")
                continue
            r.good(f"{pid}: proof_depth=atom ({ds})")
        elif depth == "extract":
            n_extract += 1
            r.good(f"{pid}: proof_depth=extract")
        elif depth == "close":
            n_close += 1
            r.good(f"{pid}: proof_depth=close")
        elif depth == "model":
            n_model += 1
            r.good(f"{pid}: proof_depth=model (stand-in)")
        else:
            r.fail(f"{pid}: unknown proof_depth from twin_kind={pair.get('twin_kind')!r}")
    r.good(
        f"proof_depth counts: extract={n_extract} close={n_close} "
        f"atom={n_atom} model={n_model}"
    )
    res_path = root / "scripts/formal/residuals.json"
    if res_path.is_file():
        res = json.loads(res_path.read_text(encoding="utf-8"))
        pd = (res.get("glue") or {}).get("proof_depth")
        live = {
            "extract": n_extract,
            "close": n_close,
            "atom": n_atom,
            "model": n_model,
        }
        if not isinstance(pd, dict):
            r.gap("RFC-0170 P2.5: glue.proof_depth not frozen yet")
        else:
            freeze_failed = False
            for k, v in live.items():
                if pd.get(k) != v:
                    freeze_failed = True
                    r.fail(
                        f"residuals freeze: glue.proof_depth.{k}={pd.get(k)!r} != live {v}"
                    )
            if not freeze_failed:
                r.good(f"proof_depth freeze matches live {live}")
        n_sa = sum(1 for p in catalog.get("pairs") or [] if p.get("single_artifact"))
        n_df = sum(1 for p in catalog.get("pairs") or [] if p.get("data_fate"))
        print(f"== single_artifact fraction (RFC-0174 P0.2): sa={n_sa} data_fate={n_df} ==")
        got_sa = (res.get("glue") or {}).get("single_artifact")
        got_df = (res.get("glue") or {}).get("data_fate")
        if got_sa != n_sa:
            r.fail(
                f"residuals freeze: glue.single_artifact={got_sa!r} != live {n_sa} "
                "(RFC-0174 P0.2)"
            )
        else:
            r.good(f"single_artifact freeze matches live {n_sa}")
        if got_df != n_df:
            r.fail(
                f"residuals freeze: glue.data_fate={got_df!r} != live {n_df} "
                "(RFC-0174 P0.2)"
            )
        else:
            r.good(f"data_fate freeze matches live {n_df}")


def check_twins(root: Path, catalog: dict, r: Report, strict: bool) -> None:
    print("== twins (kind + kernel tokens ⊆ Verus twin) ==")
    for pair in catalog["pairs"]:
        pid = pair["id"]
        kind = pair.get("twin_kind")
        if kind not in TWIN_KINDS:
            r.fail(f"{pid}: twin_kind must be close|atom|model (got {kind!r})")
            continue
        if pair.get("single_artifact"):
            kpath = pair.get("kernel") or ""
            tpath = pair.get("twin") or ""
            if kpath != tpath:
                r.fail(
                    f"{pid}: single_artifact requires twin == kernel "
                    f"(RFC-0171 P0.3; kernel={kpath!r} twin={tpath!r})"
                )
            else:
                r.good(f"{pid}: single_artifact twin == kernel")
        absent = pair.get("status") == "absent"
        ksrc = load_text(root, pair["kernel"])
        if ksrc is None:
            r.fail(f"{pid}: missing kernel {pair['kernel']}")
            continue
        tsrc = load_text(root, pair["twin"])
        if tsrc is None:
            msg = f"{pid}: missing twin {pair['twin']}"
            if absent and not strict:
                r.gap(msg)
            else:
                r.fail(msg)
            continue
        if absent:
            r.good(f"{pid}: twin present (catalog still says absent — update catalog)")
        kexec = exec_fns(ksrc)
        texec = exec_fns(tsrc)
        # Verus `ensures` clauses contain `{`; compare against the whole twin
        # (comments already stripped) so we do not have to parse proof syntax.
        twin_tok = set(tokens(strip_comments(tsrc)))

        if kind == "close":
            name = pair.get("token_src") or pair.get("entry")
            if not name:
                r.fail(f"{pid}: close twin needs entry or token_src")
                continue
            if name not in texec:
                r.fail(
                    f"{pid}: close twin missing exec fn {name}() "
                    f"(got {sorted(texec)}; mark twin_kind=atom if this is a cartoon)"
                )
                continue
            names = [name]
        elif kind == "atom":
            atom = pair.get("atom")
            if not atom:
                r.fail(f"{pid}: atom twin needs atom= (the fn the Verus file proves)")
                continue
            if atom not in texec:
                r.fail(f"{pid}: atom twin missing exec fn {atom}() (got {sorted(texec)})")
                continue
            if atom in kexec:
                names = [atom]
            elif pair.get("token_src") in kexec:
                names = [pair["token_src"]]
            else:
                r.good(f"{pid}: atom {atom} (not in kernel; twin-only)")
                continue
        else:  # model
            name = pair.get("token_src") or pair.get("entry")
            if not name:
                r.fail(f"{pid}: model twin needs entry or token_src")
                continue
            if name not in texec:
                r.fail(f"{pid}: model twin missing exec fn {name}()")
                continue
            names = [name]

        for name in names:
            if name not in kexec:
                r.fail(f"{pid}: {name} not in kernel")
                continue
            missing = [t for t in tokens(kexec[name]) if t not in twin_tok]
            if missing:
                r.fail(f"{pid}: {name} twin missing tokens {missing}")
            elif kind == "model":
                r.good(f"{pid}: {name} tokens covered (model: stand-in domain, not production types)")
            elif kind == "atom":
                r.good(f"{pid}: atom {name} tokens covered (not the production entry)")
            else:
                r.good(f"{pid}: close {name} tokens covered")


def check_scripts(root: Path, catalog: dict, r: Report, strict: bool) -> None:
    print("== verus scripts (SRC must match catalog) ==")
    catalog_scripts = {}
    for pair in catalog["pairs"]:
        script = pair.get("verus")
        if script:
            catalog_scripts[script] = pair
    for sh in sorted((root / "scripts").glob("verus_*.sh")):
        rel = f"scripts/{sh.name}"
        if rel not in catalog_scripts:
            r.fail(f"unlisted {rel}")
            continue
        pair = catalog_scripts[rel]
        text = sh.read_text(encoding="utf-8")
        m = re.search(r'SRC="\$ROOT/([^"]+)"', text)
        if not m:
            r.fail(f"{rel}: no SRC= line")
            continue
        src = m.group(1)
        if src != pair["twin"]:
            r.fail(f"{rel}: SRC {src} != catalog twin {pair['twin']}")
            continue
        exists = (root / src).is_file()
        if not exists:
            msg = f"{rel}: SRC missing on disk ({src})"
            if pair.get("status") == "absent" and not strict:
                r.gap(msg)
            else:
                r.fail(msg)
        else:
            r.good(f"{rel} → {src}")


def check_models(root: Path, catalog: dict, r: Report) -> None:
    print("== models (Stateright on production kernels) ==")
    env = os.environ.copy()
    env.setdefault("CARGO_TERM_COLOR", "never")
    for model in catalog.get("models", []):
        cmd = [
            "cargo",
            "test",
            "-q",
            "-p",
            model["crate"],
            "--test",
            model["test"],
            "--",
            "--test-threads=1",
        ]
        print("      " + " ".join(cmd))
        p = subprocess.run(cmd, cwd=root, env=env)
        if p.returncode != 0:
            r.fail(f"model {model['crate']}::{model['test']} exit {p.returncode}")
        else:
            r.good(f"model {model['crate']}::{model['test']}")


def check_extract(
    root: Path, r: Report, *, want_charon: bool, charon_required: bool
) -> None:
    print("== extract (include crate; Charon/Aeneas optional) ==")
    hook = root / ".githooks/pre-commit"
    if not hook.is_file() or not os.access(hook, os.X_OK):
        r.fail("aeneas stamp guard .githooks/pre-commit missing or not executable")
    else:
        hs = hook.read_text(encoding="utf-8")
        if "formal/aeneas/out/SOURCE" in hs and "sha256=" in hs:
            r.good("aeneas stamp guard pre-commit hook present (RFC-0157 follow-up)")
        else:
            r.fail("aeneas stamp guard pre-commit hook no longer checks SOURCE stamps")
    manifest = root / "formal/aeneas/vote-kernel/Cargo.toml"
    if not manifest.is_file():
        r.fail("formal/aeneas/vote-kernel/Cargo.toml missing")
        return
    env = os.environ.copy()
    env.setdefault("CARGO_TERM_COLOR", "never")
    p = subprocess.run(
        [
            "cargo",
            "test",
            "-q",
            "--manifest-path",
            str(manifest),
            "--",
            "--test-threads=1",
        ],
        cwd=root,
        env=env,
    )
    if p.returncode != 0:
        r.fail(f"aeneas include crate cargo test exit {p.returncode}")
        return
    r.good("aeneas extract crate (production vote_kernel.rs as [lib] path)")
    lean = root / "formal/aeneas/out/lean/VoteKernel.lean"
    if lean.is_file() and "def vote_decision" in lean.read_text(encoding="utf-8"):
        r.good("aeneas extract artifact has def vote_decision")
        vk = lean.read_text(encoding="utf-8")
        axiom = "axiom core.option.Option.Insts.CoreCmpPartialEqOption.eq"
        modeled = "def core.option.Option.Insts.CoreCmpPartialEqOption.eq"
        if axiom in vk:
            r.fail(
                "RFC-0053 P40: VoteKernel still axioms Option::eq "
                "(replace with match def; see formal/aeneas/lean/Vote.lean)"
            )
        elif modeled in vk:
            r.good("RFC-0053 P40: Option::eq is a match def, not an axiom")
        else:
            r.good("RFC-0053 P40: extract has no Option::eq (derive dropped)")
        vote_thy = root / "formal/aeneas/lean/Vote.lean"
        if vote_thy.is_file() and "theorem vote_decision_iff" in vote_thy.read_text(
            encoding="utf-8"
        ):
            r.good("RFC-0053 P40: Vote.lean has theorem vote_decision_iff")
        else:
            r.fail("RFC-0053 P40: Vote.lean missing theorem vote_decision_iff")
    else:
        r.gap("aeneas extract artifact formal/aeneas/out/lean/VoteKernel.lean missing (run ./scripts/aeneas_vote.sh)")
    stamp = root / "formal/aeneas/out/SOURCE"
    src = root / "crates/pedradb-raft/src/vote_kernel.rs"
    if stamp.is_file() and src.is_file():
        want = None
        for line in stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE sha256 matches vote_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE sha256 drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_vote.sh)"
            )
    iso_lean = root / "formal/aeneas/out/lean/IsolatedKernel.lean"
    if iso_lean.is_file() and "def isolated_id_matches" in iso_lean.read_text(
        encoding="utf-8"
    ):
        r.good("aeneas extract artifact has def isolated_id_matches")
    else:
        r.gap(
            "aeneas extract artifact IsolatedKernel.lean missing (run ./scripts/aeneas_isolated.sh)"
        )
    iso_stamp = root / "formal/aeneas/out/SOURCE.isolated"
    iso_src = root / "crates/pedradb-fold/src/isolated_kernel.rs"
    if iso_stamp.is_file() and iso_src.is_file():
        want = None
        for line in iso_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(iso_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.isolated sha256 matches isolated_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.isolated drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_isolated.sh)"
            )
    bloom_lean = root / "formal/aeneas/out/lean/BloomKernel.lean"
    if bloom_lean.is_file() and "def BloomFilter.may_contain" in bloom_lean.read_text(
        encoding="utf-8"
    ):
        r.good("aeneas extract artifact has def BloomFilter.may_contain")
    else:
        r.gap(
            "aeneas extract artifact BloomKernel.lean missing (run ./scripts/aeneas_bloom.sh)"
        )
    bloom_stamp = root / "formal/aeneas/out/SOURCE.bloom"
    bloom_src = root / "crates/pedradb-core/src/bloom.rs"
    if bloom_stamp.is_file() and bloom_src.is_file():
        want = None
        for line in bloom_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(bloom_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.bloom sha256 matches bloom.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.bloom drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_bloom.sh)"
            )
    ae_lean = root / "formal/aeneas/out/lean/AeKernel.lean"
    if ae_lean.is_file() and "def ae_entry_action" in ae_lean.read_text(encoding="utf-8"):
        r.good("aeneas extract artifact has def ae_entry_action")
    else:
        r.gap(
            "aeneas extract artifact AeKernel.lean missing (run ./scripts/aeneas_ae.sh)"
        )
    ae_stamp = root / "formal/aeneas/out/SOURCE.ae"
    ae_src = root / "crates/pedradb-raft/src/ae_kernel.rs"
    if ae_stamp.is_file() and ae_src.is_file():
        want = None
        for line in ae_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(ae_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.ae sha256 matches ae_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.ae drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_ae.sh)"
            )
    ae_thy = root / "formal/aeneas/lean/Ae.lean"
    if ae_thy.is_file():
        at = ae_thy.read_text(encoding="utf-8")
        if re.search(r"\bsorry\b", at):
            r.fail("RFC-0053 P2.1: Ae.lean contains sorry")
        elif "theorem as_is_rewrites_committed" in at and "theorem ae_keep_if_same_term" in at:
            r.good("RFC-0053 P2.1: Ae.lean theorems (no sorry)")
        else:
            r.fail("RFC-0053 P2.1: Ae.lean missing named theorems")
    else:
        r.fail("RFC-0053 P2.1: formal/aeneas/lean/Ae.lean missing")
    cm_lean = root / "formal/aeneas/out/lean/CommitKernel.lean"
    if cm_lean.is_file() and "def recover_commit" in cm_lean.read_text(encoding="utf-8"):
        r.good("aeneas extract artifact has def recover_commit")
    else:
        r.gap(
            "aeneas extract artifact CommitKernel.lean missing (run ./scripts/aeneas_commit.sh)"
        )
    cm_stamp = root / "formal/aeneas/out/SOURCE.commit"
    cm_src = root / "crates/pedradb-raft/src/commit_kernel.rs"
    if cm_stamp.is_file() and cm_src.is_file():
        want = None
        for line in cm_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(cm_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.commit sha256 matches commit_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.commit drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run ./scripts/aeneas_commit.sh)"
            )
    cm_thy = root / "formal/aeneas/lean/Commit.lean"
    if cm_thy.is_file():
        ct = cm_thy.read_text(encoding="utf-8")
        if re.search(r"\bsorry\b", ct):
            r.fail("RFC-0053 P2.1: Commit.lean contains sorry")
        elif "theorem may_commit_at_iff" in ct and "theorem recover_commit_caps_examples" in ct:
            r.good("RFC-0053 P2.1: Commit.lean theorems (no sorry)")
        else:
            r.fail("RFC-0053 P2.1: Commit.lean missing named theorems")
    else:
        r.fail("RFC-0053 P2.1: formal/aeneas/lean/Commit.lean missing")
    # RFC-0056 P1.2: WAL/apply/reopen extracts + Lean theorems
    for stamp, artifact, marker, regen, src, thy, theorems in [
        (
            "reopen",
            "formal/aeneas/out/lean/ReopenKernel.lean",
            "def reopen_outcome",
            "./scripts/aeneas_reopen.sh",
            "crates/pedradb-core/src/wal/reopen_kernel.rs",
            "formal/aeneas/lean/Reopen.lean",
            ("theorem fail_closed_refuses_every_damage", "theorem as_is_swallows_damage"),
        ),
        (
            "apply",
            "formal/aeneas/out/lean/ApplyKernel.lean",
            "def apply_advance",
            "./scripts/aeneas_apply.sh",
            "crates/pedradb-raft/src/apply_kernel.rs",
            "formal/aeneas/lean/Apply.lean",
            ("theorem apply_advance_closed_form", "theorem as_is_applies_hole"),
        ),
        (
            "wal_recover",
            "formal/aeneas/out/lean/WalRecoverKernel.lean",
            "def recover_kernel.recover_collect_act",
            "./scripts/aeneas_wal_recover.sh",
            "crates/pedradb-core/src/wal/recover_kernel.rs",
            "formal/aeneas/lean/WalRecover.lean",
            ("theorem crc_fresh_alignment_fail_stops", "theorem as_is_torn_is_silent_eof"),
        ),
        (
            "group_commit",
            "formal/aeneas/out/lean/GroupCommitKernel.lean",
            "def fence_publish_seq",
            "./scripts/aeneas_group_commit.sh",
            "crates/pedradb-core/src/group_commit_kernel.rs",
            "formal/aeneas/lean/GroupCommit.lean",
            ("theorem occ_conflict_closed_form", "theorem as_is_serialized_aborts_same_group_writer"),
        ),
    ]:
        art = root / artifact
        if art.is_file() and marker in art.read_text(encoding="utf-8"):
            r.good(f"aeneas extract artifact has {marker}")
        else:
            r.gap(
                f"aeneas extract artifact {artifact.rsplit('/', 1)[-1]} missing (run {regen})"
            )
        st = root / f"formal/aeneas/out/SOURCE.{stamp}"
        kr = root / src
        if st.is_file() and kr.is_file():
            want = None
            for line in st.read_text(encoding="utf-8").splitlines():
                if line.startswith("sha256="):
                    want = line.split("=", 1)[1].strip()
            have = hashlib.sha256(kr.read_bytes()).hexdigest()
            if want and have == want:
                r.good(f"aeneas SOURCE.{stamp} sha256 matches {src.rsplit('/', 1)[-1]}")
            elif want:
                r.fail(
                    f"aeneas SOURCE.{stamp} drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run {regen})"
                )
        tf = root / thy
        if tf.is_file():
            tt = tf.read_text(encoding="utf-8")
            if re.search(r"\bsorry\b", tt):
                r.fail(f"RFC-0056 P1.2: {thy.rsplit('/', 1)[-1]} contains sorry")
            elif all(t in tt for t in theorems):
                r.good(f"RFC-0056 P1.2: {thy.rsplit('/', 1)[-1]} theorems (no sorry)")
            else:
                r.fail(f"RFC-0056 P1.2: {thy.rsplit('/', 1)[-1]} missing named theorems")
        else:
            r.fail(f"RFC-0056 P1.2: {thy} missing")
    p12_script = root / "scripts/lean_wal_apply_reopen.sh"
    p = subprocess.run(
        ["bash", str(p12_script)] + (["--required"] if charon_required else []),
        cwd=root,
    )
    if p.returncode != 0:
        r.fail(f"lean_wal_apply_reopen.sh exit {p.returncode}")
    elif charon_required:
        r.good("lean_wal_apply_reopen.sh (Reopen + Apply + WalRecover)")
    lean_script = root / "scripts/lean_vote.sh"
    p = subprocess.run(
        ["bash", str(lean_script)] + (["--required"] if charon_required else []),
        cwd=root,
    )
    if p.returncode != 0:
        r.fail(f"lean_vote.sh exit {p.returncode}")
    elif charon_required:
        r.good("lean_vote.sh (vote + Ae + Commit)")

    # RFC-0170 P0.3: prefix.rs stamp.
    pref_stamp = root / "formal/aeneas/out/SOURCE.prefix"
    pref_src = root / "crates/pedradb-core/src/prefix.rs"
    if pref_stamp.is_file() and pref_src.is_file():
        want = None
        for line in pref_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(pref_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.prefix sha256 matches prefix.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.prefix drifted (kernel {have[:12]}… vs stamp {want[:12]}…; "
                "re-run ./scripts/aeneas_prefix.sh)"
            )
        thy = root / "formal/aeneas/lean/Prefix.lean"
        if thy.is_file():
            tt = thy.read_text(encoding="utf-8")
            if re.search(r"\bsorry\b", tt):
                r.fail("RFC-0170 P0.3: Prefix.lean contains sorry")
            elif "theorem prefix_exclusive_end_matches_spec" in tt:
                r.good("RFC-0170 P0.3: Prefix.lean theorem prefix_exclusive_end_matches_spec")
            else:
                r.fail("RFC-0170 P0.3: Prefix.lean missing prefix_exclusive_end_matches_spec")
        else:
            r.fail("RFC-0170 P0.3: formal/aeneas/lean/Prefix.lean missing")
    else:
        r.gap("aeneas SOURCE.prefix missing (run ./scripts/aeneas_prefix.sh)")
    # RFC-0170 P2.1: write_admission_kernel.rs stamp.
    wa_stamp = root / "formal/aeneas/out/SOURCE.write_admission"
    wa_src = root / "crates/pedradb-core/src/write_admission_kernel.rs"
    if wa_stamp.is_file() and wa_src.is_file():
        want = None
        for line in wa_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(wa_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.write_admission sha256 matches write_admission_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.write_admission drifted (kernel {have[:12]}… vs stamp {want[:12]}…; "
                "re-run ./scripts/aeneas_write_admission.sh)"
            )
        thy = root / "formal/aeneas/lean/WriteAdmission.lean"
        if thy.is_file():
            tt = thy.read_text(encoding="utf-8")
            if re.search(r"\bsorry\b", tt):
                r.fail("RFC-0170 P2.1: WriteAdmission.lean contains sorry")
            elif "theorem write_admission_idle_matches_spec" in tt:
                r.good("RFC-0170 P2.1: WriteAdmission.lean theorem write_admission_idle_matches_spec")
            else:
                r.fail("RFC-0170 P2.1: WriteAdmission.lean missing write_admission_idle_matches_spec")
        else:
            r.fail("RFC-0170 P2.1: formal/aeneas/lean/WriteAdmission.lean missing")
    else:
        r.gap("aeneas SOURCE.write_admission missing (run ./scripts/aeneas_write_admission.sh)")
    # RFC-0174 P1.3: flush_kernel.rs stamp.
    fl_stamp = root / "formal/aeneas/out/SOURCE.flush"
    fl_src = root / "crates/pedradb-core/src/flush_kernel.rs"
    if fl_stamp.is_file() and fl_src.is_file():
        want = None
        for line in fl_stamp.read_text(encoding="utf-8").splitlines():
            if line.startswith("sha256="):
                want = line.split("=", 1)[1].strip()
        have = hashlib.sha256(fl_src.read_bytes()).hexdigest()
        if want and have == want:
            r.good("aeneas SOURCE.flush sha256 matches flush_kernel.rs")
        elif want:
            r.fail(
                f"aeneas SOURCE.flush drifted (kernel {have[:12]}… vs stamp {want[:12]}…; "
                "re-run ./scripts/aeneas_flush.sh)"
            )
        thy = root / "formal/aeneas/lean/Flush.lean"
        if thy.is_file():
            tt = thy.read_text(encoding="utf-8")
            if re.search(r"\bsorry\b", tt):
                r.fail("RFC-0174 P1.3: Flush.lean contains sorry")
            elif "theorem flush_plan_empty_rotates_only" in tt:
                r.good("RFC-0174 P1.3: Flush.lean theorem flush_plan_empty_rotates_only")
            else:
                r.fail("RFC-0174 P1.3: Flush.lean missing flush_plan_empty_rotates_only")
        else:
            r.fail("RFC-0174 P1.3: formal/aeneas/lean/Flush.lean missing")
    else:
        r.gap("aeneas SOURCE.flush missing (run ./scripts/aeneas_flush.sh)")
    # Newly enrolled extracts: SOURCE sha256 + Lean theorems, no sorry.
    for stamp, artifact, marker, regen, src, thy, theorems in [
        (
            "lookup",
            "formal/aeneas/out/lean/LookupKernel.lean",
            "def snap_is_empty",
            "./scripts/aeneas_lookup.sh",
            "crates/pedradb-core/src/lookup_kernel.rs",
            "formal/aeneas/lean/Lookup.lean",
            ("theorem snap_is_empty_zero",),
        ),
        (
            "rpc_mode",
            "formal/aeneas/out/lean/RpcModeKernel.lean",
            "def allow_direct_rpc",
            "./scripts/aeneas_rpc_mode.sh",
            "crates/pedradb-store/src/rpc_mode_kernel.rs",
            "formal/aeneas/lean/RpcMode.lean",
            ("theorem allow_direct_rpc_pin_refuses",),
        ),
        (
            "store_compact",
            "formal/aeneas/out/lean/StoreCompactKernel.lean",
            "def may_compact_through",
            "./scripts/aeneas_store_compact.sh",
            "crates/pedradb-store/src/compact_kernel.rs",
            "formal/aeneas/lean/StoreCompact.lean",
            ("theorem may_compact_through_zero_false",),
        ),
        (
            "snapshot",
            "formal/aeneas/out/lean/SnapshotKernel.lean",
            "def snapshot_touches_user_key",
            "./scripts/aeneas_snapshot.sh",
            "crates/pedradb-store/src/snapshot_kernel.rs",
            "formal/aeneas/lean/Snapshot.lean",
            ("theorem snapshot_touches_user_key_unreserved",),
        ),
        (
            "si",
            "formal/aeneas/out/lean/SiKernel.lean",
            "def si_reader_beats",
            "./scripts/aeneas_si.sh",
            "crates/pedradb-store/src/si_kernel.rs",
            "formal/aeneas/lean/Si.lean",
            ("theorem si_reader_beats_c_live",),
        ),
        (
            "index_val",
            "formal/aeneas/out/lean/IndexValKernel.lean",
            "def value_len_tag",
            "./scripts/aeneas_index_val.sh",
            "crates/pedradb-store/src/index_val_kernel.rs",
            "formal/aeneas/lean/IndexVal.lean",
            ("theorem value_len_tag_identity",),
        ),
        (
            "changelog",
            "formal/aeneas/out/lean/ChangelogKernel.lean",
            "def changelog_should_store",
            "./scripts/aeneas_changelog.sh",
            "crates/pedradb-core/src/changelog_kernel.rs",
            "formal/aeneas/lean/Changelog.lean",
            ("theorem changelog_should_store_due",),
        ),
        (
            "cursor",
            "formal/aeneas/out/lean/CursorKernel.lean",
            "def next_seq",
            "./scripts/aeneas_cursor.sh",
            "crates/pedradb-stream/src/cursor_kernel.rs",
            "formal/aeneas/lean/Cursor.lean",
            ("theorem next_seq_from_zero",),
        ),
        (
            "cl",
            "formal/aeneas/out/lean/ClKernel.lean",
            "def keep_body_without_cl",
            "./scripts/aeneas_cl.sh",
            "crates/pedradb-http/src/cl_kernel.rs",
            "formal/aeneas/lean/Cl.lean",
            ("theorem keep_body_without_cl_true",),
        ),
        (
            "children",
            "formal/aeneas/out/lean/ChildrenKernel.lean",
            "def PACKED_CHILD_END",
            "./scripts/aeneas_children.sh",
            "crates/montanha-fdb-recipes/src/children_kernel.rs",
            "formal/aeneas/lean/Children.lean",
            ("theorem packed_child_end_byte",),
        ),
        (
            "pin",
            "formal/aeneas/out/lean/PinKernel.lean",
            "def may_advance_pin",
            "./scripts/aeneas_pin.sh",
            "crates/pedradb-journal/src/pin_kernel.rs",
            "formal/aeneas/lean/Pin.lean",
            ("theorem may_advance_pin_forward",),
        ),
        (
            "pack",
            "formal/aeneas/out/lean/PackKernel.lean",
            "def pack_cut_tag",
            "./scripts/aeneas_pack.sh",
            "crates/montanha-fdb-recipes/src/pack_kernel.rs",
            "formal/aeneas/lean/Pack.lean",
            ("theorem pack_cut_tag_identity",),
        ),
        (
            "ship",
            "formal/aeneas/out/lean/ShipKernel.lean",
            "def stamp_changed",
            "./scripts/aeneas_ship.sh",
            "crates/pedradb-replicate/src/ship_kernel.rs",
            "formal/aeneas/lean/Ship.lean",
            ("theorem stamp_changed_is_def",),
        ),
        (
            "fold",
            "formal/aeneas/out/lean/FoldKernel.lean",
            "def fold_event_hides_key",
            "./scripts/aeneas_fold.sh",
            "crates/pedradb-fold/src/fold_kernel.rs",
            "formal/aeneas/lean/Fold.lean",
            ("theorem fold_event_hides_key_is_def",),
        ),
        (
            "manifest",
            "formal/aeneas/out/lean/ManifestKernel.lean",
            "def sst_recover_action",
            "./scripts/aeneas_manifest.sh",
            "crates/pedradb-core/src/manifest_kernel.rs",
            "formal/aeneas/lean/Manifest.lean",
            ("theorem sst_recover_absent_scans",),
        ),
        (
            "compact",
            "formal/aeneas/out/lean/CompactKernel.lean",
            "def compact_pick",
            "./scripts/aeneas_compact.sh",
            "crates/pedradb-core/src/compact_kernel.rs",
            "formal/aeneas/lean/Compact.lean",
            ("theorem compact_pick_empty_noop",),
        ),
        (
            "vlog_gc",
            "formal/aeneas/out/lean/VlogGcKernel.lean",
            "def vlog_recover_action",
            "./scripts/aeneas_vlog_gc.sh",
            "crates/pedradb-core/src/vlog_gc_kernel.rs",
            "formal/aeneas/lean/VlogGc.lean",
            ("theorem vlog_recover_blob_opens",),
        ),
        (
            "tx_glue",
            "formal/aeneas/out/lean/TxGlueKernel.lean",
            "def tx_range_action",
            "./scripts/aeneas_tx_glue.sh",
            "crates/pedradb-store/src/tx_glue_kernel.rs",
            "formal/aeneas/lean/TxGlue.lean",
            ("theorem tx_range_keep_committed",),
        ),
        (
            "l28",
            "formal/aeneas/out/lean/L28Kernel.lean",
            "def l28_durability_ok",
            "./scripts/aeneas_l28.sh",
            "crates/pedradb-store/src/l28.rs",
            "formal/aeneas/lean/L28.lean",
            ("theorem l28_durability_all_ok",),
        ),
        (
            "tcg",
            "formal/aeneas/out/lean/TcgKernel.lean",
            "def tcg_guest_admitted",
            "./scripts/aeneas_tcg.sh",
            "crates/pedradb-world/src/tcg.rs",
            "formal/aeneas/lean/Tcg.lean",
            ("theorem tcg_guest_admitted_true",),
        ),
        (
            "cqe",
            "formal/aeneas/out/lean/CqeKernel.lean",
            "def cqe_res_ok",
            "./scripts/aeneas_cqe.sh",
            "crates/pedradb-io-uring/src/cqe_kernel.rs",
            "formal/aeneas/lean/Cqe.lean",
            ("theorem cqe_res_ok_nonneg",),
        ),
        (
            "iter",
            "formal/aeneas/out/lean/IterKernel.lean",
            "def iter_window_keep",
            "./scripts/aeneas_iter.sh",
            "crates/rocksdb-compat/src/iter_kernel.rs",
            "formal/aeneas/lean/Iter.lean",
            ("theorem iter_window_keep_live",),
        ),
        (
            "properties",
            "formal/aeneas/out/lean/PropertiesKernel.lean",
            "def d1_holds_loop.body",
            "./scripts/aeneas_properties.sh",
            "crates/pedradb-spec/src/properties_kernel.rs",
            "formal/aeneas/lean/Properties.lean",
            ("theorem d1_holds_loop_body_is_def",),
        ),
        (
            "scale",
            "formal/aeneas/out/lean/ScaleKernel.lean",
            "def point_get_probes",
            "./scripts/aeneas_scale.sh",
            "crates/pedradb-core/src/scale_kernel.rs",
            "formal/aeneas/lean/Scale.lean",
            ("theorem point_get_probes_one_plus_one",),
        ),
        (
            "disk_pressure",
            "formal/aeneas/out/lean/DiskPressureKernel.lean",
            "def disk_pressure_admit",
            "./scripts/aeneas_disk_pressure.sh",
            "crates/pedradb-core/src/disk_pressure_kernel.rs",
            "formal/aeneas/lean/DiskPressure.lean",
            ("theorem disk_pressure_unknown_admits",),
        ),
        (
            "crc",
            "formal/aeneas/out/lean/CrcKernel.lean",
            "def crc_match_ok",
            "./scripts/aeneas_crc.sh",
            "crates/pedradb-core/src/wal/crc.rs",
            "formal/aeneas/lean/Crc.lean",
            ("theorem crc_match_ok_equal",),
        ),
        (
            "lsm_r1",
            "formal/aeneas/out/lean/LsmR1Kernel.lean",
            "def lsm_reopen",
            "./scripts/aeneas_lsm_r1.sh",
            "crates/pedradb-core/src/lsm_r1_kernel.rs",
            "formal/aeneas/lean/LsmR1.lean",
            ("theorem lsm_reopen_id", "theorem lsm_compact_depth_zero"),
        ),
        (
            "t1_modelo",
            "formal/aeneas/out/lean/T1ModeloKernel.lean",
            "def t1_modelo_kernel.tx_recover",
            "./scripts/aeneas_t1_modelo.sh",
            "crates/pedradb-store/src/t1_modelo_kernel.rs",
            "formal/aeneas/lean/T1Modelo.lean",
            ("theorem t1_modelo_empty", "theorem t1_modelo_as_is_dente"),
        ),
        (
            "d1_modelo",
            "formal/aeneas/out/lean/D1ModeloKernel.lean",
            "def d1_modelo_kernel.put_ok",
            "./scripts/aeneas_d1_modelo.sh",
            "crates/pedradb-core/src/d1_modelo_kernel.rs",
            "formal/aeneas/lean/D1Modelo.lean",
            ("theorem d1_modelo_unacked_vacuous", "theorem d1_modelo_as_is_dente"),
        ),
        (
            "c1_modelo",
            "formal/aeneas/out/lean/C1ModeloKernel.lean",
            "def c1_modelo_kernel.c1_modelo",
            "./scripts/aeneas_c1_modelo.sh",
            "crates/pedradb-raft/src/c1_modelo_kernel.rs",
            "formal/aeneas/lean/C1Modelo.lean",
            ("theorem c1_modelo_joint_add_refuses", "theorem c1_modelo_as_is_dente"),
        ),
    ]:
        art = root / artifact
        if art.is_file() and marker in art.read_text(encoding="utf-8"):
            r.good(f"aeneas extract artifact has {marker}")
        else:
            r.gap(
                f"aeneas extract artifact {artifact.rsplit('/', 1)[-1]} missing (run {regen})"
            )
        st = root / f"formal/aeneas/out/SOURCE.{stamp}"
        kr = root / src
        if st.is_file() and kr.is_file():
            want = None
            for line in st.read_text(encoding="utf-8").splitlines():
                if line.startswith("sha256="):
                    want = line.split("=", 1)[1].strip()
            have = hashlib.sha256(kr.read_bytes()).hexdigest()
            if want and have == want:
                r.good(f"aeneas SOURCE.{stamp} sha256 matches {src.rsplit('/', 1)[-1]}")
            elif want:
                r.fail(
                    f"aeneas SOURCE.{stamp} drifted (kernel {have[:12]}… vs stamp {want[:12]}…; re-run {regen})"
                )
        else:
            r.gap(f"aeneas SOURCE.{stamp} missing (run {regen})")
        tf = root / thy
        if tf.is_file():
            tt = tf.read_text(encoding="utf-8")
            if re.search(r"\bsorry\b", tt):
                r.fail(f"{thy.rsplit('/', 1)[-1]} contains sorry")
            elif all(t in tt for t in theorems):
                r.good(f"{thy.rsplit('/', 1)[-1]} theorems (no sorry)")
            else:
                r.fail(f"{thy.rsplit('/', 1)[-1]} missing named theorems")
        else:
            r.fail(f"{thy} missing")
    # RFC-0170 P2.3: D1/R1/T1/C1 twins cite close production fns.
    # All four mirrors (lsm_r1, t1_modelo, d1_modelo, c1_modelo) swept to
    # single-artifact 2026-09-09: close citation is enforced by the
    # theory-check block entries above — twin==kernel + extract artifact
    # + LsmR1.lean/T1Modelo.lean/D1Modelo.lean/C1Modelo.lean theorems.
    # (The twin-file cites loop is gone: no twin files remain to grep.)

    extracts_script = root / "scripts/lean_extracts.sh"
    if charon_required:
        p = subprocess.run(
            ["bash", str(extracts_script), "--required"],
            cwd=root,
        )
        if p.returncode != 0:
            r.fail(f"lean_extracts.sh exit {p.returncode}")
        else:
            r.good("lean_extracts.sh (new extracts)")
    if not (want_charon or charon_required):
        return
    script = root / "scripts/aeneas_vote.sh"
    extra = ["--required"] if charon_required else []
    p = subprocess.run(["bash", str(script), *extra], cwd=root)
    if p.returncode != 0:
        r.fail(f"aeneas_vote.sh exit {p.returncode}")
    elif charon_required:
        r.good("aeneas_vote.sh (Charon+Aeneas)")


def check_verus(root: Path, catalog: dict, r: Report, required: bool) -> None:
    print("== verus (optional unless --verus-required) ==")
    verus = find_verus(root)
    if not verus:
        msg = "verus binary not found (set VERUS= or install ~/.local/verus/verus-arm64-macos)"
        if required:
            r.fail(msg)
        else:
            print(f"skip  {msg}")
        return
    print(f"      verus={verus}")
    for pair in catalog["pairs"]:
        if pair.get("status") == "absent":
            continue
        script = pair.get("verus")
        if not script:
            continue
        if not (root / pair["twin"]).is_file():
            r.fail(f"{pair['id']}: twin missing, skip verus")
            continue
        p = subprocess.run(["bash", str(root / script)], cwd=root)
        if p.returncode != 0:
            msg = f"verus {pair['id']} exit {p.returncode}"
            if required:
                r.fail(msg)
            else:
                print(f"warn  {msg} (not fatal; pass --verus-required to fail)")
        else:
            r.good(f"verus {pair['id']}")


# RFC-0157 P1.1: workspace-wide class guards — the three RFC-0156 classes.
# A site without its gate needs an inline waiver naming a REGISTERED
# residual id: `RFC0157-WAIVER(R-unsafe-posix): ...`. A waiver naming an
# unknown id fails. The scan covers production code only (before the
# first `#[cfg(test)]` / `mod tests {` marker).
FFI_RC_FNS = ("fdatasync(", "fcntl(", "fallocate(", "fsync(", "posix_fadvise(")
FFI_GATE_TOKENS = ("posix_rc_to_io(rc)", "rc == 0", "rc != 0", "raw_os_error")
CQE_ROUTE_TOKENS = ("cqe_act(", "next_user_data(", "submit_complete_act(")
CAP_CAPI_LEN = re.compile(r"([A-Za-z_][A-Za-z0-9_]*len[A-Za-z0-9_]*)\s*:\s*usize")
CAPI_FN = re.compile(r'pub unsafe extern "C" fn\s+([A-Za-z0-9_]+)\s*\(')
WAIVER_RE = re.compile(r"RFC0157-WAIVER\(\s*(R-[A-Za-z0-9_-]+)\s*\)")


def _production_lines(path: Path) -> list[str]:
    """File lines before the test module (test code is out of scope).

    Cuts at `mod tests {` — including the `#[cfg(test)]` attribute line
    above it when present. Inline `#[cfg(test)]` items (test-only fields
    inside production structs, as in ring.rs) do NOT cut: only a module
    boundary does.
    """
    try:
        src = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []
    lines = src.splitlines()
    for i, line in enumerate(lines):
        stripped = line.lstrip()
        attr = stripped.startswith("#[cfg(test)]")
        if stripped.startswith("mod tests {") or (
            attr
            and any(
                lines[j].lstrip().startswith("mod ")
                for j in range(i + 1, min(i + 3, len(lines)))
            )
        ):
            # Drop the attribute line(s) immediately above the module too.
            start = i
            while start > 0 and lines[start - 1].lstrip().startswith("#["):
                start -= 1
            return lines[:start]
    return lines


def check_class_scan(root: Path, r: Report) -> None:
    print("== class scan (RFC-0157 P1.1: 0156 classes, waiver = residual id) ==")
    res = json.loads((root / "scripts/formal/residuals.json").read_text(encoding="utf-8"))
    ids = {row["id"] for row in res["residuals"]}

    def waivers(lines: list[str], lo: int, hi: int) -> set[str]:
        found: set[str] = set()
        for w in lines[max(0, lo) : max(0, hi)]:
            m = WAIVER_RE.search(w)
            if m:
                found.add(m.group(1))
        return found

    def waivered_ok(w: set[str]) -> bool:
        if w & ids:
            return True
        if w:
            r.fail(f"class scan: waiver names unknown residual id(s) {sorted(w)}")
        return False

    files = sorted((root / "crates").glob("*/src/**/*.rs"))
    ffi = capi = cqe = waivered = 0
    for f in files:
        rel = f.relative_to(root).as_posix()
        lines = _production_lines(f)
        n = len(lines)

        # (a) R-unsafe-posix: `unsafe` rc-returning FFI must be gated in
        # the same expression window (same tokens as the 0156 guard test).
        for i, line in enumerate(lines):
            if "unsafe {" not in line:
                continue
            if not any(fn in line for fn in FFI_RC_FNS):
                continue
            ffi += 1
            if waivered_ok(waivers(lines, i - 3, i + 9)):
                waivered += 1
                continue
            window = "\n".join(lines[i : i + 8])
            if not any(t in window for t in FFI_GATE_TOKENS):
                r.fail(
                    f"class R-unsafe-posix: ungated unsafe FFI rc at "
                    f"{rel}:{i + 1}: {line.strip()}"
                )

        # (b) R-unsafe-capi: C ABI length params must be capped
        # (`c_len_admitted` / `MAX_C_`), explicitly dead, or waivered.
        i = 0
        while i < n:
            m = CAPI_FN.search(lines[i])
            if not m:
                i += 1
                continue
            j = i
            sig = lines[i]
            while "{" not in sig and j + 1 < n:
                j += 1
                sig += "\n" + lines[j]
            params = sig[sig.find("(") : sig.rfind(")") + 1]
            lens = CAP_CAPI_LEN.findall(params)
            depth = 0
            started = False
            k = j
            while k < n:
                depth += lines[k].count("{") - lines[k].count("}")
                if "{" in lines[k]:
                    started = True
                if started and depth <= 0:
                    break
                k += 1
            body_txt = "\n".join(lines[j + 1 : k + 1])
            for lp in lens:
                capi += 1
                if waivered_ok(waivers(lines, i - 3, k + 2)):
                    waivered += 1
                    continue
                dead = f"let _ = {lp};" in body_txt
                used = re.search(r"\b" + re.escape(lp) + r"\b", body_txt) is not None
                capped = "c_len_admitted(" in body_txt or "MAX_C_" in body_txt
                if used and not dead and not capped:
                    r.fail(
                        f"class R-unsafe-capi: {rel}:{i + 1} {m.group(1)} "
                        f"param `{lp}: usize` used without cap "
                        f"(c_len_admitted / MAX_C_)"
                    )
            i = k + 1

        # (c) R-uring: CQE tag adoption must route through the cqe kernel
        # (minting via `next_user_data`, harvesting via `cqe_act` /
        # `submit_complete_act`) — never a bare/constant-tag adoption.
        # Matched sites are reads/adoptions (`.user_data` / `user_data(`);
        # import continuations (`next_user_data,`) do not match.
        user_data_site = re.compile(r"\.user_data\b|\buser_data\s*\(")
        if "pedradb-io-uring" in rel and "cqe_kernel" not in rel:
            for i, line in enumerate(lines):
                if not user_data_site.search(line):
                    continue
                if line.lstrip().startswith(("//", "use ")):
                    continue
                cqe += 1
                if waivered_ok(waivers(lines, i - 3, i + 7)):
                    waivered += 1
                    continue
                window = "\n".join(lines[max(0, i - 6) : i + 7])
                if not any(t in window for t in CQE_ROUTE_TOKENS):
                    r.fail(
                        f"class R-uring: {rel}:{i + 1} user_data site not "
                        f"routed through the cqe kernel: {line.strip()}"
                    )

    r.good(
        f"class scan: {len(files)} files — {ffi} FFI rc sites, "
        f"{capi} C ABI len params, {cqe} CQE tag sites, "
        f"{waivered} waivered"
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lint", action="store_true")
    ap.add_argument("--twins", action="store_true")
    ap.add_argument("--clones", action="store_true")
    ap.add_argument("--scripts", action="store_true")
    ap.add_argument("--models", action="store_true")
    ap.add_argument(
        "--extract",
        action="store_true",
        help="include-crate cargo test + Charon/Aeneas when installed",
    )
    ap.add_argument(
        "--extract-required",
        action="store_true",
        help="fail if Charon/Aeneas are missing",
    )
    ap.add_argument("--verus", action="store_true")
    ap.add_argument(
        "--verus-required",
        action="store_true",
        help="fail if the verus binary is missing",
    )
    ap.add_argument(
        "--strict",
        action="store_true",
        help="fail on catalog status=absent gaps (missing twins)",
    )
    ap.add_argument(
        "--ci",
        action="store_true",
        help="lint + clones + twins + scripts + models + include-crate extract (no Verus/Charon)",
    )
    ap.add_argument(
        "--all",
        action="store_true",
        help="--ci plus Verus when installed",
    )
    args = ap.parse_args()
    selected = any(
        [
            args.lint,
            args.twins,
            args.clones,
            args.scripts,
            args.models,
            args.extract,
            args.extract_required,
            args.verus,
            args.verus_required,
            args.ci,
            args.all,
        ]
    )
    if not selected:
        args.ci = True

    root = Path(__file__).resolve().parents[2]
    catalog = json.loads((root / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    r = Report()

    run_ci = args.ci or args.all
    if args.lint or run_ci:
        check_lint(root, catalog, r)
        check_tcb_freeze(root, catalog, r)
        check_three_teeth(root, catalog, r)
        check_proof_vs_campaign(root, catalog, r)
        check_proof_depth(root, catalog, r)
        check_residuals(root, r, catalog)
        check_class_scan(root, r)
    if args.clones or run_ci:
        check_clones(root, catalog, r)
    if args.twins or run_ci:
        check_twins(root, catalog, r, args.strict)
    if args.scripts or run_ci:
        check_scripts(root, catalog, r, args.strict)
    if args.models or run_ci:
        check_models(root, catalog, r)
    if args.extract or args.extract_required or run_ci:
        check_extract(
            root,
            r,
            want_charon=args.extract or args.extract_required or args.all,
            charon_required=args.extract_required,
        )
    if args.verus or args.verus_required:
        check_verus(root, catalog, r, args.verus_required)
    elif args.all:
        # Optional: try Verus, do not fail the glue on a missing toolchain.
        check_verus(root, catalog, r, required=False)

    print()
    print(
        f"summary: {len(r.ok)} ok, {len(r.gaps)} gap, {len(r.failed)} fail"
    )
    if r.gaps and not args.strict:
        print("gaps are recorded twins (status=absent); pass --strict to fail on them")
    return 1 if r.failed else 0


if __name__ == "__main__":
    sys.exit(main())
