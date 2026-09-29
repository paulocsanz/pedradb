# Triage & Eradication Checklist: Incorporate Feedback (`/fix`)

> ### 🛑 "PROIBIDO SER PREGUIÇOSO" CHECKLIST
> Use this checklist during each execution of `/fix` to guarantee that no reviewer or auditor can ever get close to this vector again.

---

## 1. Feedback Ingestion & Deep Dissection
- [ ] Read the review / audit / failure log in full.
- [ ] Separate noise/subjective style remarks from concrete correctness, safety, and performance defects.
- [ ] For each technical defect, identify:
  - [ ] Precise failure mechanism (e.g. integer overflow, missing lock, memory leak, unhandled error, silent drop, bad stage-2 mapping).
  - [ ] Violated contract or invariant.
  - [ ] Blast radius in the system.
- [ ] **Zero-Laziness Check**: Did you identify the architectural reason the bug was possible, rather than just the line where it blew up?

---

## 2. Mechanical Oracle (Red Phase — PROIBIDO PULAR)
- [ ] **NO PRODUCTION CODE EDITED YET.** (Editing code before writing the test is strictly forbidden).
- [ ] Create a minimal, deterministic reproduction test or harness scenario.
- [ ] Run the test and capture the exact failure signature:
  - [ ] Panic, assertion failure, timeout, exit code, or diagnostic output.
- [ ] Verify the test fails because of the specific diagnosed bug, not due to ambient environment issues.

---

## 3. Principled Root-Cause Fix (Green Phase)
- [ ] **Make Invalid States Unrepresentable**: Can the type system, non-zero types, private constructors, or `const_assert!` prevent this state from ever compiling?
- [ ] Implement the architectural fix at the root cause level.
- [ ] Avoid lazy antipatterns:
  - [ ] NO `unwrap_or_default()` to hide missing values.
  - [ ] NO arbitrary `thread::sleep` to dodge races.
  - [ ] NO manual host shell hacks ("conserto na marra").
- [ ] Re-run the reproduction test and observe deterministic clean `PASS`.
- [ ] **Catraca de Mutação (Mutation Ratchet)**: Temporarily mutate/invert the fix (flip `<` to `<=`, shift by 1 bit, remove barrier) and verify that the test fails immediately with 100% kill rate.

---

## 4. Bug Class Eradication (Repository Sweep)
- [ ] Define the abstract pattern of the bug class.
- [ ] Formulate search queries (`ripgrep`, regex, AST patterns) across the whole repo.
- [ ] Inspect every match across all crates, modules, and scripts.
- [ ] Apply the principled fix to all sibling instances.
- [ ] **Exhaustive Boundary Matrix**: Add parameterized, property-based, or matrix tests covering:
  - [ ] Boundaries: 0, 1, MAX-1, MAX, MAX+1, wrap-around.
  - [ ] Degenerate inputs: empty strings, truncated buffers, poison payloads.
  - [ ] Environmental stress: out-of-order events, burst traffic, OOM, disk full.

---

## 5. Self-Reconciliation & Autonomic Healing
- [ ] Verify cold boot: does the system start cleanly from zero state without manual intervention?
- [ ] Verify transient failure recovery: if the process is killed or a transient network/disk error occurs, does the system converge to green automatically?
- [ ] Confirm no operator commands are needed to restore normal operation.

---

## 6. Immune System Evolution (A Crítica Nunca Mais Chega Perto)
- [ ] **Arm the Auditor**:
  - [ ] Add the failure mode and boundary tests directly into the checklist of relevant audit skills (e.g., [`hackernews-adversarial-review`](file:///Users/paulo/.gemini/config/skills/hackernews-adversarial-review/SKILL.md)).
- [ ] **Static Tripwires & CI Gates**:
  - [ ] Add compiler lints, `#![deny(...)]`, or custom CI checker scripts that mechanically reject future regressions.
- [ ] **Update Documentation**:
  - [ ] Record root cause, invariant, and prevention rule in `lessons_learned.md` / `ESTADO_VIGENTE.md` / repo docs.
- [ ] Final Verification: Ask yourself — *"Can an adversarial reviewer find any variant of this flaw in the codebase right now?"* If the answer is not an absolute NO, keep hardening.
