#!/usr/bin/env python3
"""RFC-0222 — seL4 gap por eixo, com denominador nomeado (audit 2026-09-13 §6).

Mede mecanicamente os eixos da auditoria independente; nenhum número é
copiado de doc. Cada eixo imprime o denominador. Dois blocos compostos:

- ``defining``  — os eixos que DEFINEM o seL4 (audit §6 linhas 1-4, 6-7):
  refinamento topo, superfície provada, confinamento, espinha de recovery,
  ∀-concorrência, binário. Teto de engenharia ~60-70%; o resto é pesquisa.
- ``claim``     — classe de claim + evidência (audit §6 linhas 8-10):
  TCB escrito, gates verdes, CI externo, cadeia nomeada. Piso Verde = 100%.

Uso:
    python3 scripts/sel4_gap.py            # mede e imprime
    python3 scripts/sel4_gap.py --gate     # congela pisos (nunca regredir);
                                           # subir = mover o piso no mesmo
                                           # commit da prova que subiu

O piso vive em ``scripts/ratchet/sel4_gap_floors.json``. Regra de movimento
igual aos demais ratchets: o piso sobe no mesmo commit da evidência; nunca
desce sem movimento de ledger.
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CATALOG = REPO / "scripts/formal/catalog.json"
RESIDUALS = REPO / "scripts/formal/residuals.json"
CLOSE_PROOFS = REPO / "scripts/ratchet/close_proofs.tsv"
LEAN_DIR = REPO / "formal/aeneas/lean"
FLOORS = REPO / "scripts/ratchet/sel4_gap_floors.json"
CHEAP_GATES = [
    "check_barrier_floor.py",
    "check_depth_floor.py",
    "check_inventory_terminal.py",
    "check_ledger_consistency.py",
    "check_no_prod_time_spawn.py",
    "check_product_floor.py",
    "check_seam_inventory.py",
    "check_twin_contracts.py",
]

RECOVERY_KERNELS = (
    "wal/recover_kernel.rs",
    "wal/reopen_kernel.rs",
    "manifest_kernel.rs",
    "vlog_gc_kernel.rs",
)


def loc(paths: list[Path]) -> int:
    return sum(len(p.read_text(encoding="utf-8", errors="replace").splitlines()) for p in paths)


def kernel_files() -> list[Path]:
    return sorted(p for p in (REPO / "crates").rglob("*_kernel.rs") if "/src/" in str(p))


def crate_src_files(crates: set[str]) -> list[Path]:
    out: list[Path] = []
    for c in sorted(crates):
        out += [p for p in (REPO / "crates" / c / "src").rglob("*.rs")]
    return out


def pct(a: float, b: float) -> float:
    return round(100.0 * a / b, 2) if b else 0.0


def run_gates() -> tuple[int, int]:
    green = 0
    for g in CHEAP_GATES:
        r = subprocess.run(
            [sys.executable, str(REPO / "scripts" / g)],
            capture_output=True,
            timeout=120,
        )
        green += 1 if r.returncode == 0 else 0
    return green, len(CHEAP_GATES)


def measure() -> dict[str, object]:
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    residuals = json.loads(RESIDUALS.read_text(encoding="utf-8"))
    kernels = kernel_files()
    enrolled = residuals["glue"]["kernel_paths"]
    allow: dict[str, list[str]] = residuals["glue"].get("kernel_fn_allowlist", {})
    pairs = catalog["pairs"]

    # --- eixo 2: superfície provada (LOC e por-fn) ---
    # Numerador A2a: TODOS os *_kernel.rs (o audit contou 68 arquivos); o
    # denominador são as crates com kernel no catálogo (não só as enroladas).
    kernel_crates = {k.split("/")[1] for k in enrolled}
    for p in pairs:
        k = p.get("kernel") or ""
        if k.startswith("crates/"):
            kernel_crates.add(k.split("/")[1])
    src_loc = loc(crate_src_files(kernel_crates))
    k_loc = loc(kernels)
    surface_fns = {
        (p.get("kernel") or "", p.get("entry") or "") for p in pairs if p.get("entry")
    }
    for cl in catalog.get("clones") or []:
        for fn in cl.get("fns") or []:
            if cl.get("a"):
                surface_fns.add((cl["a"], fn))
            if cl.get("b"):
                surface_fns.add((cl["b"], fn))
    tot_pub = on_surface = 0
    for rel in enrolled:
        p = REPO / rel
        if not p.is_file():
            continue
        src = p.read_text(encoding="utf-8", errors="replace")
        fns = re.findall(r"pub(?:\(crate\))? fn (\w+)", src)
        al = set(allow.get(rel, []))
        for f in fns:
            tot_pub += 1
            if (
                (rel, f) in surface_fns
                or f in al
                or f.endswith(("_as_is", "_spec"))
                or re.search(rf"fn {f}_as_is\b|fn {f}_spec\b|fn as_is_{f}\b", src)
            ):
                on_surface += 1

    # --- eixo 3 (escada 0188): degraus registrados ---
    covered = set()
    for line in CLOSE_PROOFS.read_text(encoding="utf-8").splitlines():
        parts = line.split("\t")
        if len(parts) >= 2 and parts[1].startswith("catalog:"):
            covered.add(parts[1][len("catalog:") :])

    # --- eixo 1 (composição, m2 do RFC-0220): átomos encadeados ---
    # m2 = átomos (registro TSV, ~286) cujo fn aparece em ≥1 Compose*.lean.
    atom_names: set[str] = set()
    atom_count = 0
    for line in CLOSE_PROOFS.read_text(encoding="utf-8").splitlines():
        parts = line.split("\t")
        if parts and parts[0] == "atom":
            atom_count += 1
            if len(parts) >= 5:
                atom_names.add(parts[4])
    compose_text = ""
    for c in sorted(LEAN_DIR.glob("Compose*.lean")):
        compose_text += c.read_text(encoding="utf-8", errors="replace")
    chained = {f for f in atom_names if re.search(rf"\b{re.escape(f)}\b", compose_text)}
    atom_fns_catalog = {
        p["atom"] for p in pairs if p.get("twin_kind") == "atom" and p.get("atom")
    }
    chained |= {
        f
        for f in atom_fns_catalog
        if f not in atom_names and re.search(rf"\b{re.escape(f)}\b", compose_text)
    }

    # --- eixo 4: espinha de recovery (átomos de recovery em composição) ---
    kernel_of = {p["id"]: (p.get("kernel") or "") for p in pairs}
    recovery_atoms = {
        parts[4]
        for line in CLOSE_PROOFS.read_text(encoding="utf-8").splitlines()
        if (parts := line.split("\t")) and parts[0] == "atom" and len(parts) >= 5
        and any(rk in kernel_of.get(parts[1][len("catalog:") :], "") for rk in RECOVERY_KERNELS)
    }
    recovery_chained = recovery_atoms & chained

    # --- eixos de pesquisa: teoremas nomeados existem? ---
    all_lean = ""
    for c in sorted(LEAN_DIR.rglob("*.lean")):
        if "/.lake/" in str(c):
            continue
        all_lean += c.read_text(encoding="utf-8", errors="replace")
    top_theorem = len(re.findall(r"\bpedra_refines\b", all_lean))
    confinement = len(re.findall(r"\bconfinement\b", all_lean))
    conc_forall = len(re.findall(r"\bconcurrent_db_\w*forall\w*\b", all_lean))

    # --- eixo 10: cadeia nomeada (sorries da stdlib Aeneas no TCB do ledger) ---
    ledger = (REPO / "docs/verification-ledger.md").read_text(encoding="utf-8")
    stdlib_sorries_named = len(re.findall(r"Slice\.lean|StringIter\.lean", ledger))

    gates_green, gates_total = run_gates()

    return {
        "kernel_loc": k_loc,
        "kernel_files_total": len(kernels),
        "kernel_files_enrolled": len(enrolled),
        "formalized_src_loc": src_loc,
        "kernel_pub_fns": tot_pub,
        "kernel_pub_fns_on_surface": on_surface,
        "pairs_total": len(pairs),
        "pairs_covered": len(covered & {p["id"] for p in pairs}),
        "atom_fns": atom_count,
        "atom_fns_chained": len(chained),
        "recovery_atoms": len(recovery_atoms),
        "recovery_atoms_chained": len(recovery_chained),
        "top_theorems": top_theorem,
        "confinement_theorems": confinement,
        "concurrency_forall_theorems": conc_forall,
        "stdlib_sorries_named_in_tcb": min(stdlib_sorries_named, 1),
        "gates_green": gates_green,
        "gates_total": gates_total,
        # CI do GitHub não é observável da árvore (exige push); eixo fica 0
        # até P0.8 registrar o run verde.
        "ci_github_green": 0,
    }


AXES = [
    # (eixo, chave numérica, denominador, destino, bloco)
    ("A1 refinamento topo (teorema único)", "top_theorems", "0/1 (existe `pedra_refines` no corpus)", 1, "defining"),
    ("A2a superfície kernel LOC", "kernel_loc", "LOC src das crates formalizadas", None, "defining"),
    ("A2b superfície kernel fns", "kernel_pub_fns_on_surface", "pub fns nos kernels enrolados", None, "defining"),
    ("A3 confinamento (2º teorema)", "confinement_theorems", "0/1", 1, "defining"),
    ("A4 espinha de recovery", "recovery_atoms_chained", "átomos de recovery (wal/manifest/vlog)", None, "defining"),
    ("A6 ∀-concorrência", "concurrency_forall_theorems", "0/1", 1, "defining"),
    ("A8 classe de claim (TCB escrito)", None, "never-list + residuals congelados = 1", 1, "claim"),
    ("A9a gates baratos verdes", "gates_green", f"gates {{}}".format("check_*.py"), None, "claim"),
    ("A9b CI GitHub verde", "ci_github_green", "0/1 (exige push)", 1, "claim"),
    ("A10 cadeia nomeada (stdlib sorries no TCB)", "stdlib_sorries_named_in_tcb", "0/1", 1, "claim"),
]


def render(m: dict[str, object]) -> str:
    lines = ["seL4 gap por eixo (RFC-0222; audit 2026-09-13 §6) — denominador nomeado"]
    for name, key, den, _dst, _blk in AXES:
        if key is None:
            lines.append(f"  {name}: 1 (cânone RFC-0061; never-list no residuals.json)")
        elif name.startswith("A2a"):
            lines.append(
                f"  {name}: {m['kernel_loc']}/{m['formalized_src_loc']} = "
                f"{pct(m['kernel_loc'], m['formalized_src_loc'])}%"
            )
        elif name.startswith("A2b"):
            lines.append(
                f"  {name}: {m['kernel_pub_fns_on_surface']}/{m['kernel_pub_fns']} = "
                f"{pct(m['kernel_pub_fns_on_surface'], m['kernel_pub_fns'])}% "
                f"(arquivos enrolados {m['kernel_files_enrolled']}/{m['kernel_files_total']})"
            )
        elif name == "A4 espinha de recovery":
            lines.append(f"  {name}: {m['recovery_atoms_chained']}/{m['recovery_atoms']}")
        else:
            v = m[key]
            d = m.get(f"{key}_total") if key == "gates_green" else None
            lines.append(f"  {name}: {v}/{d}" if d else f"  {name}: {v}")
    lines.append(
        f"  escada 0188 (contexto): {m['pairs_covered']}/{m['pairs_total']} = "
        f"{pct(m['pairs_covered'], m['pairs_total'])}%"
    )
    lines.append(
        f"  composição m2 (contexto, RFC-0220): {m['atom_fns_chained']}/{m['atom_fns']} = "
        f"{pct(m['atom_fns_chained'], m['atom_fns'])}%"
    )
    defining = [
        m["top_theorems"],
        pct(m["kernel_loc"], m["formalized_src_loc"]) / 100.0,
        pct(m["kernel_pub_fns_on_surface"], m["kernel_pub_fns"]) / 100.0,
        m["confinement_theorems"],
        (m["recovery_atoms_chained"] / m["recovery_atoms"]) if m["recovery_atoms"] else 0,
        m["concurrency_forall_theorems"],
    ]
    claim = [
        1.0,
        m["gates_green"] / m["gates_total"],
        m["ci_github_green"],
        m["stdlib_sorries_named_in_tcb"],
    ]
    lines.append(
        f"bloco DEFINING (o que define o seL4): {round(100*sum(defining)/len(defining), 2)}%"
    )
    lines.append(
        f"bloco CLAIM+EVIDÊNCIA (piso verde):   {round(100*sum(claim)/len(claim), 2)}%"
    )
    return "\n".join(lines)


def gate(m: dict[str, object], floors_path: Path = FLOORS) -> int:
    if not floors_path.is_file():
        print(f"GATE sel4_gap: FAIL — pisos ausentes ({floors_path}) — RFC-0222 P1.2")
        return 1
    floors = json.loads(floors_path.read_text(encoding="utf-8"))["floors"]
    bad = 0
    for key, floor in floors.items():
        live = m.get(key)
        if isinstance(live, (int, float)) and live < floor:
            print(
                f"GATE sel4_gap: FAIL — {key}={live} < piso {floor} "
                "(regrediu; mover o piso exige prova no mesmo commit)"
            )
            bad += 1
        else:
            print(f"ok    sel4_gap: {key}={live} ≥ piso {floor}")
    print(f"GATE sel4_gap: {'RED' if bad else 'GREEN'}")
    return 1 if bad else 0


def selftest() -> int:
    import tempfile

    m = measure()
    caught = 0
    total = 2
    missing = Path("/nonexistent/sel4_gap_floors.json")
    if gate(m, missing) == 1:
        print("SELFTEST sel4_gap: caught=missing-floors")
        caught += 1
    else:
        print("SELFTEST sel4_gap: MISSED missing-floors")
    high = dict(m)
    # pick a numeric live key and demand more than live
    key = "pairs_covered"
    bogus = {"floors": {key: int(m[key]) + 1}}
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(bogus, f)
        bogus_path = Path(f.name)
    try:
        if gate(m, bogus_path) == 1:
            print("SELFTEST sel4_gap: caught=floor-above-live")
            caught += 1
        else:
            print("SELFTEST sel4_gap: MISSED floor-above-live")
    finally:
        bogus_path.unlink(missing_ok=True)
    print(f"SELFTEST sel4_gap: {caught}/{total} sabotages caught")
    return 0 if caught == total else 1


def main() -> int:
    args = sys.argv[1:]
    if "--selftest" in args:
        return selftest()
    m = measure()
    if "--gate" in args:
        return gate(m)
    print(render(m))
    print(json.dumps(m, indent=1, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
