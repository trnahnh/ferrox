//! Lock-free single-producer single-consumer ring buffer (Disruptor pattern).
//!
//! # Verification and experiment knobs
//!
//! All knobs are `--cfg` flags passed through `RUSTFLAGS`; none is set in a
//! default build, so the shipped code path is exactly the plain
//! acquire/release, cache-padded, cursor-caching ring.
//!
//! | cfg                      | effect                                                                 |
//! |--------------------------|------------------------------------------------------------------------|
//! | `loom`                   | use loom's instrumented atomics/`UnsafeCell`/`Arc` so `tests/loom_ring.rs` can model-check every interleaving under the C11 memory model |
//! | `ring_seqcst`            | `SeqCst` on every cursor access (x86-64: `xchg` instead of `mov` for the stores) |
//! | `ring_no_padding`        | head and tail share one cache line (false sharing experiment)         |
//! | `ring_no_cursor_cache`   | always reload the remote cursor instead of using the local copy        |
//! | `ring_relaxed_ordering`  | `Relaxed` cursor accesses. **Unsound.** Only compiles together with `loom` (to show the model checker catches the resulting data race) or `ring_allow_unsound` (to show ThreadSanitizer does) |
//! | `ring_allow_unsound`     | lets `ring_relaxed_ordering` compile without `loom`, for sanitizer negative controls only; never for a binary anyone runs for real |
//! | `ring_unchecked_index`   | index the slot array with `get_unchecked` (the index is always `< len` because it is masked, but safe Rust cannot express that); measures the cost of the bounds check |
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test --release --test loom_ring
//! RUSTFLAGS="--cfg ring_no_padding" cargo run --release --example ring_throughput
//! ```
use std::fmt;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::{Deref, DerefMut};

#[cfg(not(loom))]
use std::cell::UnsafeCell;
#[cfg(not(loom))]
use std::sync::Arc;
#[cfg(not(loom))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(loom)]
use loom::cell::UnsafeCell;
#[cfg(loom)]
use loom::sync::Arc;
#[cfg(loom)]
use loom::sync::atomic::{AtomicUsize, Ordering};

#[cfg(all(ring_relaxed_ordering, not(loom), not(ring_allow_unsound)))]
compile_error!(
    "`--cfg ring_relaxed_ordering` makes the ring buffer unsound and is only allowed together with `--cfg loom` (model checking) or `--cfg ring_allow_unsound` (sanitizer negative controls)"
);

/// Ordering used for the cursor stores (`head` by the producer, `tail` by the consumer).
const STORE_ORDER: Ordering = if cfg!(ring_relaxed_ordering) {
    Ordering::Relaxed
} else if cfg!(ring_seqcst) {
    Ordering::SeqCst
} else {
    Ordering::Release
};

/// Ordering used for the cursor loads that observe the other side's progress.
const LOAD_ORDER: Ordering = if cfg!(ring_relaxed_ordering) {
    Ordering::Relaxed
} else if cfg!(ring_seqcst) {
    Ordering::SeqCst
} else {
    Ordering::Acquire
};

/// Pads `T` to its own 64-byte cache line so two frequently written values do
/// not ping-pong a shared line between cores (false sharing).
#[cfg_attr(not(ring_no_padding), repr(align(64)))]
pub struct CachePadded<T> {
    value: T,
}

impl<T> CachePadded<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<T> Deref for CachePadded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for CachePadded<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

pub struct Full<T>(pub T);

impl<T> fmt::Debug for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Full(..)")
    }
}

#[derive(Debug)]
pub struct Empty;

/// One buffer slot. Wraps the interior-mutable storage so the std and loom
/// code paths differ only here; the ring logic above it is identical.
struct Slot<T>(UnsafeCell<MaybeUninit<T>>);

impl<T> Slot<T> {
    fn uninit() -> Self {
        Self(UnsafeCell::new(MaybeUninit::uninit()))
    }
}

#[cfg(not(loom))]
impl<T> Slot<T> {
    /// # Safety
    /// The caller must be the only party accessing this slot (the producer, for
    /// a slot the consumer has released) and the slot must not hold a live value.
    #[inline]
    unsafe fn write(&self, value: T) {
        // SAFETY: exclusive access is guaranteed by the SPSC protocol (see
        // `Producer::push`), so no other reference to this slot exists.
        unsafe { (*self.0.get()).write(value) };
    }

