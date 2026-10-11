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
    std::hint::black_box(latency_loop(&mut p, &mut c, v));
    std::hint::black_box(latency_loop_nounwrap(&mut p, &mut c, v));
    std::hint::black_box(latency_bench(v));
    std::hint::black_box(latency_bench_leak(v));
}

/// The single-thread push+pop loop of `examples/ring_throughput.rs::run_latency`,
/// out of line so the whole loop body can be read as one block
/// (`scripts/asm_loop.sh`).
#[inline(never)]
pub fn latency_loop(p: &mut Producer<u64>, c: &mut Consumer<u64>, count: u64) -> u64 {
    let mut sum = 0u64;
    for i in 0..count {
        p.push(i).unwrap();
        sum = sum.wrapping_add(c.pop().unwrap());
    }
    sum
}

/// Same loop without the `unwrap` panic paths: a full ring or an empty pop is
/// impossible here, so this isolates what the two cold branches cost.
#[inline(never)]
pub fn latency_loop_nounwrap(p: &mut Producer<u64>, c: &mut Consumer<u64>, count: u64) -> u64 {
    let mut sum = 0u64;
    for i in 0..count {
        let _ = p.push(i);
        if let Ok(v) = c.pop() {
            sum = sum.wrapping_add(v);
        }
    }
    sum
}

/// Exactly `run_latency` from the harness: the ring is created inside, so the
/// producer/consumer are locals whose fields the compiler may keep in
/// registers (the `latency_loop` variants above force them through memory
/// because they are passed by reference).
#[inline(never)]
pub fn latency_bench(count: u64) -> u64 {
    let (mut p, mut c) = ring_buffer::<u64>(1024);
    let mut sum = 0u64;
    for i in 0..count {
        p.push(i).unwrap();
        sum = sum.wrapping_add(c.pop().unwrap());
    }
    sum
}

/// Diagnostic twin of `latency_bench` that leaks the ring instead of dropping
/// it: if the loop below keeps the cursors in registers while `latency_bench`
/// spills them, the escape that blocks promotion is the `Arc` drop.
#[inline(never)]
pub fn latency_bench_leak(count: u64) -> u64 {
    let (mut p, mut c) = ring_buffer::<u64>(1024);
    let mut sum = 0u64;
    for i in 0..count {
        p.push(i).unwrap();
        sum = sum.wrapping_add(c.pop().unwrap());
    }
    std::mem::forget(p);
    std::mem::forget(c);
    sum
}
