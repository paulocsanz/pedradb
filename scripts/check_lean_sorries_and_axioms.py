#!/usr/bin/env python3
"""
RFC-0260 P0 — Gate de Integridade Estrita de Lean 4 (Sorries & Axioms Ratchet).

1. Varre recursivamente TODOS os arquivos em formal/aeneas/lean/*.lean.
2. Falha imediatamente (código 1) se houver qualquer `sorry` ativo (fora de comentários).
3. Cataloga todas as declarações de `axiom` e confere contra o teto máximo permitido
   em scripts/ratchet/lean_axioms_ceiling.json. Novos axiomas falham o gate.
"""

import glob
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
LEAN_DIR = os.path.join(ROOT, "formal", "aeneas", "lean")
CEILING_PATH = os.path.join(ROOT, "scripts", "ratchet", "lean_axioms_ceiling.json")

def audit():
    lean_files = sorted(glob.glob(os.path.join(LEAN_DIR, "*.lean")))

    if not lean_files:
        print(f"FAIL  Nenhum arquivo Lean encontrado sob {LEAN_DIR}", file=sys.stderr)
        sys.exit(1)

    sorries = []
    axioms = []
    theorems = 0

    for fpath in lean_files:
        rel_name = os.path.relpath(fpath, ROOT)
        with open(fpath, "r", encoding="utf-8", errors="replace") as fp:
            lines = fp.readlines()

        in_multiline_comment = False
        for idx, line in enumerate(lines, 1):
            stripped = line.strip()
            if "/-" in stripped:
                in_multiline_comment = True
            if in_multiline_comment:
                if "-/" in stripped:
                    in_multiline_comment = False
                continue

            if stripped.startswith("--"):
                continue

            # Checar sorry, admit, give_up fora de comentários
            if re.search(r"\b(sorry|admit|give_up)\b", line):
                sorries.append((rel_name, idx, stripped))

            # Checar axiomas (suporta nome na mesma linha ou na linha seguinte)
            if re.match(r"^\s*axiom\b", line):
                m = re.match(r"^\s*axiom\s+([^\s:]+)", line)
                if m:
                    ax_name = m.group(1)
                else:
                    next_line = lines[idx].strip() if idx < len(lines) else ""
                    m2 = re.match(r"^([^\s:]+)", next_line)
                    ax_name = m2.group(1) if m2 else "multiline_axiom"
                axioms.append((rel_name, idx, ax_name))

            if stripped.startswith("theorem "):
                theorems += 1

    if sorries:
        print(f"FAIL  Encontrados {len(sorries)} 'sorry's/admit/give_up ativos na árvore Lean!", file=sys.stderr)
        for fn, lno, text in sorries:
            print(f"  {fn}:{lno}: {text}", file=sys.stderr)
        sys.exit(1)

    print(f"ok    zero sorries/admits em {len(lean_files)} arquivos Lean ({theorems} teoremas)")

    # Auditoria de axiomas
    current_axiom_count = len(axioms)
    ceiling = None
    if os.path.exists(CEILING_PATH):
        with open(CEILING_PATH, "r", encoding="utf-8") as fp:
            data = json.load(fp)
            ceiling = data.get("max_axioms")

    CATALOG_PATH = os.path.join(ROOT, "scripts", "ratchet", "lean_axioms_catalog.tsv")
    with open(CATALOG_PATH, "w", encoding="utf-8") as fp:
        fp.write("# file\tline\taxiom_name\n")
        for fn, lno, ax in axioms:
            fp.write(f"{fn}\t{lno}\t{ax}\n")

    if ceiling is None:
        ceiling = current_axiom_count
        with open(CEILING_PATH, "w", encoding="utf-8") as fp:
            json.dump({"max_axioms": ceiling, "current": current_axiom_count, "rfc": "0261"}, fp, indent=2)
        print(f"ok    congelado teto de axiomas: {ceiling}")
    elif current_axiom_count > ceiling:
        print(f"FAIL  Axiomas ({current_axiom_count}) excedem o teto permitido ({ceiling})!", file=sys.stderr)
        for fn, lno, ax in axioms:
            print(f"  {fn}:{lno}: {ax}", file=sys.stderr)
        print(f"      Proibido adicionar novos axiomas sem justificação formal em RFC.", file=sys.stderr)
        sys.exit(1)
    else:
        print(f"ok    axiomas: {current_axiom_count} <= teto {ceiling}")

    print(f"GATE lean_integrity: GREEN")

if __name__ == "__main__":
    audit()