    /// # Safety
    /// The slot must hold an initialized value that the caller now exclusively owns.
    #[inline]
    unsafe fn read(&self) -> T {
        // SAFETY: exclusive access is guaranteed by the SPSC protocol (see
        // `Consumer::pop`); the value was initialized by `write`.
        unsafe { (*self.0.get()).assume_init_read() }
    }

    /// # Safety
    /// The slot must hold an initialized value. Only used from `Drop` with `&mut`.
    unsafe fn drop_in_place(&mut self) {
        // SAFETY: `&mut self` proves exclusivity; the caller guarantees the
        // slot is initialized.
        unsafe { (*self.0.get()).assume_init_drop() };
    }
}

#[cfg(loom)]
impl<T> Slot<T> {
    /// # Safety
    /// Same contract as the non-loom version; loom additionally verifies that
    /// this access is ordered before/after every other access to the slot.
    #[inline]
    unsafe fn write(&self, value: T) {
        self.0.with_mut(|ptr| unsafe { (*ptr).write(value) });
    }

    /// # Safety
    /// Same contract as the non-loom version.
    #[inline]
    unsafe fn read(&self) -> T {
        self.0.with(|ptr| unsafe { (*ptr).assume_init_read() })
    }

    /// # Safety
    /// Same contract as the non-loom version.
    unsafe fn drop_in_place(&mut self) {
        self.0.with_mut(|ptr| unsafe { (*ptr).assume_init_drop() });
    }
}

struct RingBufferInner<T> {
    buffer: Box<[Slot<T>]>,
    capacity: usize,
    mask: usize,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
}

impl<T> RingBufferInner<T> {
    /// The slot for a free-running cursor value.
    #[inline]
    fn slot(&self, cursor: usize) -> &Slot<T> {
        let index = cursor & self.mask;

        #[cfg(not(ring_unchecked_index))]
        {
            &self.buffer[index]
        }

        #[cfg(ring_unchecked_index)]
        // SAFETY: `mask == buffer.len() - 1` and `buffer.len()` is a power of
        // two, so `cursor & mask < buffer.len()` for every cursor value.
        unsafe {
            self.buffer.get_unchecked(index)
        }
    }
}

// SAFETY: The SPSC protocol guarantees that only the Producer writes to slots
// and advances `head`, while only the Consumer reads from slots and advances
// `tail`. The Acquire/Release ordering on the atomic cursors establishes the
// necessary happens-before relationships. `UnsafeCell` access is safe because
// each slot is exclusively accessed by one side at a time.
unsafe impl<T: Send> Sync for RingBufferInner<T> {}

impl<T> Drop for RingBufferInner<T> {
    fn drop(&mut self) {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);

        // SAFETY: We have exclusive `&mut self` access (Drop guarantees no
        // concurrent readers). Items in `tail..head` were written by the
        // producer and never read by the consumer, so they are initialized
        // and need dropping.
        let mut i = tail;
        while i != head {
            unsafe { self.buffer[i & self.mask].drop_in_place() };
            i = i.wrapping_add(1);
        }
    }
}

/// Shared ownership of the ring, dropped by value.
///
/// `Arc`'s own `Drop` hands `&mut self` to the out-of-line `Arc::drop_slow`.
/// When a `Producer`/`Consumer` is a local variable, that call (which exists
/// even if it only sits on the panic-cleanup path of the caller's loop) makes
/// the address of the whole struct escape, so LLVM cannot promote
/// `cached_head`/`cached_tail` to registers and every push/pop round-trips the
/// cursors through the stack. Taking the `Arc` out by value in our own `Drop`
/// means only a temporary's address is ever passed, and the hot-loop locals
/// stay in registers. See docs/MEMORY_ORDERING.md §5 for the measurements.
struct Shared<T>(ManuallyDrop<Arc<RingBufferInner<T>>>);

impl<T> Shared<T> {
    #[inline]
    fn new(inner: Arc<RingBufferInner<T>>) -> Self {
        Self(ManuallyDrop::new(inner))
    }
}

impl<T> Deref for Shared<T> {
    type Target = RingBufferInner<T>;
    #[inline]
    fn deref(&self) -> &RingBufferInner<T> {
        &self.0
    }
}

impl<T> Drop for Shared<T> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: `Drop::drop` runs exactly once and nothing reads `self.0`
        // afterwards, so the `Arc` is moved out exactly once.
        drop(unsafe { ManuallyDrop::take(&mut self.0) });
    }
}

pub struct Producer<T> {
    inner: Shared<T>,
    cached_head: usize,
    cached_tail: usize,
}

