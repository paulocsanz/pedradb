# Post-extract patch for kernel 'store_vote' — moved verbatim from
# scripts/aeneas_store_vote.sh. Applied by `cargo xtask proof store_vote` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/StoreVoteKernel.lean']
import sys
axiom = (
    "axiom core.option.Option.Insts.CoreCmpPartialEqOption.eq\n"
    "  {T : Type} (cmpPartialEqInst : core.cmp.PartialEq T T) :\n"
    "  Option T → Option T → Result Bool\n"
)
defn = (
    "def core.option.Option.Insts.CoreCmpPartialEqOption.eq\n"
    "  {T : Type} (cmpPartialEqInst : core.cmp.PartialEq T T) :\n"
    "  Option T → Option T → Result Bool\n"
    "  := fun a b =>\n"
    "    match a, b with\n"
    "    | some x, some y => cmpPartialEqInst.eq x y\n"
    "    | none, none => ok true\n"
    "    | _, _ => ok false\n"
)
src = open(sys.argv[1], encoding="utf-8").read()
if axiom in src:
    src = src.replace(axiom, defn, 1)
    open(sys.argv[1], "w", encoding="utf-8").write(src)
    print("      patched Option::eq as match def (RFC-0053 P40)")
