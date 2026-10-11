//! Hand-rolled ring-buffer throughput / latency harness.
//!
//! Methodology is identical to `cpp/bench_ring.cpp` so the Rust and C++ rings can
//! be compared like-for-like (criterion is not used here on purpose: the C++ side
//! has no criterion, so both sides use the same plain loop and the same stats):
//!   - u64 and 48-byte `Order` payloads, 1M items per iteration
//!   - one producer thread, consumer on the calling thread, spin-wait with
//!     `std::hint::spin_loop()`
//!   - thread spawn and join are inside the timed region (same as
//!     `benches/ring_bench.rs`)
//!   - N iterations; min / median / mean reported
//!
//! Run:  cargo run --release --example ring_throughput -- [iters=20] [count=1000000] [capacity=8192]
//!
//! The `--cfg` knobs in `src/ring.rs` apply here too, e.g.
//!   RUSTFLAGS="--cfg ring_no_padding" cargo run --release --example ring_throughput
use ferrox::order::{Order, Side};
use ferrox::ring::ring_buffer;
use std::hint::spin_loop;
use std::thread;
use std::time::Instant;

fn make_order(id: u64) -> Order {
    Order::new(
        id,
        id % 100,
        if id.is_multiple_of(2) { Side::Bid } else { Side::Ask },
        10_000 + (id as i64 % 500),
        (id % 1000) + 1,
        id * 1000,
    )
    .unwrap()
}

struct Stats {
    min: f64,
    median: f64,
    mean: f64,
}

fn stats(mut v: Vec<f64>) -> Stats {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let n = v.len();
    let median = if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    };
    Stats {
        min: v[0],
        median,
        mean,
    }
}

fn run_u64(count: u64, capacity: usize) -> f64 {
    let t0 = Instant::now();
    let (mut p, mut c) = ring_buffer::<u64>(capacity);
    let producer = thread::spawn(move || {
        for i in 0..count {
            while p.push(i).is_err() {
                spin_loop();
            }
        }
    });
    let mut sum = 0u64;
    for _ in 0..count {
        loop {
            if let Ok(v) = c.pop() {
                sum = sum.wrapping_add(v);
                break;
            }
            spin_loop();
        }
    }
    producer.join().unwrap();
    let elapsed = t0.elapsed();
    assert_eq!(sum, count * (count - 1) / 2, "checksum mismatch");
    elapsed.as_secs_f64() * 1e3
}

fn run_order(count: u64, capacity: usize) -> f64 {
    let t0 = Instant::now();
    let (mut p, mut c) = ring_buffer::<Order>(capacity);
    let producer = thread::spawn(move || {
        for i in 0..count {
            let order = make_order(i);
            while p.push(order.clone()).is_err() {
                spin_loop();
            }
        }
    });
    let mut sum = 0u64;
    for _ in 0..count {
        loop {
            if let Ok(o) = c.pop() {
                sum = sum.wrapping_add(o.id);
                break;
            }
            spin_loop();
        }
    }
    producer.join().unwrap();
    let elapsed = t0.elapsed();
    assert_eq!(sum, count * (count - 1) / 2, "checksum mismatch");
    elapsed.as_secs_f64() * 1e3
}

/// Single thread, push one / pop one, ns per push+pop pair.
fn run_latency(count: u64) -> f64 {
    let (mut p, mut c) = ring_buffer::<u64>(1024);
    let mut sum = 0u64;
    let t0 = Instant::now();
    for i in 0..count {
        p.push(i).unwrap();
        sum = sum.wrapping_add(c.pop().unwrap());
    }
    let elapsed = t0.elapsed();
    assert_eq!(sum, count * (count - 1) / 2);
    elapsed.as_nanos() as f64 / count as f64
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let iters: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let count: u64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let capacity: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8192);

    let ordering = if cfg!(ring_seqcst) {
        "seq_cst"
    } else if cfg!(ring_relaxed_ordering) {
        "relaxed (UNSOUND)"
    } else {
        "acquire/release"
    };
    println!(
        "rust spsc ring | ordering={ordering} padded={} cursor_cache={} | iters={iters} count={count} capacity={capacity}",
        !cfg!(ring_no_padding) as u8,
        !cfg!(ring_no_cursor_cache) as u8,
    );
    println!(
        "{:<18} {:>10} {:>10} {:>10} {:>12}",
        "workload", "min_ms", "median_ms", "mean_ms", "median_Mops"
    );

    let u64_ms: Vec<f64> = (0..iters).map(|_| run_u64(count, capacity)).collect();
    let order_ms: Vec<f64> = (0..iters).map(|_| run_order(count, capacity)).collect();
    let lat_ns: Vec<f64> = (0..iters).map(|_| run_latency(count)).collect();

    let (u, o, l) = (stats(u64_ms), stats(order_ms), stats(lat_ns));
    println!(
        "{:<18} {:>10.3} {:>10.3} {:>10.3} {:>12.1}",
        "u64/1M",
        u.min,
        u.median,
        u.mean,
        count as f64 / u.median / 1000.0
    );
    println!(
        "{:<18} {:>10.3} {:>10.3} {:>10.3} {:>12.1}",
        "order48/1M",
        o.min,
        o.median,
        o.mean,
        count as f64 / o.median / 1000.0
    );
    println!(
        "{:<18} {:>10.3} {:>10.3} {:>10.3} {:>12}",
        "push_pop_ns/op", l.min, l.median, l.mean, "-"
    );
}
