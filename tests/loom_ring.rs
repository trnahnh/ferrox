//! Model-checks the SPSC ring buffer with [loom](https://docs.rs/loom).
//!
//! Loom runs each closure under every interleaving the C11 memory model allows
//! (including stale reads permitted by acquire/release) and checks `UnsafeCell`
//! accesses for data races. These tests only exist when compiled with
//! `--cfg loom`, which swaps the ring's atomics for loom's instrumented ones:
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring
//! ```
//!
//! To see the checker catch a real bug, weaken the cursor ordering to Relaxed
//! (only possible together with `--cfg loom`) and watch it report a data race:
//!
//! ```text
//! RUSTFLAGS="--cfg loom --cfg ring_relaxed_ordering" cargo test --release --test loom_ring
//! ```
#![cfg(loom)]

use ferrox::ring::ring_buffer;
use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::thread;

/// Exhaustive exploration of the spin-wait loops below does not terminate in
/// reasonable time, so the search is bounded to 3 preemptions per execution
/// (the CHESS-style bound loom recommends; most concurrency bugs need <= 2).
/// `LOOM_MAX_PREEMPTIONS=<n>` in the environment overrides it.
fn model<F: Fn() + Sync + Send + 'static>(f: F) {
    let mut builder = loom::model::Builder::new();
    if builder.preemption_bound.is_none() {
        builder.preemption_bound = Some(3);
    }
    builder.check(f);
}

/// One producer pushes `count` sequential values through a ring of `capacity`;
/// the consumer must receive exactly those values in FIFO order.
fn spsc_fifo(capacity: usize, count: u64) {
    model(move || {
        let (mut p, mut c) = ring_buffer::<u64>(capacity);

        let producer = thread::spawn(move || {
            for i in 0..count {
                while p.push(i).is_err() {
                    thread::yield_now();
                }
            }
        });

        let mut next = 0u64;
        while next < count {
            match c.pop() {
                Ok(v) => {
                    assert_eq!(v, next, "FIFO order violated");
                    next += 1;
                }
                Err(_) => thread::yield_now(),
            }
        }

        producer.join().unwrap();
        assert!(c.pop().is_err(), "ring should be empty after draining");
    });
}

#[test]
fn fifo_capacity_1_two_items() {
    // Every push after the first must observe the consumer's release of the slot.
    spsc_fifo(1, 2);
}

#[test]
fn fifo_capacity_2_three_items() {
    // Wraps around once: slot 0 is reused while the consumer may still be reading it.
    spsc_fifo(2, 3);
}

#[test]
fn fifo_capacity_2_five_items() {
    // Multiple wraparounds with sustained backpressure.
    spsc_fifo(2, 5);
}

#[test]
fn unconsumed_items_are_dropped_exactly_once() {
    struct Counted(Arc<AtomicUsize>);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    model(|| {
        let drops = Arc::new(AtomicUsize::new(0));
        let (mut p, mut c) = ring_buffer::<Counted>(2);

        let drops_p = Arc::clone(&drops);
        let producer = thread::spawn(move || {
            p.push(Counted(Arc::clone(&drops_p))).unwrap();
            p.push(Counted(Arc::clone(&drops_p))).unwrap();
            // Producer dropped here; items stay in the ring.
        });

        // Consume at most one (dropped right here as a temporary); whatever is
        // left in the ring must be dropped exactly once by the ring itself.
        let _ = c.pop();
        producer.join().unwrap();
        drop(c);

        assert_eq!(drops.load(Ordering::Relaxed), 2);
    });
}
