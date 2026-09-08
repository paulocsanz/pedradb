Verified Detection and Prevention of ConcurrencyAnomalies in Multi-Agent Large Language Model Systems

Title:

Content selection saved. Describe the issue below:

Description:

![](/static/base/1.0.1/images/icons/smileybones-small.svg) arXiv is now an independent nonprofit! [Learn more](https://info.arxiv.org/about) × 

[License: CC BY 4.0](https://info.arxiv.org/help/license/index.html#licenses-available)

arXiv:2606.17182v1 \[cs.LG\] 15 Jun 2026

# Verified Detection and Prevention of Concurrency  
Anomalies in Multi-Agent Large Language Model Systems

 Sajjad Khan ††thanks: S.˜Khan is an independent researcher (e-mail: sajjadanwar200@gmail.com).

###### Abstract

Multi-agent LLM systems share state through memory stores, vector indices, and tool registries. We model such sharing as long-running read–generate–write operations under deterministic-generation semantics—the regime durable-execution engines enforce by deterministic replay—and formalize four concurrency anomalies in TLA+: stale-generation, phantom-tool, causal-cascade, and tool-effect reordering, structural analogues of classical isolation anomalies, each with a TLC counter-example. The exclusion lattice over these anomalies is trivial; the contribution is the mechanically verified *realizability and strict separation of one maximal chain* within it, L0⊊⋯⊊L4L\_{0}\\subsetneq\\cdots\\subsetneq L\_{4}—to our knowledge the first machine-checked consistency hierarchy for such runtimes. A development of 274 Verus obligations (zero assume, zero admit; trust base: two structural axioms and a mutex correspondence) proves the detectors sound and complete against the specifications and each runtime its avoidance set. Three deployed Rust runtimes realize L0L\_{0}–L1L\_{1} (pessimistic locking, serializable snapshot isolation, default-SI), each verified against stale-generation, refined to its state machine; L2L\_{2}–L4L\_{4} are exec-mode-verified with dependency-free prevention twins (A3A\_{3}, A6A\_{6}, A2A\_{2}: 0/10000/1000 versus 1000/10001000/1000), and L2L\_{2} is additionally run live across three model families (A3A\_{3} prevented in all 120120 retracted sessions); L3L\_{3}/L4L\_{4} remain exec-verified. Prevention costs are bounded, not zero: snapshot isolation adds ∼8%{\\sim}8\\% tokens on one workload, pessimistic locking 1.61.6–2.3×2.3\\times—not the order-of-magnitude penalty commonly assumed. We reproduce a silent lost update in ByteDance’s deer-flow, formalizing its fix as a verified L0→L1L\_{0}\\rightarrow L\_{1} refinement, and exhibit tool-effect reordering in LangGraph’s ToolNode on unmodified output, removed by an L3L\_{3} commit-order sequencer. The verified detector, refinements, and realizability artifacts are the contribution; the phenomena and lattice are classical.

###### Index Terms: 

Multi-agent systems, memory consistency, isolation levels, formal methods, TLA+, Verus, large language models. 

## I Introduction

### I-A A constructed failure within a documented pattern

We construct a scenario within the travel-booking workflow used to motivate the SagaLLM \[[1](#bib.bib1 "")\] architecture: a multi-agent system in which one agent reserves a flight while another reserves a hotel, both consulting and updating a shared trip-state record. Suppose the flight-booking agent reads “trip date \=\= 14 June” from shared state, begins a generation phase of several seconds in which it drafts the reservation request, and during that phase the user (acting through a third agent) updates the trip date to 21 June. When the flight agent commits, it submits the original date, producing a booking that contradicts the system’s now-current state. No fault has occurred at any layer of the runtime. Yet the system has produced an external effect that no current state justifies. SagaLLM’s compensating-transactions architecture is one possible response to this failure mode; Atomix’s progress-gated tool calls \[[2](#bib.bib2 "")\] are another. Neither names the phenomen

--- excerpt: commit abort ---
