# Post-extract patch for kernel 'fail_closed' — moved verbatim from
# scripts/aeneas_fail_closed.sh. Applied by `cargo xtask proof fail_closed` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/FailClosedKernel.lean']
import re
import sys

p = sys.argv[1]
src = open(p, encoding="utf-8").read()
n = 0

def subn(pat, new, label, count=0):
    global src, n
    src2, k = re.subn(pat, new, src, count=count)
    if count and k != count:
        sys.exit(f"patch target not found: {label} (got {k})")
    if k == 0:
        sys.exit(f"patch target not found: {label}")
    src = src2
    n += k

subn(
    r"axiom core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr\.next\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  core\.str\.iter\.Split P → Result \(\(Option Str\) × \(core\.str\.iter\.Split P\)\)\n",
    "axiom core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next\n"
    "  {P : Type} :\n"
    "  core.str.iter.Split P → Result ((Option Str) × (core.str.iter.Split P))\n",
    "split next axiom",
    1,
)
subn(
    r"impl_def core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr \{P\n"
    r"  : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  core\.iter\.traits\.iterator\.Iterator \(core\.str\.iter\.Split P\) Str := \{\n"
    r"  next :=\n"
    r"    core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr\.next\n"
    r"    clauseInst\n"
    r"  all := fun[\s\S]*?\n\}",
    "impl_def core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr {P\n"
    "  : Type} :\n"
    "  core.iter.traits.iterator.Iterator (core.str.iter.Split P) Str := {\n"
    "  next :=\n"
    "    core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr.next\n"
    "}",
    "split Iterator impl",
    1,
)
subn(
    r"axiom core\.str\.Str\.split\n"
    r"  \{P : Type\} \(clauseInst : sorry /- Could not find: trait_decl_id: \d+-/ P\) :\n"
    r"  Str → P → Result \(core\.str\.iter\.Split P\)\n",
    "axiom core.str.Str.split {P : Type} :\n"
    "  Str → P → Result (core.str.iter.Split P)\n",
    "split axiom",
    1,
)
subn(
    r"core\.str\.Str\.split sorry /- Could not find: trait_impl_id: \d+-/ ",
    "core.str.Str.split ",
    "split calls",
)
subn(
    r"core\.str\.iter\.Split\.Insts\.CoreIterTraitsIteratorIteratorSharedAStr\n"
    r"      sorry /- Could not find: trait_impl_id: \d+-/",
    "core.str.iter.Split.Insts.CoreIterTraitsIteratorIteratorSharedAStr",
    "split inst calls",
)
src2, k = re.subn(
    r"\n  (?:all|any|position|map|filter|collect|max|min) := fun[\s\S]*?(?=\n  \w+ :=|\n\})",
    "",
    src,
)
if k < 1:
    sys.exit(f"patch target not found: Iterator extra fields (got {k})")
src = src2
n += k
if "sorry" in src:
    sys.exit("FailClosedKernel.lean still contains sorry after patches")
open(p, "w", encoding="utf-8").write(src)
print(f"      patched fail_closed ×{n}")
