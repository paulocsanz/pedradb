//! Pedra Aeneas fork `pedra-dyn-struct`: type-decl of `Box<dyn Iterator>`
//! extracts. Vtable `next` (`dyn_holder_pull`) is still FnOpDynamic.

/// REPRO: struct holding `Box<dyn Iterator>` — now a Lean `structure`.
pub struct DynHolder {
    pub stream: Box<dyn Iterator<Item = u8>>,
    pub last: Option<u8>,
}

pub fn dyn_holder_last(h: &DynHolder) -> Option<u8> {
    h.last
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