// SAFETY: Producer is the sole writer. It can be sent to another thread
// as long as T: Send (values cross thread boundaries).
unsafe impl<T: Send> Send for Producer<T> {}

impl<T> Producer<T> {
    pub fn push(&mut self, value: T) -> Result<(), Full<T>> {
        let head = self.cached_head;

        if cfg!(ring_no_cursor_cache) || head.wrapping_sub(self.cached_tail) == self.inner.capacity
        {
            self.cached_tail = self.inner.tail.load(LOAD_ORDER);
            if head.wrapping_sub(self.cached_tail) == self.inner.capacity {
                return Err(Full(value));
            }
        }

        let slot = self.inner.slot(head);

        // SAFETY: Producer has exclusive write access to buffer[head & mask].
        // The slot has been released by the consumer (tail has advanced past it)
        // or was never written (initial state). The Acquire load of `tail`
        // above ensures the consumer's read of the previous value is complete.
        unsafe { slot.write(value) };

        self.inner.head.store(head.wrapping_add(1), STORE_ORDER);
        self.cached_head = head.wrapping_add(1);

        Ok(())
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }
}

pub struct Consumer<T> {
    inner: Shared<T>,
    cached_tail: usize,
    cached_head: usize,
}

// SAFETY: Consumer is the sole reader. It can be sent to another thread
// as long as T: Send (values cross thread boundaries).
unsafe impl<T: Send> Send for Consumer<T> {}

impl<T> Consumer<T> {
    pub fn pop(&mut self) -> Result<T, Empty> {
        let tail = self.cached_tail;

        if cfg!(ring_no_cursor_cache) || tail == self.cached_head {
            self.cached_head = self.inner.head.load(LOAD_ORDER);
            if tail == self.cached_head {
                return Err(Empty);
            }
        }

        let slot = self.inner.slot(tail);

        // SAFETY: Consumer has exclusive read access to buffer[tail & mask].
        // The slot was written by the producer (head has advanced past it).
        // The Acquire load of `head` above ensures the producer's write is
        // visible.
        let value = unsafe { slot.read() };

        self.inner.tail.store(tail.wrapping_add(1), STORE_ORDER);
        self.cached_tail = tail.wrapping_add(1);

        Ok(value)
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }
}

