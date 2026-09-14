//! RFC-0188 P2.4 — minimal upstream repro: Aeneas refuses `dyn`-Trait
//! types at the TYPE-DECL level ("Dynamic trait types are not supported
//! yet"). Distilled from the production `StreamingVisibleIter` shape in
//! `crates/pedradb-core/src/merge_kernel.rs`: a struct holding a
//! `Box<dyn Iterator>` field (the fire 803 measurement showed the
//! refusal fires on the TYPE DECL translation, before any body).
//!
//! The control type below is the same struct and the same pull fn over
//! a concrete slice — same crate, same pin, no `dyn` — and extracts
//! cleanly, isolating the refusal to the dynamic trait type alone.

/// REPRO: struct holding `Box<dyn Iterator>` — type-decl translation
/// refuses with "Dynamic trait types are not supported yet".
pub struct DynHolder {
    pub stream: Box<dyn Iterator<Item = u8>>,
    pub last: Option<u8>,
}

pub fn dyn_holder_pull(h: &mut DynHolder) -> Option<u8> {
    h.last = h.stream.next();
    h.last
}

/// CONTROL: same shape, concrete field — extracts.
pub struct ConcreteHolder {
    pub items: Vec<u8>,
    pub last: Option<u8>,
}

pub fn concrete_holder_pull(h: &mut ConcreteHolder) -> Option<u8> {
    h.last = h.items.pop();
    h.last
}
