//! Load generator: proves the kill metric (per-order latency distribution and
//! sustained throughput) on the ring-pop -> match -> encode hot path.
//!
//! Direct mode (default) pre-loads 1M resting asks across 50 price levels, pushes
//! 1M bids at the same 50 levels into the ring, then drains the ring through the
//! engine on the calling thread. Because bids sweep from the lowest ask level,
//! roughly 63% of them fill and the rest rest in the book; both outcomes are
//! measured (the "Fills produced" / "Orders resting" lines say how many of each).
//!
//! # Clocks (`--features metrics`)
//!
//! Per-order latency is a few hundred nanoseconds, so the clock matters:
//!
//! - default: the x86-64 timestamp counter (`rdtsc`), calibrated against
//!   `Instant` at startup. Sub-nanosecond resolution, ~10 ns read cost.
//! - `--instant`: `std::time::Instant`. On Windows this is
//!   QueryPerformanceCounter and advances in 100 ns steps, so every percentile it
//!   reports is quantized to a multiple of 100 ns. The observed step size is
//!   measured and printed so the quantization is visible, not hidden.
//!
//! ```text
//! cargo run --release --example loadgen --features metrics             # TSC clock
//! cargo run --release --example loadgen --features metrics -- --instant
//! cargo run --release --example loadgen                                # throughput only, no timing overhead
//! cargo run --release --example loadgen --features metrics,alloc-track # + heap allocation count
//! cargo run --release --example loadgen -- --tcp                       # full TCP -> ring -> UDP pipeline
//! ```
#[cfg(feature = "alloc-track")]
use ferrox::alloc_track::TrackingAllocator;

#[cfg(feature = "alloc-track")]
#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

use ferrox::matching::MatchingEngine;
use ferrox::order::{Order, Side};
use ferrox::protocol::{EXECUTION_REPORT_SIZE, EngineCommand, encode_execution_report};
use ferrox::ring;

#[cfg(feature = "metrics")]
use ferrox::metrics::LatencyRecorder;

const ORDER_COUNT: usize = 1_000_000;
const WARMUP_COUNT: usize = 10_000;
const RING_CAPACITY: usize = 1 << 20;
const ARENA_CAPACITY: u32 = 1 << 20;
const PRICE_LEVELS: usize = 50;

fn make_order(id: u64, side: Side, price: i64, qty: u64) -> Order {
    Order::new(id, id, side, price, qty, id).unwrap()
}

#[cfg(feature = "metrics")]
mod clock {
    use std::time::{Duration, Instant};

