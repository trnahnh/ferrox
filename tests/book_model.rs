//! Differential (model-based) property test for the matching engine.
//!
//! The optimized engine (arena + intrusive lists + BTreeMap + cached best
//! prices) is run side by side with a deliberately naive reference model: a flat
//! `Vec` of resting orders that is linearly scanned for the best maker on every
//! step. The reference encodes the matching rules in the most obvious way
//! possible (price-time priority, fill at maker price, self-trade prevention
//! cancels the taker, partial fills rest) and has no data-structure cleverness
//! to get wrong.
//!
//! For a random sequence of adds and cancels the two must agree, after every
//! single operation, on:
//!   - the order status and the exact list of fills,
//!   - the result of cancels (found / not found, and which order),
//!   - best bid / best ask / resting order count,
//!   - the complete book contents in canonical order (`all_resting_orders`).
//!
//! The existing proptests in `src/matching.rs` check invariants on one or two
//! orders; this test checks hundreds of interleaved operations so that stale
//! `best_bid`/`best_ask` caches, broken intrusive-list unlinks, and arena slot
//! reuse bugs would surface.
//!
//! Run: `cargo test --test book_model`
use ferrox::matching::{Fill, MatchingEngine, OrderStatus};
use ferrox::order::{Order, Side};
use proptest::prelude::*;

const ARENA_CAPACITY: u32 = 1024;

#[derive(Clone, Debug)]
struct Resting {
    order: Order,
    seq: u64,
}

/// Naive reference order book: O(n) everything, no caches.
#[derive(Default)]
struct ReferenceBook {
    resting: Vec<Resting>,
    next_seq: u64,
}

impl ReferenceBook {
    /// Index of the maker with the best price on `side`, FIFO among equal prices.
    fn best_index(&self, side: Side) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, r) in self.resting.iter().enumerate() {
            if r.order.side != side {
                continue;
            }
            best = Some(match best {
                None => i,
                Some(b) => {
                    let cur = &self.resting[b];
                    let better_price = match side {
                        Side::Ask => r.order.price < cur.order.price,
                        Side::Bid => r.order.price > cur.order.price,
                    };
                    let same_price_earlier = r.order.price == cur.order.price && r.seq < cur.seq;
                    if better_price || same_price_earlier {
                        i
                    } else {
                        b
                    }
                }
            });
        }
        best
    }

    fn add(&mut self, mut taker: Order) -> (OrderStatus, Vec<Fill>) {
        let maker_side = match taker.side {
            Side::Bid => Side::Ask,
            Side::Ask => Side::Bid,
        };
        let mut fills = Vec::new();
        let mut self_trade = false;

        while taker.quantity > 0 {
            let Some(i) = self.best_index(maker_side) else {
                break;
            };
            let maker = &self.resting[i].order;
            let crosses = match taker.side {
                Side::Bid => maker.price <= taker.price,
                Side::Ask => maker.price >= taker.price,
            };
            if !crosses {
                break;
            }
            if maker.trader_id == taker.trader_id {
                self_trade = true;
                break;
            }
            let qty = taker.quantity.min(maker.quantity);
            fills.push(Fill {
                taker_order_id: taker.id,
                maker_order_id: maker.id,
                price: maker.price,
                quantity: qty,
                maker_fully_filled: maker.quantity == qty,
            });
            taker.quantity -= qty;
            self.resting[i].order.quantity -= qty;
            if self.resting[i].order.quantity == 0 {
                self.resting.remove(i);
            }
        }

        let status = if self_trade {
            OrderStatus::CancelledSelfTrade
        } else if taker.quantity == 0 {
            OrderStatus::FullyFilled
        } else {
            let had_fills = !fills.is_empty();
            self.resting.push(Resting {
                order: taker,
                seq: self.next_seq,
            });
            self.next_seq += 1;
            if had_fills {
                OrderStatus::PartiallyFilled
            } else {
                OrderStatus::Resting
            }
        };
        (status, fills)
    }

    fn cancel(&mut self, id: u64) -> Option<Order> {
        let i = self.resting.iter().position(|r| r.order.id == id)?;
        Some(self.resting.remove(i).order)
    }

    fn best_bid(&self) -> Option<i64> {
        self.resting
            .iter()
            .filter(|r| r.order.side == Side::Bid)
            .map(|r| r.order.price)
            .max()
    }

    fn best_ask(&self) -> Option<i64> {
        self.resting
            .iter()
            .filter(|r| r.order.side == Side::Ask)
            .map(|r| r.order.price)
            .min()
    }

    /// Same canonical order as `OrderBook::all_resting_orders`:
    /// asks ascending by price, then bids descending by price, FIFO within a level.
    fn canonical(&self) -> Vec<Order> {
        let mut asks: Vec<&Resting> = self
            .resting
            .iter()
            .filter(|r| r.order.side == Side::Ask)
            .collect();
        let mut bids: Vec<&Resting> = self
            .resting
            .iter()
            .filter(|r| r.order.side == Side::Bid)
            .collect();
        asks.sort_by_key(|r| (r.order.price, r.seq));
        bids.sort_by_key(|r| (std::cmp::Reverse(r.order.price), r.seq));
        asks.into_iter()
            .chain(bids)
            .map(|r| r.order.clone())
            .collect()
    }
}

