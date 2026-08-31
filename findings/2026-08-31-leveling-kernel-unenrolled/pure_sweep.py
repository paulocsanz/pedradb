"""Advisory sweep v2: pure-looking decision fns outside the WHOLE formal surface.

Enrolled = glob *_kernel.rs (non-verus) UNION residuals glue.kernel_paths
UNION every file referenced by catalog.json (pairs kernel/twin/plant,
clones). Leftovers are candidates for the leveling class of hole.
"""
import json
import re
import sys
from pathlib import Path

root = Path("/Users/paulo/software/pedradb")
sys.path.insert(0, "/Users/paulo/software/pedradb/scripts/formal")
import pedra_formal as pf

cat = json.loads((root / "scripts/formal/catalog.json").read_text())
surface = {str(p.relative_to(root)) for p in pf.decision_kernel_paths(root)}
for pair in cat.get("pairs", []):
    for key in ("kernel", "twin"):
        v = pair.get(key)
        if isinstance(v, str):
            surface.add(v)
    plant = pair.get("dst_plant") or {}
    if isinstance(plant, dict) and plant.get("file"):
        surface.add(plant["file"])
    for c in pair.get("callers") or []:
        surface.add(c)
for cl in cat.get("clones", []):
    surface.add(cl["a"])
    surface.add(cl["b"])
print(f"formal-surface files (registry+catalog): {len(surface)}")

IMPURE = re.compile(
    r"&(mut )?self|unsafe\b|\.write\(|\.read\(|\.flush|\.sync|sync_all|sync_file|"
    r"fdatasync|thread::|spawn|\.lock\(|Mutex|RwLock|RefCell|AtomicU|Instant::|"
    r"SystemTime|\.send\(|\.recv\(|File::|OpenOptions|\.remove_|\.create_dir|"
    r"\bBox::pin|async fn|\.await"
)
FN_HEAD = re.compile(
    r"^(?P<pre>(?:pub(?:\(crate\))?\s+)?(?:const\s+)?fn\s+)(?P<name>\w+)\s*\(",
    re.M,
)

hits: dict[str, list[str]] = {}
for p in sorted((root / "crates").glob("*/src/**/*.rs")):
    if "verus" in p.parts or not p.is_file():
        continue
    rel = str(p.relative_to(root))
    if rel in surface:
        continue
    src = p.read_text(encoding="utf-8", errors="replace")
    text = pf.strip_comments(src)
    cfg = text.find("#[cfg(test)]")
    if cfg < 0:
        cfg = len(text)
    names = []
    for m in FN_HEAD.finditer(text[:cfg]):
        start = text.find("{", m.end())
        if start < 0 or start > cfg:
            continue
        depth, i = 0, start
        while i < len(text):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        body = text[start : i + 1]
        head_zone = text[m.start() : start]
        if "-> ()" in head_zone or IMPURE.search(body) or IMPURE.search(head_zone):
            continue
        names.append(m.group("name"))
    if names:
        hits[rel] = names

for rel, names in sorted(hits.items(), key=lambda kv: -len(kv[1])):
    print(f"{len(names):3d}  {rel}: {', '.join(names[:14])}{' …' if len(names) > 14 else ''}")
print(f"\nfiles outside the formal surface with pure-looking fns: {len(hits)}")
