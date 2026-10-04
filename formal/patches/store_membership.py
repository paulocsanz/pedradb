# Post-extract patch for kernel 'store_membership' — moved verbatim from
# scripts/aeneas_store_membership.sh. Applied by `cargo xtask proof store_membership` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/StoreMembershipKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
n = 0

def repl(old, new, label):
    global src, n
    if old not in src:
        sys.exit(f"patch target not found: {label}")
    src = src.replace(old, new, 1)
    n += 1

repl(
    """def membership_kernel.elect_claim_banner
  (es1 : Bool) (es2 : Bool) (es3 : Bool) : Result Str := do
  sorry
""",
    r'''def membership_kernel.elect_claim_banner
  (es1 : Bool) (es2 : Bool) (es3 : Bool) : Result Str := do
  let b ← membership_kernel.liveness_admitted es1 es2 es3
  if b
  then ok (toStr "eventual-live es1=1 es2=1 es3=1")
  else ok (toStr "bounded-elect not-eventual")
''',
    "elect_claim_banner",
)
repl(
    """def membership_kernel.elect_claim_banner_as_is
  (_es1 : Bool) (_es2 : Bool) (_es3 : Bool) : Result Str := do
  sorry
""",
    r'''def membership_kernel.elect_claim_banner_as_is
  (_es1 : Bool) (_es2 : Bool) (_es3 : Bool) : Result Str := do
  ok (toStr "live")
''',
    "elect_claim_banner_as_is",
)
old = "core.cmp.Ord.max.default core.cmp.OrdU64"
new = "core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt"
k = src.count(old)
if k:
    src = src.replace(old, new)
    n += k
    print(f"      patched Ord.max.default ×{k}")
if "sorry" in src:
    sys.exit("StoreMembershipKernel.lean still contains sorry")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched store_membership ×{n}")
