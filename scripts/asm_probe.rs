//! Standalone probe that exposes the ring's push/pop as out-of-line functions so
//! their x86-64 code can be inspected. Compiled directly with `rustc` by
//! `scripts/asm_ordering.sh`; it pulls in the real `src/ring.rs` via `#[path]`.
#[path = "../src/ring.rs"]
#[allow(dead_code)]
mod ring;

use ring::{Consumer, Producer, ring_buffer};

#[inline(never)]
pub fn push_u64(p: &mut Producer<u64>, v: u64) -> bool {
    p.push(v).is_ok()
}

#[inline(never)]
pub fn pop_u64(c: &mut Consumer<u64>) -> u64 {
    c.pop().unwrap_or(0)
}

fn main() {
    let (mut p, mut c) = ring_buffer::<u64>(1024);
    let v = std::env::args().count() as u64;
    std::hint::black_box(push_u64(&mut p, v));
    std::hint::black_box(pop_u64(&mut c));
}
