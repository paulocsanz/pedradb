# Post-extract patch for kernel 'key' — moved verbatim from
# scripts/aeneas_key.sh. Applied by `cargo xtask proof key` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/KeyKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = (
    "@[reducible]\n"
    "impl_def key.InternalKey.Insts.CoreCmpEq : core.cmp.Eq key.InternalKey := {\n"
    "  partialEqInst := key.InternalKey.Insts.CoreCmpPartialEqInternalKey\n"
    "  assert_fields_are_eq := core.cmp.Eq.assert_fields_are_eq.default\n"
    "    key.InternalKey.Insts.CoreCmpEq\n"
    "}\n"
)
new = (
    "def key.InternalKey.Insts.CoreCmpEq.assert_fields_are_eq\n"
    "  (self : key.InternalKey) : Result Unit := do\n"
    "  ok ()\n"
    "\n"
    "@[reducible]\n"
    "def key.InternalKey.Insts.CoreCmpEq : core.cmp.Eq key.InternalKey := {\n"
    "  partialEqInst := key.InternalKey.Insts.CoreCmpPartialEqInternalKey\n"
    "  assert_fields_are_eq := key.InternalKey.Insts.CoreCmpEq.assert_fields_are_eq\n"
    "}\n"
)
if old in src:
    open(p, "w", encoding="utf-8").write(src.replace(old, new, 1))
    print("      patched InternalKey Eq impl_def")
elif new not in src:
    sys.exit("InternalKey Eq patch target not found")
