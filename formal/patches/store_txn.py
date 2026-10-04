# Post-extract patch for kernel 'store_txn' — moved verbatim from
# scripts/aeneas_store_txn.sh. Applied by `cargo xtask proof store_txn` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/StoreTxnKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = "core.cmp.Ord.max.default core.cmp.OrdU64 "
new = "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt "
if old in src:
    n = src.count(old)
    src = src.replace(old, new)
    open(p, "w", encoding="utf-8").write(src)
    print(f"      patched Ord.max lt projection ({n})")
elif new in src:
    print("      Ord.max already patched")
else:
    sys.exit("Ord.max patch target not found")
