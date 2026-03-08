use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static DEALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

pub struct TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: Delegates to System allocator which upholds GlobalAlloc contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACKING.load(Ordering::Relaxed) {
            DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Delegates to System allocator which upholds GlobalAlloc contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

pub fn start_tracking() {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    DEALLOC_COUNT.store(0, Ordering::SeqCst);
    ALLOC_BYTES.store(0, Ordering::SeqCst);
    TRACKING.store(true, Ordering::SeqCst);
}

pub fn stop_tracking() -> AllocStats {
    TRACKING.store(false, Ordering::SeqCst);
    AllocStats {
        alloc_count: ALLOC_COUNT.load(Ordering::SeqCst),
        dealloc_count: DEALLOC_COUNT.load(Ordering::SeqCst),
        alloc_bytes: ALLOC_BYTES.load(Ordering::SeqCst),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AllocStats {
    pub alloc_count: u64,
    pub dealloc_count: u64,
    pub alloc_bytes: u64,
}

impl std::fmt::Display for AllocStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "allocs: {}, deallocs: {}, bytes: {}",
            self.alloc_count, self.dealloc_count, self.alloc_bytes
        )
    }
}
