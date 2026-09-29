---
name: fix
description: >-
  Slash command alias for the incorporate-feedback engine. Systematically proves bugs
  with mechanical oracles, applies principled root-cause fixes, eradicates entire bug classes
  in mass, enforces autonomic self-reconciliation (no brute-force host hacks), and updates
  the system immune layers (audit skills, rules, docs) so regressions are structurally impossible.
  Operates strictly under the "Proibido Ser Preguiçoso" (Zero-Laziness) doctrine.
---

# `/fix` — Incorporate Feedback & Self-Healing Engine

> ### 🛑 DOUTRINA CENTRAL: "PROIBIDO SER PREGUIÇOSO"
> A meta de `/fix` não é apenas remendar o código para o teste passar. A meta é a **blindagem imunológica total**: garantir que nenhum crítico, auditor adversário ou reviewer consiga sequer chegar perto de encontrar uma falha nesse vetor ou em qualquer primo distante dele.

This is the direct slash command entry point for the [`incorporate-feedback`](../incorporate-feedback/SKILL.md) skill.

## Immediate Execution Workflow

When `/fix` is triggered:

1. **Ingest Context**:
   - If user provided arguments with `/fix <issue>`, use that description.
   - Otherwise, inspect the preceding messages in this conversation to extract the latest review, audit, test failure, or compiler diagnostic.

2. **Execute the Anti-Marra 6-Step Cycle**:
   - **Step 1: Map Contract**: State the broken invariant, physical constraint, and root cause.
   - **Step 2: Prove First (Red)**: Write a failing reproduction test or mechanical oracle. DO NOT edit production code yet. Run and observe the deterministic failure.
   - **Step 3: Root-Cause Fix & Proof (Green)**: Implement the structural, architectural fix (make invalid states unrepresentable where possible). Run test and prove clean `PASS`. Verify mutation (reverting fix fails test with 100% kill rate).
   - **Step 4: Mass Class Eradication**: Search the entire codebase for all occurrences of this bug class. Fix all of them and add parameterized/boundary tests covering the entire domain.
   - **Step 5: Self-Reconciliation**: Verify the system cold-boots, recovers from transient failures, and converges autonomously without manual host commands.
   - **Step 6: Evolve Immune System**: Update audit skills (e.g. `hackernews-adversarial-review`), rules, test gates, and `lessons_learned.md`/`ESTADO_VIGENTE.md`.

For detailed instructions and examples, see the full guide in [incorporate-feedback/SKILL.md](../incorporate-feedback/SKILL.md).
