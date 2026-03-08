use crate::book::{BookError, OrderBook};
use crate::order::{Order, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fill {
    pub taker_order_id: u64,
    pub maker_order_id: u64,
    pub price: i64,
    pub quantity: u64,
    pub maker_fully_filled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    FullyFilled,
    PartiallyFilled,
    Resting,
    CancelledSelfTrade,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AddOrderResult<'a> {
    pub order_id: u64,
    pub status: OrderStatus,
    pub fills: &'a [Fill],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchingError {
    Book(BookError),
    ZeroQuantity,
}

impl From<BookError> for MatchingError {
    fn from(e: BookError) -> Self {
        Self::Book(e)
    }
}
const FILLS_INITIAL_CAPACITY: usize = 16;

#[derive(Debug)]
pub struct MatchingEngine {
    book: OrderBook,
    fills_buf: Vec<Fill>,
}

impl MatchingEngine {
    pub fn new() -> Self {
        Self {
            book: OrderBook::new(),
            fills_buf: Vec::with_capacity(FILLS_INITIAL_CAPACITY),
        }
    }

    pub fn with_capacity(arena_capacity: u32) -> Self {
        Self {
            book: OrderBook::with_capacity(arena_capacity),
            fills_buf: Vec::with_capacity(FILLS_INITIAL_CAPACITY),
        }
    }

    pub fn book(&self) -> &OrderBook {
        &self.book
    }

    pub fn add_order(&mut self, mut order: Order) -> Result<AddOrderResult<'_>, MatchingError> {
        if order.quantity == 0 {
            return Err(MatchingError::ZeroQuantity);
        }

        self.fills_buf.clear();

        let order_id = order.id;
        let mut self_trade = false;

        match order.side {
            Side::Bid => {
                while order.quantity > 0 {
                    let best_ask = match self.book.best_ask() {
                        Some(p) if p <= order.price => p,
                        _ => break,
                    };

                    let maker = match self.book.peek_front(Side::Ask, best_ask) {
                        Some(m) => m,
                        None => break,
                    };

                    if maker.trader_id == order.trader_id {
                        self_trade = true;
                        break;
                    }

                    let fill_qty = order.quantity.min(maker.quantity);
                    let maker_id = maker.id;
                    let fill_price = maker.price;

                    let maker_remaining =
                        self.book
                            .reduce_front_quantity(Side::Ask, best_ask, fill_qty)?;

                    self.fills_buf.push(Fill {
                        taker_order_id: order.id,
                        maker_order_id: maker_id,
                        price: fill_price,
                        quantity: fill_qty,
                        maker_fully_filled: maker_remaining == 0,
                    });

                    order.quantity -= fill_qty;
                }
            }
            Side::Ask => {
                while order.quantity > 0 {
                    let best_bid = match self.book.best_bid() {
                        Some(p) if p >= order.price => p,
                        _ => break,
                    };

                    let maker = match self.book.peek_front(Side::Bid, best_bid) {
                        Some(m) => m,
                        None => break,
                    };

                    if maker.trader_id == order.trader_id {
                        self_trade = true;
                        break;
                    }

                    let fill_qty = order.quantity.min(maker.quantity);
                    let maker_id = maker.id;
                    let fill_price = maker.price;

                    let maker_remaining =
                        self.book
                            .reduce_front_quantity(Side::Bid, best_bid, fill_qty)?;

                    self.fills_buf.push(Fill {
                        taker_order_id: order.id,
                        maker_order_id: maker_id,
                        price: fill_price,
                        quantity: fill_qty,
                        maker_fully_filled: maker_remaining == 0,
                    });

                    order.quantity -= fill_qty;
                }
            }
        }

        let status = if self_trade {
            OrderStatus::CancelledSelfTrade
        } else if order.quantity == 0 {
            OrderStatus::FullyFilled
        } else {
            self.book.insert_order(order)?;
            if self.fills_buf.is_empty() {
                OrderStatus::Resting
            } else {
                OrderStatus::PartiallyFilled
            }
        };

        Ok(AddOrderResult {
            order_id,
            status,
            fills: &self.fills_buf,
        })
    }

    pub fn cancel_order(&mut self, order_id: u64) -> Result<Order, MatchingError> {
        Ok(self.book.cancel_order(order_id)?)
    }

    /// Inserts directly into the book without matching (non-crossed snapshot state).
    pub(crate) fn restore_from_orders(
        orders: &[Order],
        arena_capacity: u32,
    ) -> Result<Self, MatchingError> {
        let mut engine = Self::with_capacity(arena_capacity);
        for order in orders {
            engine.book.insert_order(order.clone())?;
        }
        Ok(engine)
    }
}

