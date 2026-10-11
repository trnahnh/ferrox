//! Model-based property test for the SPSC ring buffer (single-threaded).
//!
//! A `VecDeque` bounded to the ring's capacity is the oracle. For a random
//! sequence of pushes and pops the ring must report `Full` / `Empty` exactly
//! when the model does, hand back the rejected value on `Full`, and yield the
//! same values in the same order. Covers wraparound of the free-running
//! cursors across many capacities. (Cross-thread behaviour is covered by the
//! loom model-check in `tests/loom_ring.rs` and the concurrent unit tests.)
//!
//! Run: `cargo test --test ring_model`
use ferrox::ring::ring_buffer;
use proptest::prelude::*;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
enum Op {
    Push(u64),
    Pop,
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any::<u64>().prop_map(Op::Push),
        2 => Just(Op::Pop),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        ..ProptestConfig::default()
    })]

    #[test]
    fn ring_matches_bounded_deque(
        capacity_log2 in 0_u32..=6,
        ops in prop::collection::vec(arb_op(), 1..400),
    ) {
        let capacity = 1usize << capacity_log2;
        let (mut producer, mut consumer) = ring_buffer::<u64>(capacity);
        let mut model: VecDeque<u64> = VecDeque::with_capacity(capacity);

        for (step, op) in ops.iter().enumerate() {
            match op {
                Op::Push(v) => {
                    let result = producer.push(*v);
                    if model.len() == capacity {
                        let rejected = result.err().map(|full| full.0);
                        prop_assert_eq!(rejected, Some(*v), "step {}: expected Full({})", step, v);
                    } else {
                        prop_assert!(result.is_ok(), "step {}: unexpected Full", step);
                        model.push_back(*v);
                    }
                }
                Op::Pop => {
                    prop_assert_eq!(consumer.pop().ok(), model.pop_front(), "step {}: pop differs", step);
                }
            }
        }

        // Drain and compare the tail of the sequence too.
        while let Some(expected) = model.pop_front() {
            prop_assert_eq!(consumer.pop().ok(), Some(expected));
        }
        prop_assert!(consumer.pop().is_err());
    }
}