    /// Source of per-order timestamps. `now()` returns raw ticks; convert with
    /// `ticks_to_ns` only when printing so the hot loop does no arithmetic
    /// beyond one subtraction.
    pub enum Clock {
        Instant {
            epoch: Instant,
        },
        #[cfg(target_arch = "x86_64")]
        Tsc {
            ticks_per_ns: f64,
        },
    }

    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn rdtsc() -> u64 {
        // SAFETY: `rdtsc` has no preconditions on x86-64; it only reads the
        // timestamp counter.
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    impl Clock {
        pub fn instant() -> Self {
            Clock::Instant {
                epoch: Instant::now(),
            }
        }

        /// Calibrates the TSC against `Instant` over a 200 ms window. Modern
        /// x86-64 CPUs have an invariant TSC (constant rate regardless of
        /// frequency scaling), so one calibration is valid for the whole run.
        #[cfg(target_arch = "x86_64")]
        pub fn tsc() -> Self {
            let t0 = Instant::now();
            let c0 = rdtsc();
            while t0.elapsed() < Duration::from_millis(200) {
                std::hint::spin_loop();
            }
            let c1 = rdtsc();
            let elapsed = t0.elapsed();
            Clock::Tsc {
                ticks_per_ns: (c1 - c0) as f64 / elapsed.as_nanos() as f64,
            }
        }

        #[inline]
        pub fn now(&self) -> u64 {
            match self {
                Clock::Instant { epoch } => epoch.elapsed().as_nanos() as u64,
                #[cfg(target_arch = "x86_64")]
                Clock::Tsc { .. } => rdtsc(),
            }
        }

        pub fn ticks_to_ns(&self, ticks: f64) -> f64 {
            match self {
                Clock::Instant { .. } => ticks,
                #[cfg(target_arch = "x86_64")]
                Clock::Tsc { ticks_per_ns } => ticks / ticks_per_ns,
            }
        }

        pub fn describe(&self) -> String {
            match self {
                Clock::Instant { .. } => "std::time::Instant".to_string(),
                #[cfg(target_arch = "x86_64")]
                Clock::Tsc { ticks_per_ns } => {
                    format!("x86-64 TSC, calibrated {:.4} GHz", ticks_per_ns)
                }
            }
        }

        /// Smallest non-zero step between consecutive reads, in ns, over 1M
        /// reads. For a coarse clock this is its tick (100 ns for QPC on
        /// Windows); for the TSC it is the read cost itself.
        pub fn observed_resolution_ns(&self) -> f64 {
            let mut smallest = u64::MAX;
            let mut prev = self.now();
            for _ in 0..1_000_000 {
                let now = self.now();
                let d = now.wrapping_sub(prev);
                if d > 0 && d < smallest {
                    smallest = d;
                }
                prev = now;
            }
            self.ticks_to_ns(smallest as f64)
        }
    }

    pub fn print_latency(recorder: &super::LatencyRecorder, clock: &Clock) {
        println!("Latency ({}):", clock.describe());
        println!(
            "  clock step : {:>10.1} ns (smallest observed delta between consecutive reads)",
            clock.observed_resolution_ns()
        );
        let rows = [
            ("P50", recorder.p50()),
            ("P90", recorder.p90()),
            ("P99", recorder.p99()),
            ("P99.9", recorder.p999()),
            ("Max", recorder.max()),
        ];
        for (name, ticks) in rows {
            println!(
                "  {name:<6}     : {:>10.0} ns",
                clock.ticks_to_ns(ticks as f64)
            );
        }
        println!(
            "  Mean       : {:>10.0} ns",
            clock.ticks_to_ns(recorder.mean())
        );
        println!("  Count      : {:>10}", recorder.count());
    }
}

fn run_direct(use_instant_clock: bool) {
    println!("Ferrox Load Generator — Direct Mode");
    println!("  Orders:  {ORDER_COUNT}");
    println!("  Warmup:  {WARMUP_COUNT}");
    println!("  Levels:  {PRICE_LEVELS}");
    println!();

    #[cfg(not(feature = "metrics"))]
    let _ = use_instant_clock;

    #[cfg(feature = "metrics")]
    let clock = {
        #[cfg(target_arch = "x86_64")]
        let c = if use_instant_clock {
            clock::Clock::instant()
        } else {
            clock::Clock::tsc()
        };
        #[cfg(not(target_arch = "x86_64"))]
        let c = clock::Clock::instant();
        c
    };

    let (mut producer, mut consumer) = ring::ring_buffer::<EngineCommand>(RING_CAPACITY);
    let mut engine = MatchingEngine::with_capacity(ARENA_CAPACITY);

    let total_pairs = ORDER_COUNT;
    let mut next_id = 1u64;

    for i in 0..total_pairs {
        let price = 10000 + (i % PRICE_LEVELS) as i64;
        engine
            .add_order(make_order(next_id, Side::Ask, price, 10))
            .unwrap();
        next_id += 1;
    }

    for i in 0..total_pairs {
        let price = 10000 + (i % PRICE_LEVELS) as i64;
        let mut cmd = EngineCommand::NewOrder(make_order(next_id, Side::Bid, price, 10));
        loop {
            match producer.push(cmd) {
                Ok(()) => break,
                Err(ring::Full(returned)) => {
                    cmd = returned;
                    std::thread::yield_now();
                }
            }
        }
        next_id += 1;
    }

    drop(producer);

    let mut report_buf = [0u8; EXECUTION_REPORT_SIZE];
    let mut seq = 0u32;
    let mut warmup_done = 0;

    while warmup_done < WARMUP_COUNT {
        match consumer.pop() {
            Ok(cmd) => {
                match cmd {
                    EngineCommand::NewOrder(order) => {
                        let ts = order.timestamp;
                        if let Ok(result) = engine.add_order(order) {
                            for fill in result.fills {
                                seq += 1;
                                let _ = encode_execution_report(&mut report_buf, seq, fill, ts);
                            }
                        }
                    }
                    EngineCommand::CancelOrder { order_id } => {
                        let _ = engine.cancel_order(order_id);
                    }
                }
                warmup_done += 1;
            }
            Err(_) => break,
        }
    }

    println!("Warmup complete ({warmup_done} orders)");

    #[cfg(feature = "alloc-track")]
    ferrox::alloc_track::start_tracking();

    #[cfg(feature = "metrics")]
    let mut recorder = LatencyRecorder::new();

    let measured_start = std::time::Instant::now();
    let mut measured_count = 0u64;
    let mut fill_count = 0u64;

    while let Ok(cmd) = consumer.pop() {
        #[cfg(feature = "metrics")]
        let order_start = clock.now();

        match cmd {
            EngineCommand::NewOrder(order) => {
                let ts = order.timestamp;
                if let Ok(result) = engine.add_order(order) {
                    for fill in result.fills {
                        seq += 1;
                        let _ = encode_execution_report(&mut report_buf, seq, fill, ts);
                        fill_count += 1;
                    }
                }
            }
            EngineCommand::CancelOrder { order_id } => {
                let _ = engine.cancel_order(order_id);
            }
        }

        #[cfg(feature = "metrics")]
        recorder.record_nanos(clock.now().wrapping_sub(order_start));

        measured_count += 1;
    }

    let elapsed = measured_start.elapsed();

    #[cfg(feature = "alloc-track")]
    let alloc_stats = ferrox::alloc_track::stop_tracking();

    println!();
    println!("=== Results ===");
    println!("  Measured orders : {measured_count}");
    println!("  Fills produced  : {fill_count}");
    println!(
        "  Orders resting  : {}",
        measured_count.saturating_sub(fill_count)
    );
    println!(
        "  Elapsed         : {:.3} ms",
        elapsed.as_secs_f64() * 1000.0
    );

    if elapsed.as_nanos() > 0 {
        let throughput = measured_count as f64 / elapsed.as_secs_f64();
        println!("  Throughput      : {throughput:.0} orders/sec");
        #[cfg(feature = "metrics")]
        println!("  (throughput includes two clock reads per order for the histogram)");
    }

    #[cfg(feature = "metrics")]
    {
        println!();
        clock::print_latency(&recorder, &clock);
    }

    #[cfg(feature = "alloc-track")]
    {
        println!();
        println!("Allocation tracking (measured phase):");
        println!(
            "  allocs: {}, deallocs: {}, bytes: {}",
            alloc_stats.alloc_count, alloc_stats.dealloc_count, alloc_stats.alloc_bytes
        );
        println!(
            "  (expected source: BTreeMap price-level node alloc/dealloc; the arena and fills buffer never allocate)"
        );
    }
}

fn run_tcp() {
    use ferrox::gateway::{GatewayConfig, run};
    use ferrox::protocol::{NEW_ORDER_SIZE, encode_new_order};
    use std::io::Write;
    use std::net::{TcpStream, UdpSocket};
    use std::time::Duration;

    println!("Ferrox Load Generator — TCP Pipeline Mode");
    println!("  Orders: {ORDER_COUNT}");
    println!();

    let tcp_addr = "127.0.0.1:19000".parse().unwrap();
    let udp_addr = "127.0.0.1:19001".parse().unwrap();

    let config = GatewayConfig {
        listen_addr: tcp_addr,
        multicast_addr: udp_addr,
        ring_capacity: RING_CAPACITY,
        arena_capacity: ARENA_CAPACITY,
        data_dir: None,
        snapshot_interval: 100_000,
    };

    let gateway_handle = std::thread::spawn(move || {
        let _ = run(config);
    });

    std::thread::sleep(Duration::from_millis(100));

    let udp_recv = UdpSocket::bind(udp_addr).unwrap();
    udp_recv
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();

    let total_pairs = ORDER_COUNT;

    let send_start = std::time::Instant::now();
    {
        let mut stream = TcpStream::connect(tcp_addr).unwrap();
        let mut buf = [0u8; NEW_ORDER_SIZE];
        let mut next_id = 1u64;

        for i in 0..total_pairs {
            let price = 10000 + (i % PRICE_LEVELS) as i64;
            let order = make_order(next_id, Side::Ask, price, 10);
            encode_new_order(&mut buf, &order).unwrap();
            stream.write_all(&buf).unwrap();
            next_id += 1;
        }

        for i in 0..total_pairs {
            let price = 10000 + (i % PRICE_LEVELS) as i64;
            let order = make_order(next_id, Side::Bid, price, 10);
            encode_new_order(&mut buf, &order).unwrap();
            stream.write_all(&buf).unwrap();
            next_id += 1;
        }
    }

    let send_elapsed = send_start.elapsed();

    let e2e_start = std::time::Instant::now();

    let mut recv_count = 0u64;
    let mut recv_buf = [0u8; EXECUTION_REPORT_SIZE];
    loop {
        match udp_recv.recv_from(&mut recv_buf) {
            Ok(_) => recv_count += 1,
            Err(_) => break,
        }
    }

    let e2e_elapsed = e2e_start.elapsed();

    let _ = gateway_handle.join();

    println!("=== Results ===");
    println!("  Orders sent     : {}", total_pairs * 2);
    println!("  Reports received: {recv_count}");
    println!(
        "  Send elapsed    : {:.3} ms",
        send_elapsed.as_secs_f64() * 1000.0
    );
    if send_elapsed.as_nanos() > 0 {
        let throughput = (total_pairs * 2) as f64 / send_elapsed.as_secs_f64();
        println!("  Send throughput : {throughput:.0} orders/sec");
    }
    if recv_count > 0 {
        println!(
            "  Recv elapsed    : {:.3} ms",
            e2e_elapsed.as_secs_f64() * 1000.0
        );
        let avg_ns = e2e_elapsed.as_nanos() as f64 / recv_count as f64;
        println!("  Avg per report  : {:.0} ns", avg_ns);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tcp_mode = args.iter().any(|a| a == "--tcp");
    let use_instant_clock = args.iter().any(|a| a == "--instant");

    if tcp_mode {
        run_tcp();
    } else {
        run_direct(use_instant_clock);
    }
}
