# Post-extract patch for kernel 'ratio_curve' — moved verbatim from
# scripts/aeneas_ratio_curve.sh. Applied by `cargo xtask proof ratio_curve` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/RatioCurveKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
n = 0
for old, new in (
    ("core.cmp.Ord.max.default core.cmp.OrdU64",
     "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"),
    ("core.cmp.Ord.min.default core.cmp.OrdU64",
     "core.cmp.Ord.min.default core.cmp.OrdU64.partialOrdInst.lt"),
):
    k = src.count(old)
    if k:
        src = src.replace(old, new)
        n += k
if n:
    open(p, "w", encoding="utf-8").write(src)
    print(f"      patched Ord.max/min.default ×{n}")
