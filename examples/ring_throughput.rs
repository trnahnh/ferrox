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
//! Run:  cargo run --release --example ring_throughput -- [iters=20] [count=1000000] [capacity=8192] [flags]
//!
//! Flags:
//!   --pin P,C        pin the producer to CPU P and the consumer (this thread) to CPU C
//!                    (`sched_setaffinity` on Linux, `SetThreadAffinityMask` on Windows)
//!   --raw            print every trial as `raw <workload> <value>` (for scripts/bench_compare.py)
//!   --workload W     u64 | order | latency | all (default all)
//!
//! The `--cfg` knobs in `src/ring.rs` apply here too, e.g.
//!   RUSTFLAGS="--cfg ring_no_padding" cargo run --release --example ring_throughput
use ferrox::order::{Order, Side};
use ferrox::ring::ring_buffer;
use std::hint::spin_loop;
use std::io::Write;
use std::thread;
use std::time::Instant;

mod affinity {
    /// Pins the calling thread to one logical CPU.
    #[cfg(target_os = "linux")]
    pub fn pin_current_thread(cpu: usize) -> Result<(), String> {
        unsafe extern "C" {
            fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u64) -> i32;
        }
        // cpu_set_t is 1024 bits on glibc.
        let mut set = [0u64; 16];
        if cpu >= 1024 {
            return Err(format!("cpu {cpu} out of range"));
        }
        set[cpu / 64] |= 1u64 << (cpu % 64);
        // SAFETY: `set` is a valid, initialized buffer of the size passed; pid 0
        // means the calling thread.
        let rc = unsafe { sched_setaffinity(0, std::mem::size_of_val(&set), set.as_ptr()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!(
                "sched_setaffinity({cpu}) failed: {}",
                std::io::Error::last_os_error()
            ))
        }
    }

    #[cfg(windows)]
    pub fn pin_current_thread(cpu: usize) -> Result<(), String> {
        use core::ffi::c_void;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut c_void;
            fn SetThreadAffinityMask(thread: *mut c_void, mask: usize) -> usize;
        }
        if cpu >= usize::BITS as usize {
            return Err(format!("cpu {cpu} out of range (one processor group only)"));
        }
        // SAFETY: both calls take no pointers from us; the pseudo-handle from
        // GetCurrentThread is always valid for the calling thread.
        let previous = unsafe { SetThreadAffinityMask(GetCurrentThread(), 1usize << cpu) };
        if previous != 0 {
            Ok(())
        } else {
            Err(format!(
                "SetThreadAffinityMask({cpu}) failed: {}",
                std::io::Error::last_os_error()
            ))
        }
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    pub fn pin_current_thread(_cpu: usize) -> Result<(), String> {
        Err("thread pinning is not implemented for this platform".to_string())
    }
}

fn make_order(id: u64) -> Order {
    Order::new(
        id,
        id % 100,
        if id.is_multiple_of(2) {
            Side::Bid
        } else {
            Side::Ask
        },
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

#[derive(Clone, Copy)]
struct Config {
    count: u64,
    capacity: usize,
    producer_cpu: Option<usize>,
}

fn pin_producer(cfg: &Config) {
    if let Some(cpu) = cfg.producer_cpu {
        affinity::pin_current_thread(cpu).expect("pin producer");
    }
}

fn run_u64(cfg: &Config) -> f64 {
    let count = cfg.count;
    let pin = *cfg;
    let t0 = Instant::now();
    let (mut p, mut c) = ring_buffer::<u64>(cfg.capacity);
    let producer = thread::spawn(move || {
        pin_producer(&pin);
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

fn run_order(cfg: &Config) -> f64 {
    let count = cfg.count;
    let pin = *cfg;
    let t0 = Instant::now();
    let (mut p, mut c) = ring_buffer::<Order>(cfg.capacity);
    let producer = thread::spawn(move || {
        pin_producer(&pin);
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

fn print_row(name: &str, s: &Stats, mops: Option<f64>) {
    match mops {
        Some(m) => println!(
            "{:<18} {:>10.3} {:>10.3} {:>10.3} {:>12.1}",
            name, s.min, s.median, s.mean, m
        ),
        None => println!(
            "{:<18} {:>10.3} {:>10.3} {:>10.3} {:>12}",
            name, s.min, s.median, s.mean, "-"
        ),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional = Vec::new();
    let mut pin: Option<(usize, usize)> = None;
    let mut raw = false;
    let mut workload = "all".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--pin" => {
                let spec = args.get(i + 1).expect("--pin needs P,C");
                let (p, c) = spec.split_once(',').expect("--pin P,C");
                pin = Some((p.parse().expect("P"), c.parse().expect("C")));
                i += 2;
            }
            "--raw" => {
                raw = true;
                i += 1;
            }
            "--workload" => {
                workload = args.get(i + 1).expect("--workload needs a name").clone();
                i += 2;
            }
            other => {
                positional.push(other.to_string());
                i += 1;
            }
        }
    }
    let iters: usize = positional
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let count: u64 = positional
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let capacity: usize = positional
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8192);

    if let Some((_, consumer_cpu)) = pin {
        affinity::pin_current_thread(consumer_cpu).expect("pin consumer");
    }
    let cfg = Config {
        count,
        capacity,
        producer_cpu: pin.map(|(p, _)| p),
    };

    let ordering = if cfg!(ring_seqcst) {
        "seq_cst"
    } else if cfg!(ring_relaxed_ordering) {
        "relaxed (UNSOUND)"
    } else {
        "acquire/release"
    };
    println!(
        "rust spsc ring | ordering={ordering} padded={} cursor_cache={} unchecked_index={} | iters={iters} count={count} capacity={capacity} pin={}",
        !cfg!(ring_no_padding) as u8,
        !cfg!(ring_no_cursor_cache) as u8,
        cfg!(ring_unchecked_index) as u8,
        match pin {
            Some((p, c)) => format!("{p},{c}"),
            None => "none".to_string(),
        }
    );

    let want = |w: &str| workload == "all" || workload == w;
    let mut out = std::io::stdout();
    let mut collect = |name: &str, f: &dyn Fn() -> f64| -> Option<Vec<f64>> {
        if !want(name) {
            return None;
        }
        let mut v = Vec::with_capacity(iters);
        for _ in 0..iters {
            let x = f();
            if raw {
                writeln!(out, "raw {name} {x:.6}").unwrap();
                out.flush().unwrap();
            }
            v.push(x);
        }
        Some(v)
    };

    let u64_ms = collect("u64", &|| run_u64(&cfg));
    let order_ms = collect("order", &|| run_order(&cfg));
    let lat_ns = collect("latency", &|| run_latency(count));

    println!(
        "{:<18} {:>10} {:>10} {:>10} {:>12}",
        "workload", "min_ms", "median_ms", "mean_ms", "median_Mops"
    );
    if let Some(v) = u64_ms {
        let s = stats(v);
        let mops = count as f64 / s.median / 1000.0;
        print_row("u64/1M", &s, Some(mops));
    }
    if let Some(v) = order_ms {
        let s = stats(v);
        let mops = count as f64 / s.median / 1000.0;
        print_row("order48/1M", &s, Some(mops));
    }
    if let Some(v) = lat_ns {
        print_row("push_pop_ns/op", &stats(v), None);
    }
}