pub fn ring_buffer<T: Send>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    assert!(
        capacity > 0,
        "ring buffer capacity must be greater than zero"
    );
    assert!(
        capacity.is_power_of_two(),
        "ring buffer capacity must be a power of two, got {capacity}"
    );

    let mut buffer = Vec::with_capacity(capacity);
    for _ in 0..capacity {
        buffer.push(Slot::uninit());
    }

    let inner = Arc::new(RingBufferInner {
        buffer: buffer.into_boxed_slice(),
        capacity,
        mask: capacity - 1,
        head: CachePadded::new(AtomicUsize::new(0)),
        tail: CachePadded::new(AtomicUsize::new(0)),
    });

    let producer = Producer {
        inner: Shared::new(Arc::clone(&inner)),
        cached_head: 0,
        cached_tail: 0,
    };

    let consumer = Consumer {
        inner: Shared::new(inner),
        cached_tail: 0,
        cached_head: 0,
    };

    (producer, consumer)
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize as StdAtomicUsize;
    use std::thread;

    #[test]
    fn push_pop_single() {
        let (mut p, mut c) = ring_buffer::<u64>(4);
        p.push(42).unwrap();
        assert_eq!(c.pop().unwrap(), 42);
    }

    #[test]
    fn push_pop_fifo() {
        let (mut p, mut c) = ring_buffer::<u64>(8);
        for i in 0..8 {
            p.push(i).unwrap();
        }
        for i in 0..8 {
            assert_eq!(c.pop().unwrap(), i);
        }
    }

    #[test]
    fn full_returns_error() {
        let (mut p, mut _c) = ring_buffer::<u64>(4);
        for i in 0..4 {
            p.push(i).unwrap();
        }
        let err = p.push(99).unwrap_err();
        assert_eq!(err.0, 99);
    }

    #[test]
    fn empty_returns_error() {
        let (_p, mut c) = ring_buffer::<u64>(4);
        assert!(c.pop().is_err());
    }

    #[test]
    fn wraparound() {
        let (mut p, mut c) = ring_buffer::<u64>(4);
        for i in 0..100 {
            p.push(i).unwrap();
            assert_eq!(c.pop().unwrap(), i);
        }
    }

    #[test]
    fn fill_then_drain() {
        let (mut p, mut c) = ring_buffer::<u64>(8);

        for i in 0..8 {
            p.push(i).unwrap();
        }
        assert!(p.push(99).is_err());

        for i in 0..8 {
            assert_eq!(c.pop().unwrap(), i);
        }
        assert!(c.pop().is_err());

        for i in 100..108 {
            p.push(i).unwrap();
        }
        for i in 100..108 {
            assert_eq!(c.pop().unwrap(), i);
        }
    }

    #[test]
    fn capacity_accessor() {
        let (p, c) = ring_buffer::<u64>(16);
        assert_eq!(p.capacity(), 16);
        assert_eq!(c.capacity(), 16);
    }

    #[test]
    #[should_panic(expected = "greater than zero")]
    fn zero_capacity_panics() {
        ring_buffer::<u64>(0);
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn non_power_of_two_panics() {
        ring_buffer::<u64>(3);
    }

    #[test]
    fn drop_remaining_items() {
        let drop_count = Arc::new(StdAtomicUsize::new(0));

        struct DropCounter(Arc<StdAtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        {
            let (mut p, _c) = ring_buffer::<DropCounter>(4);
            p.push(DropCounter(Arc::clone(&drop_count))).unwrap();
            p.push(DropCounter(Arc::clone(&drop_count))).unwrap();
            p.push(DropCounter(Arc::clone(&drop_count))).unwrap();
        }

        assert_eq!(drop_count.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn concurrent_push_pop() {
        let (mut p, mut c) = ring_buffer::<u64>(1024);
        let count = 100_000u64;

        let producer = thread::spawn(move || {
            for i in 0..count {
                while p.push(i).is_err() {
                    thread::yield_now();
                }
            }
        });

        let mut received = Vec::with_capacity(count as usize);
        for _ in 0..count {
            loop {
                match c.pop() {
                    Ok(v) => {
                        received.push(v);
                        break;
                    }
                    Err(_) => thread::yield_now(),
                }
            }
        }

        producer.join().unwrap();

        let expected: Vec<u64> = (0..count).collect();
        assert_eq!(received, expected);
    }

    #[test]
    fn concurrent_backpressure() {
        let (mut p, mut c) = ring_buffer::<u64>(16);
        let count = 100_000u64;

        let producer = thread::spawn(move || {
            for i in 0..count {
                while p.push(i).is_err() {
                    thread::yield_now();
                }
            }
        });

        let mut received = Vec::with_capacity(count as usize);
        for _ in 0..count {
            loop {
                match c.pop() {
                    Ok(v) => {
                        received.push(v);
                        break;
                    }
                    Err(_) => thread::yield_now(),
                }
            }
        }

        producer.join().unwrap();

        let expected: Vec<u64> = (0..count).collect();
        assert_eq!(received, expected);
    }

    /// Same shape and size as `crate::order::Order` (48 bytes), defined here so
    /// the ring tests have no dependency outside this module and can be run
    /// standalone under Miri and ThreadSanitizer (see `verify/ring-standalone`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Payload48 {
        id: u64,
        trader_id: u64,
        price: i64,
        quantity: u64,
        timestamp: u64,
        side: u8,
    }

    #[test]
    fn concurrent_order_struct() {
        const _: () = assert!(std::mem::size_of::<Payload48>() == 48);

        let (mut p, mut c) = ring_buffer::<Payload48>(256);
        let count = 10_000u64;

        let producer = thread::spawn(move || {
            for i in 0..count {
                let order = Payload48 {
                    id: i,
                    trader_id: i % 100,
                    price: 10_000 + (i as i64 % 500),
                    quantity: (i % 1000) + 1,
                    timestamp: i * 1000,
                    side: (i % 2) as u8,
                };
                while p.push(order.clone()).is_err() {
                    thread::yield_now();
                }
            }
        });

        let mut received = Vec::with_capacity(count as usize);
        for _ in 0..count {
            loop {
                match c.pop() {
                    Ok(order) => {
                        received.push(order);
                        break;
                    }
                    Err(_) => thread::yield_now(),
                }
            }
        }

        producer.join().unwrap();

        for (i, order) in received.iter().enumerate() {
            let i = i as u64;
            assert_eq!(order.id, i);
            assert_eq!(order.trader_id, i % 100);
            assert_eq!(order.side, (i % 2) as u8);
            assert_eq!(order.price, 10_000 + (i as i64 % 500));
            assert_eq!(order.quantity, (i % 1000) + 1);
            assert_eq!(order.timestamp, i * 1000);
        }
    }
}
