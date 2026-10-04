# Post-extract patch for kernel 'txn' — moved verbatim from
# scripts/aeneas_txn.sh. Applied by `cargo xtask proof txn` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/TxnKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = "core.cmp.Ord.max.default core.cmp.OrdU64"
new = "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"
n = src.count(old)
if n:
    open(p, "w", encoding="utf-8").write(src.replace(old, new))
    print(f"      patched Ord.max.default ×{n} (lt, not Ord inst)")
