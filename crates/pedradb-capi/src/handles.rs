//! Slot + generation table (U2).
//!
//! AS-IS C ABI used `Box::into_raw` / `from_raw`: double-destroy and
//! use-after-destroy are UB. Handles here are packed integers (cast to
//! opaque C pointers). Stale / double-free lookups return `None`.

#![forbid(unsafe_code)]

/// Table index + generation. Generation 0 is never issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    pub slot: u32,
    pub gen: u32,
}

const KIND_SHIFT: u64 = 62;
const GEN_SHIFT: u64 = 31;
const SLOT_MASK: u64 = (1 << 31) - 1;
const GEN_MASK: u64 = (1 << 31) - 1;

/// Discriminator packed into the high bits so a db handle cannot be used as a tx.
pub const KIND_DB: u64 = 1;
pub const KIND_TX: u64 = 2;

/// Next generation that still fits in [`GEN_MASK`] (31 bits). 0 is never
/// issued: `pack` would look like NULL to `unpack`. Wrapping from
/// `GEN_MASK` back to 1 is the inherent ABA of a 31-bit generation (a
/// stale handle from 2³¹−1 reuses ago can alias); the live handle must
/// still round-trip through `pack`/`unpack`.
fn next_gen(g: u32) -> u32 {
    let n = g.wrapping_add(1) & (GEN_MASK as u32);
    if n == 0 {
        1
    } else {
        n
    }
}

/// Pack a handle into a non-zero `u64` (0 is reserved for C NULL).
#[must_use]
pub fn pack(kind: u64, h: Handle) -> u64 {
    debug_assert!(kind == KIND_DB || kind == KIND_TX);
    debug_assert!(h.gen != 0);
    (kind << KIND_SHIFT) | ((u64::from(h.gen) & GEN_MASK) << GEN_SHIFT) | u64::from(h.slot)
}

/// Unpack if `raw` is non-zero and the kind matches.
#[must_use]
pub fn unpack(kind: u64, raw: u64) -> Option<Handle> {
    if raw == 0 {
        return None;
    }
    if raw >> KIND_SHIFT != kind {
        return None;
    }
    let gen = ((raw >> GEN_SHIFT) & GEN_MASK) as u32;
    if gen == 0 {
        return None;
    }
    Some(Handle {
        slot: (raw & SLOT_MASK) as u32,
        gen,
    })
}

struct Slot<T> {
    gen: u32,
    val: Option<T>,
}

/// Generation-checked slot table.
pub struct Table<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

impl<T> Table<T> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    pub fn insert(&mut self, val: T) -> Handle {
        let slot = match self.free.pop() {
            Some(i) => i,
            None => {
                let i = u32::try_from(self.slots.len()).expect("capi handle table full");
                self.slots.push(Slot { gen: 0, val: None });
                i
            }
        };
        let s = &mut self.slots[slot as usize];
        s.gen = next_gen(s.gen);
        s.val = Some(val);
        Handle { slot, gen: s.gen }
    }

    pub fn get(&self, h: Handle) -> Option<&T> {
        let s = self.slots.get(h.slot as usize)?;
        if s.gen != h.gen {
            return None;
        }
        s.val.as_ref()
    }

    pub fn get_mut(&mut self, h: Handle) -> Option<&mut T> {
        let s = self.slots.get_mut(h.slot as usize)?;
        if s.gen != h.gen {
            return None;
        }
        s.val.as_mut()
    }

    pub fn remove(&mut self, h: Handle) -> Option<T> {
        let s = self.slots.get_mut(h.slot as usize)?;
        if s.gen != h.gen {
            return None;
        }
        let v = s.val.take()?;
        self.free.push(h.slot);
        Some(v)
    }

    /// Drop every live value matching `pred` (db-destroy sweeps its txs).
    pub fn drain_if(&mut self, mut pred: impl FnMut(&T) -> bool) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            if s.val.as_ref().is_some_and(&mut pred) {
                s.val.take();
                s.gen = next_gen(s.gen);
                self.free.push(i as u32);
            }
        }
    }
}

