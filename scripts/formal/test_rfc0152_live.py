#!/usr/bin/env python3
"""RFC-0152: dropping store live_callers must FAIL naming the pair id."""

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


def main() -> int:
    catalog = json.loads((ROOT / "scripts/formal/catalog.json").read_text(encoding="utf-8"))
    live = pf.Report()
    with redirect_stdout(io.StringIO()):
        pf.check_store_live(ROOT, catalog, live)
        pf.check_lint(ROOT, catalog, live)
    if live.failed:
        print("FAIL live RFC-0152:", live.failed)
        return 1
    print("ok live RFC-0152 store live_callers")

    for pid in (
        "vote",
        "ae_entry",
        "grant_persist",
        "ae_ack",
        "commit_raft",
        "joint_election",
        "joint_leave",
        "pending_joint_node",
        "joint_leave_ok",
        "election_grant_from",
        "joint_target",
        "joint_add_target",
        "queued_leave_finish",
        "disk_membership",
        "high_water",
        "participating_member",
        "identity_before_applied",
        "recover_apply",
        "recover_apply_node",
        "recover_truncate",
        "recover_drop_orphan",
        "recover_abort",
        "persist_meta",
        "persist_hist",
        "persist_fence",
        "force_clear",
        "drop_preimages",
        "open_peer_disk",
        "local_id_member",
        "reader_local",
        "discard_uncommitted",
        "discard_leader",
        "removed_step_down",
        "hint_member",
        "drop_repl_slot",
        "drop_sent_through",
        "apply_step",
    ):
        mutant_cat = copy.deepcopy(catalog)
        for p in mutant_cat["pairs"]:
            if p.get("id") == pid:
                p.pop("live_callers", None)
                break
        mutant = pf.Report()
        with redirect_stdout(io.StringIO()):
            pf.check_store_live(ROOT, mutant_cat, mutant)
            pf.check_raft_store_live(ROOT, mutant_cat, mutant)
        if not any(pid in m and "live_callers" in m for m in mutant.failed):
            print("FAIL drop did not name", pid, "got", mutant.failed)
            return 1
        print(f"ok live_callers-drop names {pid}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
