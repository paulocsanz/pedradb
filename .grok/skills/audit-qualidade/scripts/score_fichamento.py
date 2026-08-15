#!/usr/bin/env python3
"""Score mecânico D0–D4 das fichas em research/fichamentos/.

Papers, não livros: D3 pede ≥4 citas + template + fonte local + ≥180 linhas.

  python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --root research
  python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --file PATH
  python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --only-below D3
  python3 .grok/skills/audit-qualidade/scripts/score_fichamento.py --min-d3

Score é necessário, não suficiente. Ver research/QUALIDADE.md.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path

QUOTED_SPAN_RE = re.compile(r"[“\"]([^”\"]{24,800})[”\"]")
PAGE_RE = re.compile(
    r"(?i)\b(p{1,2}\.\s*~?\s*\d+|fig(?:ure|\.)?\s*\d+|tab(?:le|\.)?\s*\d+|"
    r"§\s*\d+|sec(?:tion|\.)?\s*\d+|§§?\s*\d+)"
)
ESTRATO_RE = re.compile(
    r"(?im)(estrato[^\n]{0,120}\([abc]\)|fonte prim[aá]ria lida|"
    r"PDF integral|estrato\s*[:\s]*[abc])"
)
FONTES_LINK_RE = re.compile(
    r"(?i)(fontes/|docs/references/|\.pdf|\.txt|Arquivo lido)"
)
NAO_LIDO_RE = re.compile(
    r"(?i)(n[aã]o (foi|foram) lido|fora do escopo|parcial|estrato\s*\(b\)|"
    r"amostra|residual|ainda n[aã]o)"
)
D4B_RE = re.compile(
    r"(?i)(\*\*Tier:\s*D4\*\*\s*—\s*via\s*\*\*D4-b\*\*|"
    r"D4-b[^\n]{0,80}(PDF|original))"
)
BANNER_INVALIDO_RE = re.compile(
    r"(?i)n[aã]o [eé] fichamento v[aá]lido|STATUS:\s*N[AÃ]O [EÉ] FICHAMENTO"
)
BANNER_AMOSTRA_RE = re.compile(
    r"(?i)amostra seletiva|STATUS:\s*AMOSTRA|📄\s*\*?\*?amostra|\(D2\)"
)

REQUIRED_SECTION_PATTERNS = [
    (r"Refer[eê]ncia", "Referência"),
    (r"Dados da leitura", "Dados da leitura"),
    (r"Estrato", "Estrato"),
    (r"Resumo|S[ií]ntese", "Resumo/Síntese"),
    (r"Tese central", "Tese central"),
    (r"Estrutura", "Estrutura"),
    (r"Conceitos", "Conceitos"),
    (r"##\s*Citaç", "Citações"),
    (r"##\s*Di[aá]logo", "Diálogo"),
    (r"Rela[cç][aã]o com (Pedra|a pesquisa)", "Relação"),
    (r"##\s*Avalia[cç][aã]o", "Avaliação"),
    (r"Palavras-chave", "Palavras-chave"),
    (r"##\s*Fontes|Arquivo lido|fontes/|docs/references/", "Fontes"),
]

TIER_ORDER = {"D0": 0, "D1": 1, "D2": 2, "D3": 3, "D4": 4}


def count_numbered_citations(text: str) -> int:
    m = re.search(r"(?im)^##\s*Citaç[^\n]*\n", text)
    if not m:
        return len(re.findall(r"(?m)^\s*\d+\.\s+[\"“]", text[:40000]))
    rest = text[m.end() : m.end() + 80000]
    m2 = re.search(r"(?m)^##\s+\S", rest)
    block = rest[: m2.start()] if m2 else rest
    return len(re.findall(r"(?m)^\s*\d+\.\s+\S", block))


def sections_ok(text: str) -> tuple[int, list[str]]:
    missing = []
    ok = 0
    for pat, label in REQUIRED_SECTION_PATTERNS:
        if re.search(pat, text, re.I):
            ok += 1
        else:
            missing.append(label)
    return ok, missing


def is_survey_or_long(text: str) -> bool:
    return bool(
        re.search(r"(?i)\b(survey|TODS|TOCS|CSUR|journal longo|≥\s*16\s*p)\b", text[:2500])
    )


def score_fichamento(path: Path, text: str) -> dict:
    lines = text.count("\n") + (1 if text and not text.endswith("\n") else 0)
    head = text[:12000]
    n_citas = count_numbered_citations(text)
    n_spans = len(QUOTED_SPAN_RE.findall(text[:40000]))
    n_pages = len(PAGE_RE.findall(text[:80000]))
    has_fontes = bool(FONTES_LINK_RE.search(text))
    n_estrato = len(ESTRATO_RE.findall(head))
    has_nao_lido = bool(NAO_LIDO_RE.search(head))
    has_banner_inv = bool(BANNER_INVALIDO_RE.search(head))
    has_banner_amostra = bool(BANNER_AMOSTRA_RE.search(head))
    n_sec, missing = sections_ok(text)
    full_template = (
        "Citações" not in missing
        and "Relação" not in missing
        and "Avaliação" not in missing
        and "Estrato" not in missing
        and n_sec >= 10
    )

    reasons: list[str] = []
    tier = "D0"
    if has_banner_inv:
        tier = "D1"
        reasons.append("banner_invalido")
    elif has_banner_amostra:
        tier = "D2"
        reasons.append("banner_amostra")

    cit_floor = 4
    line_floor = 250 if is_survey_or_long(text) else 180
    if n_citas == 0 and n_spans >= 8:
        n_citas = min(n_spans // 2, 8)
        reasons.append("citas_via_spans")

    if not has_banner_inv:
        if (
            full_template
            and n_citas >= cit_floor
            and has_fontes
            and n_estrato >= 1
            and lines >= line_floor
        ):
            tier = "D3"
            reasons.append("d3_template+citas+fontes")
            if D4B_RE.search(text):
                tier = "D4"
                reasons.append("d4b_pdf_original")
        elif full_template and has_fontes and (n_citas >= 2 or n_pages >= 4):
            if tier == "D0":
                tier = "D2"
                reasons.append("amostra_densa_insuficiente")
        elif has_fontes and not full_template:
            if tier == "D0":
                tier = "D1"
                reasons.append("nota_com_fonte")

    if re.search(r"(?i)estrato\s*\(b\)", text) and not has_nao_lido and tier in ("D3", "D4"):
        reasons.append("estrato_b_sem_residual")

    return {
        "path": str(path),
        "tier": tier,
        "lines": lines,
        "n_citas": n_citas,
        "n_locus": n_pages,
        "n_sections": n_sec,
        "missing_sections": missing,
        "has_fontes": has_fontes,
        "reasons": reasons,
    }


def find_fichas(root: Path) -> list[Path]:
    hits = []
    for pat in ("fichamentos/ficha_*.md", "**/fichamentos/ficha_*.md"):
        hits.extend(root.glob(pat))
    # skip templates
    return sorted({p.resolve() for p in hits if p.name != "template.md"})


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=Path, default=Path("research"))
    ap.add_argument("--file", type=Path)
    ap.add_argument("--only-below", choices=list(TIER_ORDER))
    ap.add_argument("--min-d3", action="store_true")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    if args.file:
        paths = [args.file]
    else:
        paths = find_fichas(args.root)

    rows = []
    for p in paths:
        if not p.is_file():
            print(f"missing {p}", file=sys.stderr)
            return 2
        rows.append(score_fichamento(p, p.read_text(encoding="utf-8", errors="replace")))

    if args.only_below:
        cut = TIER_ORDER[args.only_below]
        rows = [r for r in rows if TIER_ORDER[r["tier"]] < cut]

    if args.json:
        print(json.dumps(rows, indent=2, ensure_ascii=False))
    else:
        if not rows:
            print("no fichas (ok if catalog is still listed-only)")
        counts: Counter[str] = Counter(r["tier"] for r in rows)
        for r in rows:
            miss = ",".join(r["missing_sections"][:6])
            print(
                f"{r['tier']:3} {r['lines']:4}L citas={r['n_citas']:<2} "
                f"{Path(r['path']).name}  {';'.join(r['reasons']) or '-'}  "
                f"miss=[{miss}]"
            )
        if counts:
            print("—", " ".join(f"{k}={counts[k]}" for k in ("D0", "D1", "D2", "D3", "D4") if counts[k]))

    if args.min_d3 and any(TIER_ORDER[r["tier"]] < 3 for r in rows):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
