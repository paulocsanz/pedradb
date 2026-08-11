//! PedraDB deterministic simulation framework.
//!
//! Inspired by FoundationDB's Simulation: run an entire PedraDB instance inside
//! a single-threaded, perfectly reproducible simulation that models disk I/O,
//! time progression, and crash semantics. Randomized workloads + fault injection
//! find bugs that unit tests and integration tests cannot reach.
//!
//! FDB estimates ~1 trillion CPU-hours of simulation. This is the only way to
//! trust a novel compaction strategy (Lazy Leveling) that has no production
//! precedent.
//!
//! Status: placeholder. Implementation arrives in Slice 7.

#![allow(dead_code)]