#[derive(Clone, Debug)]
enum Op {
    Add {
        side: Side,
        price: i64,
        qty: u64,
        trader: u64,
    },
    /// Cancel one of the ids issued so far (resolved against the ids at that point).
    Cancel(prop::sample::Index),
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (
            prop_oneof![Just(Side::Bid), Just(Side::Ask)],
            95_i64..=105,
            1_u64..=20,
            0_u64..=3,
        )
            .prop_map(|(side, price, qty, trader)| Op::Add { side, price, qty, trader }),
        2 => any::<prop::sample::Index>().prop_map(Op::Cancel),
    ]
}

/// Runs the op sequence through a fresh engine and reference, checking agreement
/// after every op. Returns a transcript of (status, fills) so determinism can be
/// checked across runs.
fn run_differential(ops: &[Op]) -> Result<Vec<(OrderStatus, Vec<Fill>)>, TestCaseError> {
    let mut engine = MatchingEngine::with_capacity(ARENA_CAPACITY);
    let mut reference = ReferenceBook::default();
    let mut issued_ids: Vec<u64> = Vec::new();
    let mut transcript = Vec::new();

    for (step, op) in ops.iter().enumerate() {
        match op {
            Op::Add {
                side,
                price,
                qty,
                trader,
            } => {
                let id = step as u64 + 1;
                let order = Order::new(id, *trader, *side, *price, *qty, id).unwrap();
                issued_ids.push(id);

                let (ref_status, ref_fills) = reference.add(order.clone());
                let result = engine.add_order(order).map_err(|e| {
                    TestCaseError::fail(format!("step {step}: engine rejected order: {e:?}"))
                })?;
                prop_assert_eq!(result.status, ref_status, "step {}: status differs", step);
                prop_assert_eq!(
                    result.fills,
                    ref_fills.as_slice(),
                    "step {}: fills differ",
                    step
                );
                transcript.push((result.status, result.fills.to_vec()));
            }
            Op::Cancel(idx) => {
                if issued_ids.is_empty() {
                    continue;
                }
                let id = issued_ids[idx.index(issued_ids.len())];
                let ref_result = reference.cancel(id);
                let eng_result = engine.cancel_order(id).ok();
                prop_assert_eq!(
                    eng_result,
                    ref_result,
                    "step {}: cancel({}) differs",
                    step,
                    id
                );
            }
        }

        prop_assert_eq!(
            engine.book().best_bid(),
            reference.best_bid(),
            "step {}: best_bid",
            step
        );
        prop_assert_eq!(
            engine.book().best_ask(),
            reference.best_ask(),
            "step {}: best_ask",
            step
        );
        prop_assert_eq!(
            engine.book().order_count(),
            reference.resting.len(),
            "step {}: order_count",
            step
        );
        prop_assert_eq!(
            engine.book().all_resting_orders(),
            reference.canonical(),
            "step {}: book contents differ",
            step
        );
    }
    Ok(transcript)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        ..ProptestConfig::default()
    })]

    #[test]
    fn engine_matches_reference_model(ops in prop::collection::vec(arb_op(), 1..200)) {
        run_differential(&ops)?;
    }

    #[test]
    fn replay_is_deterministic(ops in prop::collection::vec(arb_op(), 1..100)) {
        let first = run_differential(&ops)?;
        let second = run_differential(&ops)?;
        prop_assert_eq!(first, second, "same input sequence produced different output");
    }
}

#[test]
fn reference_model_sanity() {
    // Pin the reference model itself on a hand-checked scenario so that a bug in
    // the oracle cannot silently agree with a bug in the engine.
    let mut r = ReferenceBook::default();
    let (s, f) = r.add(Order::new(1, 1, Side::Ask, 100, 5, 1).unwrap());
    assert_eq!((s, f.len()), (OrderStatus::Resting, 0));
    let (s, f) = r.add(Order::new(2, 1, Side::Ask, 101, 10, 2).unwrap());
    assert_eq!((s, f.len()), (OrderStatus::Resting, 0));
    // Trader 2 lifts both levels; second fill is partial on the maker.
    let (s, f) = r.add(Order::new(3, 2, Side::Bid, 101, 12, 3).unwrap());
    assert_eq!(s, OrderStatus::FullyFilled);
    assert_eq!(f.len(), 2);
    assert_eq!(
        (
            f[0].maker_order_id,
            f[0].price,
            f[0].quantity,
            f[0].maker_fully_filled
        ),
        (1, 100, 5, true)
    );
    assert_eq!(
        (
            f[1].maker_order_id,
            f[1].price,
            f[1].quantity,
            f[1].maker_fully_filled
        ),
        (2, 101, 7, false)
    );
    assert_eq!(r.best_ask(), Some(101));
    assert_eq!(r.canonical()[0].quantity, 3);
    // Self-trade: trader 1 buying against its own resting ask is cancelled.
    let (s, f) = r.add(Order::new(4, 1, Side::Bid, 105, 1, 4).unwrap());
    assert_eq!((s, f.len()), (OrderStatus::CancelledSelfTrade, 0));
    assert_eq!(r.resting.len(), 1);
}