/// AS-IS: `Box::into_raw` identity — the same bit pattern is always "valid"
/// to `from_raw`, so double-free is indistinguishable from the first free.
#[cfg(test)]
fn raw_box_second_free_is_alias(first: u64) -> u64 {
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_never_null_and_kind_checked() {
        let h = Handle { slot: 0, gen: 1 };
        let db = pack(KIND_DB, h);
        let tx = pack(KIND_TX, h);
        assert_ne!(db, 0);
        assert_ne!(tx, 0);
        assert_ne!(db, tx);
        assert_eq!(unpack(KIND_DB, db), Some(h));
        assert_eq!(unpack(KIND_TX, db), None);
        assert_eq!(unpack(KIND_DB, 0), None);
    }

    #[test]
    fn insert_get_remove() {
        let mut t = Table::new();
        let h = t.insert(7u8);
        assert_eq!(t.get(h), Some(&7));
        assert_eq!(t.remove(h), Some(7));
        assert_eq!(t.get(h), None);
        assert_eq!(t.remove(h), None);
    }

    #[test]
    fn reuse_slot_rejects_stale() {
        let mut t = Table::new();
        let h1 = t.insert("a");
        assert_eq!(t.remove(h1), Some("a"));
        let h2 = t.insert("b");
        assert_eq!(h1.slot, h2.slot);
        assert_ne!(h1.gen, h2.gen);
        assert_eq!(t.get(h1), None);
        assert_eq!(t.get(h2), Some(&"b"));
    }

    #[test]
    fn as_is_box_alias_equals_first() {
        // Contrasts with unique gens: the same integer is a second free.
        assert_eq!(raw_box_second_free_is_alias(0x100), 0x100);
        let mut t = Table::new();
        let h = t.insert(1u8);
        t.remove(h);
        assert_ne!(
            pack(KIND_DB, h),
            pack(
                KIND_DB,
                Handle {
                    slot: h.slot,
                    gen: h.gen.wrapping_add(1)
                }
            )
        );
    }

    #[test]
    fn drain_if_invalidates() {
        let mut t = Table::new();
        let keep = t.insert(1u8);
        let drop = t.insert(2u8);
        t.drain_if(|v| *v == 2);
        assert_eq!(t.get(keep), Some(&1));
        assert_eq!(t.get(drop), None);
    }

    #[test]
    fn gen_stays_inside_pack_mask() {
        let mut t = Table::new();
        let h0 = t.insert(0u8);
        assert_eq!(unpack(KIND_DB, pack(KIND_DB, h0)), Some(h0));
        t.remove(h0);
        // AS-IS u32 wrapping_add at 2^31: gen=0x8000_0000, packed gen=0,
        // unpack returns None for a *live* handle (F209).
        t.slots[h0.slot as usize].gen = super::GEN_MASK as u32;
        let h1 = t.insert(1u8);
        assert_eq!(h1.gen, 1);
        assert_eq!(unpack(KIND_DB, pack(KIND_DB, h1)), Some(h1));
        assert_eq!(t.get(h1), Some(&1));
        // Inherent 31-bit ABA: first gen on this slot was also 1.
        assert_eq!(h0.gen, 1);
        assert_eq!(t.get(h0), Some(&1));
    }

    #[test]
    fn as_is_u32_gen_at_2pow31_packs_to_null() {
        let h = Handle {
            slot: 0,
            gen: 1u32 << 31,
        };
        assert_eq!(unpack(KIND_DB, pack(KIND_DB, h)), None);
        let h_ok = Handle {
            slot: 0,
            gen: super::GEN_MASK as u32,
        };
        assert_eq!(unpack(KIND_DB, pack(KIND_DB, h_ok)), Some(h_ok));
    }

    #[test]
    fn fuzz_stale_never_aliases_live() {
        let mut t = Table::new();
        let mut rng = 0xC0FF_EE00_u64;
        let mut live: Vec<(Handle, u32)> = Vec::new();
        let mut stale: Vec<Handle> = Vec::new();
        for i in 0..8_000u32 {
            rng = rng.wrapping_mul(0x5851_F42D_4C95_7F2D).wrapping_add(1);
            match rng % 5 {
                0 => {
                    let h = t.insert(i);
                    live.push((h, i));
                }
                1 if !live.is_empty() => {
                    let idx = (rng as usize) % live.len();
                    let (h, v) = live.swap_remove(idx);
                    assert_eq!(t.remove(h), Some(v));
                    stale.push(h);
                }
                2 if !live.is_empty() => {
                    let idx = (rng as usize) % live.len();
                    let (h, v) = live[idx];
                    assert_eq!(t.get(h), Some(&v));
                    assert_eq!(t.get_mut(h).copied(), Some(v));
                }
                _ => {
                    if let Some(&h) = stale.last() {
                        assert_eq!(t.get(h), None);
                        assert_eq!(t.remove(h), None);
                    }
                    if rng % 17 == 0 {
                        let packed = pack(KIND_DB, Handle { slot: 3, gen: 9 });
                        assert_eq!(unpack(KIND_TX, packed), None);
                        assert_eq!(unpack(KIND_DB, 0), None);
                    }
                }
            }
        }
        for (h, v) in &live {
            assert_eq!(t.get(*h), Some(v));
        }
        for h in &stale {
            assert_eq!(t.get(*h), None);
        }
    }
}
