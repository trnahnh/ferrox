#[cfg(feature = "alloc-track")]
use ferrox::alloc_track::TrackingAllocator;

#[cfg(feature = "alloc-track")]
#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

use ferrox::matching::MatchingEngine;
use ferrox::order::{Order, Side};
use ferrox::protocol::{
    EngineCommand, EXECUTION_REPORT_SIZE, encode_execution_report,
};
use ferrox::ring;

#[cfg(feature = "metrics")]
use ferrox::metrics::LatencyRecorder;

const ORDER_COUNT: usize = 1_000_000;
const WARMUP_COUNT: usize = 10_000;
const RING_CAPACITY: usize = 1 << 20;
const ARENA_CAPACITY: u32 = 1 << 20;

fn make_order(id: u64, side: Side, price: i64, qty: u64) -> Order {
    Order::new(id, id, side, price, qty, id).unwrap()
}

fn run_direct() {
    println!("Ferrox Load Generator — Direct Mode");
    println!("  Orders:  {ORDER_COUNT}");
    println!("  Warmup:  {WARMUP_COUNT}");
    println!();

    let (mut producer, mut consumer) =
        ring::ring_buffer::<EngineCommand>(RING_CAPACITY);
    let mut engine = MatchingEngine::with_capacity(ARENA_CAPACITY);

    let total_pairs = ORDER_COUNT;
    let mut next_id = 1u64;

    for i in 0..total_pairs {
        let price = 10000 + (i % 50) as i64;
        engine
            .add_order(make_order(next_id, Side::Ask, price, 10))
            .unwrap();
        next_id += 1;
    }

    for i in 0..total_pairs {
        let price = 10000 + (i % 50) as i64;
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
                                let _ = encode_execution_report(
                                    &mut report_buf,
                                    seq,
                                    fill,
                                    ts,
                                );
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
        let order_start = LatencyRecorder::start();

        match cmd {
            EngineCommand::NewOrder(order) => {
                let ts = order.timestamp;
                if let Ok(result) = engine.add_order(order) {
                    for fill in result.fills {
                        seq += 1;
                        let _ = encode_execution_report(
                            &mut report_buf,
                            seq,
                            fill,
                            ts,
                        );
                        fill_count += 1;
                    }
                }
            }
            EngineCommand::CancelOrder { order_id } => {
                let _ = engine.cancel_order(order_id);
            }
        }

        #[cfg(feature = "metrics")]
        recorder.record(order_start);

        measured_count += 1;
    }

    let elapsed = measured_start.elapsed();

    #[cfg(feature = "alloc-track")]
    let alloc_stats = ferrox::alloc_track::stop_tracking();

    println!();
    println!("=== Results ===");
    println!("  Measured orders : {measured_count}");
    println!("  Fills produced  : {fill_count}");
    println!("  Elapsed         : {:.3} ms", elapsed.as_secs_f64() * 1000.0);

    if elapsed.as_nanos() > 0 {
        let throughput = measured_count as f64 / elapsed.as_secs_f64();
        println!("  Throughput      : {throughput:.0} orders/sec");
    }

    #[cfg(feature = "metrics")]
    {
        println!();
        recorder.print_summary();
    }

    #[cfg(feature = "alloc-track")]
    {
        println!();
        println!("Allocation tracking (measured phase):");
        println!("  allocs: {}, deallocs: {}, bytes: {}",
            alloc_stats.alloc_count, alloc_stats.dealloc_count, alloc_stats.alloc_bytes);
        println!("  {} allocations from BTreeMap level management (no order/fill heap allocs)",
            alloc_stats.alloc_count);
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
    udp_recv.set_read_timeout(Some(Duration::from_secs(10))).unwrap();

    let total_pairs = ORDER_COUNT;

    let send_start = std::time::Instant::now();
    {
        let mut stream = TcpStream::connect(tcp_addr).unwrap();
        let mut buf = [0u8; NEW_ORDER_SIZE];
        let mut next_id = 1u64;

        for i in 0..total_pairs {
            let price = 10000 + (i % 50) as i64;
            let order = make_order(next_id, Side::Ask, price, 10);
            encode_new_order(&mut buf, &order).unwrap();
            stream.write_all(&buf).unwrap();
            next_id += 1;
        }

        for i in 0..total_pairs {
            let price = 10000 + (i % 50) as i64;
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
    println!("  Send elapsed    : {:.3} ms", send_elapsed.as_secs_f64() * 1000.0);
    if send_elapsed.as_nanos() > 0 {
        let throughput = (total_pairs * 2) as f64 / send_elapsed.as_secs_f64();
        println!("  Send throughput : {throughput:.0} orders/sec");
    }
    if recv_count > 0 {
        println!("  Recv elapsed    : {:.3} ms", e2e_elapsed.as_secs_f64() * 1000.0);
        let avg_ns = e2e_elapsed.as_nanos() as f64 / recv_count as f64;
        println!("  Avg per report  : {:.0} ns", avg_ns);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tcp_mode = args.iter().any(|a| a == "--tcp");

    if tcp_mode {
        run_tcp();
    } else {
        run_direct();
    }
}
