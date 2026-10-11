// Single-producer single-consumer lock-free ring buffer in C++20.
//
// This is a line-for-line port of `ferrox::ring` (src/ring.rs) so that the two
// implementations can be benchmarked against each other on identical workloads:
//   - power-of-two capacity, bitmask indexing (`index & mask`)
//   - free-running head/tail cursors that never wrap (unsigned overflow is defined)
//   - each cursor on its own cache line (`alignas(64)`) to avoid false sharing
//   - acquire/release ordering only (no seq_cst, no fences)
//   - producer caches the last-seen tail, consumer caches the last-seen head,
//     so the shared cache line is only touched when the local view says
//     full/empty
//
// Compile-time knobs, mirroring the Rust `--cfg` knobs in src/ring.rs:
//   -DFERROX_NO_PADDING      put head and tail on the same cache line
//   -DFERROX_NO_CURSOR_CACHE always reload the remote cursor
//   -DFERROX_SEQ_CST         use memory_order_seq_cst for every cursor access
//   -DFERROX_RELAXED_UNSOUND use memory_order_relaxed (a data race; for
//                            sanitizer demonstrations only)
#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <memory>
#include <new>
#include <stdexcept>
#include <utility>

namespace ferrox {

#ifndef FERROX_CACHE_LINE
#define FERROX_CACHE_LINE 64
#endif

#ifdef FERROX_NO_PADDING
#define FERROX_CURSOR_ALIGN
#else
#define FERROX_CURSOR_ALIGN alignas(FERROX_CACHE_LINE)
#endif

#if defined(FERROX_SEQ_CST)
inline constexpr std::memory_order kStoreOrder = std::memory_order_seq_cst;
inline constexpr std::memory_order kLoadOrder = std::memory_order_seq_cst;
inline constexpr const char* kOrderingName = "seq_cst";
#elif defined(FERROX_RELAXED_UNSOUND)
inline constexpr std::memory_order kStoreOrder = std::memory_order_relaxed;
inline constexpr std::memory_order kLoadOrder = std::memory_order_relaxed;
inline constexpr const char* kOrderingName = "relaxed (UNSOUND)";
#else
inline constexpr std::memory_order kStoreOrder = std::memory_order_release;
inline constexpr std::memory_order kLoadOrder = std::memory_order_acquire;
inline constexpr const char* kOrderingName = "acquire/release";
#endif

#ifdef FERROX_NO_PADDING
inline constexpr bool kCursorsPadded = false;
#else
inline constexpr bool kCursorsPadded = true;
#endif

#ifdef FERROX_NO_CURSOR_CACHE
inline constexpr bool kCursorCache = false;
#else
inline constexpr bool kCursorCache = true;
#endif

template <typename T>
class SpscRing {
 public:
  class Producer;
  class Consumer;

  // Equivalent of `ferrox::ring::ring_buffer(capacity)`.
  static std::pair<Producer, Consumer> create(std::size_t capacity) {
    auto inner = std::make_shared<Inner>(capacity);
    return {Producer(inner), Consumer(std::move(inner))};
  }

 private:
  // Raw storage for one T; the object is constructed/destroyed explicitly.
  struct Slot {
    alignas(T) unsigned char bytes[sizeof(T)];
    void* raw() noexcept { return static_cast<void*>(bytes); }
    T* object() noexcept { return std::launder(reinterpret_cast<T*>(bytes)); }
  };

  struct FERROX_CURSOR_ALIGN Cursor {
    std::atomic<std::size_t> value{0};
  };

  struct Inner {
    explicit Inner(std::size_t cap) : capacity(cap), mask(cap - 1) {
      if (cap == 0 || (cap & (cap - 1)) != 0) {
        throw std::invalid_argument(
            "ring buffer capacity must be a non-zero power of two");
      }
      buffer.reset(new Slot[cap]);
    }

    // Items in [tail, head) were produced but never consumed: destroy them.
    // Only reached once both Producer and Consumer are gone, so plain loads
    // are sufficient: the shared_ptr refcount release/acquire orders this
    // destructor after the last access from either side.
    ~Inner() {
      const std::size_t h = head.value.load(std::memory_order_relaxed);
      const std::size_t t = tail.value.load(std::memory_order_relaxed);
      for (std::size_t i = t; i != h; ++i) {
        buffer[i & mask].object()->~T();
      }
    }

    Inner(const Inner&) = delete;
    Inner& operator=(const Inner&) = delete;

    std::unique_ptr<Slot[]> buffer;
    std::size_t capacity;
    std::size_t mask;
    Cursor head;  // advanced by the producer only
    Cursor tail;  // advanced by the consumer only
  };

 public:
  class Producer {
   public:
    Producer(Producer&&) noexcept = default;
    Producer& operator=(Producer&&) noexcept = default;
    Producer(const Producer&) = delete;
    Producer& operator=(const Producer&) = delete;

    // Returns false (and leaves `value` untouched) when the ring is full,
    // mirroring `Err(Full(value))` in Rust.
    template <typename U>
    bool push(U&& value) {
      const std::size_t head = cached_head_;
      if (!kCursorCache || head - cached_tail_ == inner_->capacity) {
        cached_tail_ = inner_->tail.value.load(kLoadOrder);
        if (head - cached_tail_ == inner_->capacity) {
          return false;
        }
      }
      ::new (inner_->buffer[head & inner_->mask].raw()) T(std::forward<U>(value));
      inner_->head.value.store(head + 1, kStoreOrder);
      cached_head_ = head + 1;
      return true;
    }

    std::size_t capacity() const noexcept { return inner_->capacity; }

   private:
    friend class SpscRing;
    explicit Producer(std::shared_ptr<Inner> inner) : inner_(std::move(inner)) {}

    std::shared_ptr<Inner> inner_;
    std::size_t cached_head_ = 0;
    std::size_t cached_tail_ = 0;
  };

  class Consumer {
   public:
    Consumer(Consumer&&) noexcept = default;
    Consumer& operator=(Consumer&&) noexcept = default;
    Consumer(const Consumer&) = delete;
    Consumer& operator=(const Consumer&) = delete;

    // Returns false when the ring is empty, mirroring `Err(Empty)` in Rust.
    bool pop(T& out) {
      const std::size_t tail = cached_tail_;
      if (!kCursorCache || tail == cached_head_) {
        cached_head_ = inner_->head.value.load(kLoadOrder);
        if (tail == cached_head_) {
          return false;
        }
      }
      T* obj = inner_->buffer[tail & inner_->mask].object();
      out = std::move(*obj);
      obj->~T();
      inner_->tail.value.store(tail + 1, kStoreOrder);
      cached_tail_ = tail + 1;
      return true;
    }

    std::size_t capacity() const noexcept { return inner_->capacity; }

   private:
    friend class SpscRing;
    explicit Consumer(std::shared_ptr<Inner> inner) : inner_(std::move(inner)) {}

    std::shared_ptr<Inner> inner_;
    std::size_t cached_tail_ = 0;
    std::size_t cached_head_ = 0;
  };
};

}  // namespace ferrox
