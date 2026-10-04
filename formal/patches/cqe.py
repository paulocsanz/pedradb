# Post-extract patch for kernel 'cqe' — moved verbatim from
# scripts/aeneas_cqe.sh. Applied by `cargo xtask proof cqe` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/CqeKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = """def submit_complete_act
  (submit_ok : Bool) (harvested : Bool) : Result SubmitCompleteAct := do
  if harvested
  then ok SubmitCompleteAct.UseHarvested
  else
    if submit_ok
    then ok SubmitCompleteAct.WaitMore
    else
      let a ← F208_WAITMORE_AFTER_SUBMIT_ERR
      let _ ←
        core.sync.atomic.AtomicU64Align8U64.fetch_add a 1#u64
          core.sync.atomic.Ordering.Relaxed
      ok SubmitCompleteAct.WaitMore
"""
new = """def submit_complete_act
  (_submit_ok : Bool) (harvested : Bool) : Result SubmitCompleteAct := do
  if harvested
  then ok SubmitCompleteAct.UseHarvested
  else ok SubmitCompleteAct.WaitMore
"""
if old not in src:
    sys.exit("patch target not found: submit_complete_act Atomic arm")
open(p, "w", encoding="utf-8").write(src.replace(old, new, 1))
print("      patched submit_complete_act Atomic telemetry")
