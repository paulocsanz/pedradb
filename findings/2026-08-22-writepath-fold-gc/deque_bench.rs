use std::collections::VecDeque;
use std::time::Instant;

#[derive(Clone)]
struct V {
    seq: u64,
    key: Vec<u8>,
    val: Vec<u8>,
}

fn main() {
    let n = 20_000usize;
    // insert(0) on a pre-filled deque (the fold-merge shape)
    let mut d: VecDeque<V> = (1..=n).map(|i| V { seq: i as u64, key: vec![0u8; 8], val: vec![0u8; 16] }).collect();
    let t = Instant::now();
    for i in 0..n {
        d.insert(0, V { seq: 100_000 + i as u64, key: vec![0u8; 8], val: vec![0u8; 16] });
    }
    println!("insert(0) x{n} on len {n}: {:?}", t.elapsed());

    // push_front for comparison
    let mut d2: VecDeque<V> = (1..=n).map(|i| V { seq: i as u64, key: vec![0u8; 8], val: vec![0u8; 16] }).collect();
    let t = Instant::now();
    for i in 0..n {
        d2.push_front(V { seq: 100_000 + i as u64, key: vec![0u8; 8], val: vec![0u8; 16] });
    }
    println!("push_front x{n} on len {n}: {:?}", t.elapsed());

    // binary search over indices (like insert_into)
    let d3: VecDeque<V> = (1..=40_000).map(|i| V { seq: i as u64, key: vec![0u8; 8], val: vec![0u8; 16] }).collect();
    let t = Instant::now();
    let mut acc = 0usize;
    for i in 0..40_000u64 {
        let (mut lo, mut hi) = (0usize, d3.len());
        let target = 100_000 + i;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if d3[mid].seq > target { lo = mid + 1 } else { hi = mid }
        }
        acc += lo;
    }
    println!("binsearch x40k: {:?} acc={acc}", t.elapsed());
}