impl Default for MatchingEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{Order, Side};

    const TEST_CAPACITY: u32 = 1_024;

    fn engine() -> MatchingEngine {
        MatchingEngine::with_capacity(TEST_CAPACITY)
    }

    fn bid(id: u64, price: i64, qty: u64, ts: u64) -> Order {
        Order::new(id, id, Side::Bid, price, qty, ts).unwrap()
    }

    fn ask(id: u64, price: i64, qty: u64, ts: u64) -> Order {
        Order::new(id, id, Side::Ask, price, qty, ts).unwrap()
    }

    fn bid_trader(id: u64, trader_id: u64, price: i64, qty: u64, ts: u64) -> Order {
        Order::new(id, trader_id, Side::Bid, price, qty, ts).unwrap()
    }

    fn ask_trader(id: u64, trader_id: u64, price: i64, qty: u64, ts: u64) -> Order {
        Order::new(id, trader_id, Side::Ask, price, qty, ts).unwrap()
    }

    #[test]
    fn no_match_resting() {
        let mut engine = engine();

        let result = engine.add_order(bid(1, 100, 10, 1)).unwrap();
        assert_eq!(result.status, OrderStatus::Resting);
        assert!(result.fills.is_empty());

        let result = engine.add_order(ask(2, 105, 10, 2)).unwrap();
        assert_eq!(result.status, OrderStatus::Resting);
        assert!(result.fills.is_empty());

        assert_eq!(engine.book().best_bid(), Some(100));
        assert_eq!(engine.book().best_ask(), Some(105));
        assert_eq!(engine.book().order_count(), 2);
    }

    #[test]
    fn full_fill_equal_quantities() {
        let mut engine = engine();
        engine.add_order(ask(1, 100, 10, 1)).unwrap();

        let result = engine.add_order(bid(2, 100, 10, 2)).unwrap();
        assert_eq!(result.status, OrderStatus::FullyFilled);
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].quantity, 10);
        assert_eq!(result.fills[0].price, 100);
        assert_eq!(result.fills[0].taker_order_id, 2);
        assert_eq!(result.fills[0].maker_order_id, 1);
        assert!(result.fills[0].maker_fully_filled);

        assert_eq!(engine.book().order_count(), 0);
    }

    #[test]
    fn partial_fill_taker_has_more() {
        let mut engine = engine();
        engine.add_order(ask(1, 100, 5, 1)).unwrap();

        let result = engine.add_order(bid(2, 100, 10, 2)).unwrap();
        assert_eq!(result.status, OrderStatus::PartiallyFilled);
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].quantity, 5);
        assert!(result.fills[0].maker_fully_filled);

        assert_eq!(engine.book().best_bid(), Some(100));
        assert_eq!(engine.book().order_count(), 1);
    }

    #[test]
    fn partial_fill_maker_has_more() {
        let mut engine = engine();
        engine.add_order(bid(1, 100, 20, 1)).unwrap();

        let result = engine.add_order(ask(2, 100, 5, 2)).unwrap();
        assert_eq!(result.status, OrderStatus::FullyFilled);
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].quantity, 5);
        assert!(!result.fills[0].maker_fully_filled);

        assert_eq!(engine.book().order_count(), 1);
        assert_eq!(engine.book().best_bid(), Some(100));
    }

    #[test]
    fn multi_level_matching() {
        let mut engine = engine();
        engine.add_order(ask(1, 100, 5, 1)).unwrap();
        engine.add_order(ask(2, 101, 5, 2)).unwrap();
        engine.add_order(ask(3, 102, 5, 3)).unwrap();

        let result = engine.add_order(bid(4, 102, 12, 4)).unwrap();
        assert_eq!(result.status, OrderStatus::FullyFilled);
        assert_eq!(result.fills.len(), 3);

        assert_eq!(result.fills[0].price, 100);
        assert_eq!(result.fills[0].quantity, 5);
        assert_eq!(result.fills[1].price, 101);
        assert_eq!(result.fills[1].quantity, 5);
        assert_eq!(result.fills[2].price, 102);
        assert_eq!(result.fills[2].quantity, 2);
        assert!(!result.fills[2].maker_fully_filled);

        assert_eq!(engine.book().order_count(), 1);
        assert_eq!(engine.book().best_ask(), Some(102));
    }

    #[test]
    fn fifo_within_price_level() {
        let mut engine = engine();
        engine.add_order(ask(1, 100, 10, 1)).unwrap();
        engine.add_order(ask(2, 100, 10, 2)).unwrap();
        engine.add_order(ask(3, 100, 10, 3)).unwrap();

        let result = engine.add_order(bid(4, 100, 15, 4)).unwrap();
        assert_eq!(result.fills.len(), 2);
        assert_eq!(result.fills[0].maker_order_id, 1);
        assert_eq!(result.fills[0].quantity, 10);
        assert_eq!(result.fills[1].maker_order_id, 2);
        assert_eq!(result.fills[1].quantity, 5);
    }

    #[test]
    fn fill_price_is_maker_price() {
        let mut engine = engine();
        engine.add_order(ask(1, 100, 10, 1)).unwrap();

        let result = engine.add_order(bid(2, 110, 10, 2)).unwrap();
        assert_eq!(result.fills[0].price, 100);
    }

    #[test]
    fn ask_taker_matches_bids() {
        let mut engine = engine();
        engine.add_order(bid(1, 102, 10, 1)).unwrap();
        engine.add_order(bid(2, 101, 10, 2)).unwrap();

        let result = engine.add_order(ask(3, 101, 15, 3)).unwrap();
        assert_eq!(result.fills.len(), 2);
        assert_eq!(result.fills[0].maker_order_id, 1);
        assert_eq!(result.fills[0].price, 102);
        assert_eq!(result.fills[0].quantity, 10);
        assert_eq!(result.fills[1].maker_order_id, 2);
        assert_eq!(result.fills[1].price, 101);
        assert_eq!(result.fills[1].quantity, 5);

        assert_eq!(result.status, OrderStatus::FullyFilled);
    }

    #[test]
    fn cancel_resting_order() {
        let mut engine = engine();
        engine.add_order(bid(1, 100, 10, 1)).unwrap();

        let cancelled = engine.cancel_order(1).unwrap();
        assert_eq!(cancelled.id, 1);
        assert_eq!(engine.book().order_count(), 0);
    }

    #[test]
    fn cancel_nonexistent_fails() {
        let mut engine = engine();
        let err = engine.cancel_order(999).unwrap_err();
        assert_eq!(err, MatchingError::Book(BookError::OrderNotFound(999)));
    }

    #[test]
    fn zero_quantity_rejected() {
        let mut engine = engine();
        let order = Order {
            id: 1,
            trader_id: 1,
            side: Side::Bid,
            price: 100,
            quantity: 0,
            timestamp: 1,
        };
        let err = engine.add_order(order).unwrap_err();
        assert_eq!(err, MatchingError::ZeroQuantity);
    }

    #[test]
    fn empty_book_no_match() {
        let mut engine = engine();
        let result = engine.add_order(bid(1, 100, 10, 1)).unwrap();
        assert_eq!(result.status, OrderStatus::Resting);
        assert!(result.fills.is_empty());
    }

    #[test]
    fn bid_below_best_ask_no_match() {
        let mut engine = engine();
        engine.add_order(ask(1, 105, 10, 1)).unwrap();

        let result = engine.add_order(bid(2, 100, 10, 2)).unwrap();
        assert_eq!(result.status, OrderStatus::Resting);
        assert!(result.fills.is_empty());
        assert_eq!(engine.book().order_count(), 2);
    }

    #[test]
    fn self_trade_prevented_cancel_newest() {
        let mut engine = engine();
        engine.add_order(ask_trader(1, 1, 100, 10, 1)).unwrap();

        let result = engine.add_order(bid_trader(2, 1, 100, 10, 2)).unwrap();
        assert_eq!(result.status, OrderStatus::CancelledSelfTrade);
        assert!(result.fills.is_empty());

        assert_eq!(engine.book().order_count(), 1);
        assert_eq!(engine.book().best_ask(), Some(100));
    }

    #[test]
    fn self_trade_different_traders_allowed() {
        let mut engine = engine();
        engine.add_order(ask_trader(1, 1, 100, 10, 1)).unwrap();

        let result = engine.add_order(bid_trader(2, 2, 100, 10, 2)).unwrap();
        assert_eq!(result.status, OrderStatus::FullyFilled);
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].quantity, 10);
        assert_eq!(engine.book().order_count(), 0);
    }

    #[test]
    fn self_trade_partial_fill_then_cancel() {
        let mut engine = engine();
        engine.add_order(ask_trader(1, 10, 100, 5, 1)).unwrap();
        engine.add_order(ask_trader(2, 20, 101, 10, 2)).unwrap();

        let result = engine.add_order(bid_trader(3, 20, 101, 15, 3)).unwrap();
        assert_eq!(result.status, OrderStatus::CancelledSelfTrade);
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].maker_order_id, 1);
        assert_eq!(result.fills[0].quantity, 5);

        assert_eq!(engine.book().order_count(), 1);
        assert_eq!(engine.book().best_ask(), Some(101));
    }

    #[test]
    fn restore_empty() {
        let engine = MatchingEngine::restore_from_orders(&[], TEST_CAPACITY).unwrap();
        assert_eq!(engine.book().order_count(), 0);
        assert_eq!(engine.book().best_bid(), None);
        assert_eq!(engine.book().best_ask(), None);
    }

    #[test]
    fn restore_from_orders_rebuilds_book() {
        let orders = vec![
            ask(1, 105, 10, 1),
            ask(2, 110, 20, 2),
            bid(3, 100, 30, 3),
            bid(4, 98, 40, 4),
        ];

        let engine = MatchingEngine::restore_from_orders(&orders, TEST_CAPACITY).unwrap();
        assert_eq!(engine.book().order_count(), 4);
        assert_eq!(engine.book().best_bid(), Some(100));
        assert_eq!(engine.book().best_ask(), Some(105));
    }

    #[test]
    fn restore_then_match() {
        let orders = vec![ask(1, 100, 10, 1), ask(2, 101, 20, 2)];
        let mut engine = MatchingEngine::restore_from_orders(&orders, TEST_CAPACITY).unwrap();

        let result = engine.add_order(bid(3, 101, 15, 3)).unwrap();
        assert_eq!(result.fills.len(), 2);
        assert_eq!(result.fills[0].maker_order_id, 1);
        assert_eq!(result.fills[0].quantity, 10);
        assert_eq!(result.fills[1].maker_order_id, 2);
        assert_eq!(result.fills[1].quantity, 5);
        assert_eq!(result.status, OrderStatus::FullyFilled);
    }

    #[test]
    fn self_trade_multiple_resting_same_trader() {
        let mut engine = engine();
        engine.add_order(ask_trader(1, 1, 100, 10, 1)).unwrap();
        engine.add_order(ask_trader(2, 1, 101, 10, 2)).unwrap();

        let result = engine.add_order(bid_trader(3, 1, 105, 30, 3)).unwrap();
        assert_eq!(result.status, OrderStatus::CancelledSelfTrade);
        assert!(result.fills.is_empty());

        assert_eq!(engine.book().order_count(), 2);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::order::{Order, Side};
    use proptest::prelude::*;

    const TEST_CAPACITY: u32 = 1_024;

    fn engine() -> MatchingEngine {
        MatchingEngine::with_capacity(TEST_CAPACITY)
    }

    fn arb_side() -> impl Strategy<Value = Side> {
        prop_oneof![Just(Side::Bid), Just(Side::Ask)]
    }

    proptest! {
        #[test]
        fn quantity_conservation(
            price in 1_i64..=1000,
            maker_qty in 1_u64..=1000,
            taker_qty in 1_u64..=1000,
        ) {
            let mut engine = engine();
            engine.add_order(Order::new(1, 1, Side::Ask, price, maker_qty, 1).unwrap()).unwrap();

            let result = engine.add_order(Order::new(2, 2, Side::Bid, price, taker_qty, 2).unwrap()).unwrap();

            let filled: u64 = result.fills.iter().map(|f| f.quantity).sum();
            let remainder = match result.status {
                OrderStatus::FullyFilled => 0,
                OrderStatus::PartiallyFilled | OrderStatus::Resting => {
                    taker_qty - filled
                }
                OrderStatus::CancelledSelfTrade => taker_qty - filled,
            };
            prop_assert_eq!(filled + remainder, taker_qty);
        }

        #[test]
        fn no_crossed_book(
            orders in proptest::collection::vec(
                (arb_side(), 1_i64..=100, 1_u64..=100),
                1..50,
            )
        ) {
            let mut engine = engine();
            for (i, (side, price, qty)) in orders.into_iter().enumerate() {
                let id = (i + 1) as u64;
                let order = Order::new(id, id, side, price, qty, id).unwrap();
                let _ = engine.add_order(order);
            }

            if let (Some(bb), Some(ba)) = (engine.book().best_bid(), engine.book().best_ask()) {
                prop_assert!(bb < ba, "crossed book: best_bid={bb} >= best_ask={ba}");
            }
        }

        #[test]
        fn crosses_produce_fills(
            price in 1_i64..=1000,
            maker_qty in 1_u64..=1000,
            taker_qty in 1_u64..=1000,
        ) {
            let mut engine = engine();
            engine.add_order(Order::new(1, 1, Side::Ask, price, maker_qty, 1).unwrap()).unwrap();

            let result = engine.add_order(Order::new(2, 2, Side::Bid, price, taker_qty, 2).unwrap()).unwrap();
            prop_assert!(!result.fills.is_empty(), "bid >= ask but no fills produced");
        }

        #[test]
        fn fill_quantities_positive(
            orders in proptest::collection::vec(
                (arb_side(), 1_i64..=100, 1_u64..=100),
                1..50,
            )
        ) {
            let mut engine = engine();
            for (i, (side, price, qty)) in orders.into_iter().enumerate() {
                let id = (i + 1) as u64;
                let order = Order::new(id, id, side, price, qty, id).unwrap();
                if let Ok(result) = engine.add_order(order) {
                    for fill in result.fills {
                        prop_assert!(fill.quantity > 0, "fill with zero quantity");
                    }
                }
            }
        }

        #[test]
        fn no_self_trade_fills(
            price in 1_i64..=100,
            maker_qty in 1_u64..=100,
            taker_qty in 1_u64..=100,
        ) {
            let mut engine = engine();
            engine.add_order(Order::new(1, 42, Side::Ask, price, maker_qty, 1).unwrap()).unwrap();

            let result = engine.add_order(Order::new(2, 42, Side::Bid, price, taker_qty, 2).unwrap()).unwrap();

            prop_assert!(result.fills.is_empty(), "self-trade produced fills");
            prop_assert_eq!(result.status, OrderStatus::CancelledSelfTrade);
        }
    }
}
