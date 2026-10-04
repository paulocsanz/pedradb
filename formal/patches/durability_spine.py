# Post-extract patch for kernel 'durability_spine' — moved verbatim from
# scripts/aeneas_durability_spine.sh. Applied by `cargo xtask proof durability_spine` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/DurabilitySpineKernel.lean']
import sys, os
p = sys.argv[1]
if not os.path.isfile(p):
    # Aeneas may name the crate file after the crate, not the inner module.
    import glob
    cands = glob.glob(os.path.join(os.path.dirname(p), "*Spine*.lean")) + \
            glob.glob(os.path.join(os.path.dirname(p), "*Durability*.lean"))
    print("      generated:", cands)
    raise SystemExit(0)
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
