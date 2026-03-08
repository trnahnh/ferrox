use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use ferrox::matching::MatchingEngine;
use ferrox::order::{Order, Side};
use ferrox::protocol::{
    EngineCommand, EXECUTION_REPORT_SIZE, encode_execution_report,
};
use ferrox::ring;

fn make_order(id: u64, trader_id: u64, side: Side, price: i64, qty: u64) -> Order {
    Order::new(id, trader_id, side, price, qty, id).unwrap()
}

fn bench_e2e_crossing(c: &mut Criterion) {
    let mut group = c.benchmark_group("e2e/crossing_orders");

    for &n in &[1_000usize, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let (mut producer, consumer) =
                        ring::ring_buffer::<EngineCommand>(n.next_power_of_two() * 2);
                    let mut engine = MatchingEngine::with_capacity(n as u32 + 16);

                    for i in 1..=n as u64 {
                        engine
                            .add_order(make_order(i, i, Side::Ask, 100, 10))
                            .unwrap();
                    }

                    for i in 1..=n as u64 {
                        let cmd = EngineCommand::NewOrder(make_order(
                            n as u64 + i,
                            n as u64 + i,
                            Side::Bid,
                            100,
                            10,
                        ));
                        producer.push(cmd).unwrap();
                    }

                    (consumer, engine)
                },
                |(mut consumer, mut engine)| {
                    let mut report_buf = [0u8; EXECUTION_REPORT_SIZE];
                    let mut seq = 0u32;
                    while let Ok(cmd) = consumer.pop() {
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
                    }
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn bench_e2e_mixed(c: &mut Criterion) {
    c.bench_function("e2e/mixed_100k", |b| {
        b.iter_batched(
            || {
                let cap = 131_072;
                let (mut producer, consumer) =
                    ring::ring_buffer::<EngineCommand>(cap);
                let engine = MatchingEngine::with_capacity(65_536);

                let mut next_id = 1u64;
                let mut resting: Vec<u64> = Vec::new();

                for i in 0..100_000u64 {
                    let action = i % 10;
                    let cmd = match action {
                        0..=5 => {
                            let side = if i % 2 == 0 { Side::Bid } else { Side::Ask };
                            let price = if side == Side::Bid { 100 } else { 200 };
                            let order = make_order(next_id, next_id, side, price, 10);
                            resting.push(next_id);
                            next_id += 1;
                            EngineCommand::NewOrder(order)
                        }
                        6 | 7 => {
                            if let Some(id) = resting.pop() {
                                EngineCommand::CancelOrder { order_id: id }
                            } else {
                                let order = make_order(next_id, next_id, Side::Bid, 100, 10);
                                next_id += 1;
                                EngineCommand::NewOrder(order)
                            }
                        }
                        _ => {
                            let order = make_order(next_id, next_id, Side::Bid, 200, 10);
                            next_id += 1;
                            EngineCommand::NewOrder(order)
                        }
                    };
                    producer.push(cmd).unwrap();
                }

                (consumer, engine)
            },
            |(mut consumer, mut engine)| {
                let mut report_buf = [0u8; EXECUTION_REPORT_SIZE];
                let mut seq = 0u32;
                while let Ok(cmd) = consumer.pop() {
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
                }
            },
            BatchSize::LargeInput,
        );
    });
}

fn bench_e2e_per_order(c: &mut Criterion) {
    let n = 100_000usize;

    c.bench_function("e2e/avg_crossing_100k", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::ZERO;

            for _ in 0..iters {
                let (mut producer, mut consumer) =
                    ring::ring_buffer::<EngineCommand>(n.next_power_of_two() * 2);
                let mut engine = MatchingEngine::with_capacity(n as u32 + 16);
                let mut report_buf = [0u8; EXECUTION_REPORT_SIZE];
                let mut seq = 0u32;

                for i in 1..=n as u64 {
                    engine
                        .add_order(make_order(i, i, Side::Ask, 100, 10))
                        .unwrap();
                }

                for i in 1..=n as u64 {
                    producer
                        .push(EngineCommand::NewOrder(make_order(
                            n as u64 + i,
                            n as u64 + i,
                            Side::Bid,
                            100,
                            10,
                        )))
                        .unwrap();
                }

                let start = std::time::Instant::now();
                while let Ok(cmd) = consumer.pop() {
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
                }
                total += start.elapsed();
            }

            total
        });
    });
}

criterion_group!(benches, bench_e2e_crossing, bench_e2e_mixed, bench_e2e_per_order);
criterion_main!(benches);
