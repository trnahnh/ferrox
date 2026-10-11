//! The real `ferrox::ring` module, included by path, with no other code and no
//! dependencies. Exists so that tools which rebuild the standard library or
//! interpret every instruction (ThreadSanitizer via `-Zbuild-std`, Miri) only
//! have to process the ring and `std`, not criterion/proptest/serde.
//!
//! The `loom` cfg is not meant to be used here (that build lives in the main
//! crate, `tests/loom_ring.rs`); the `#[path]` include is otherwise identical.
#![allow(dead_code)]

#[path = "../../../src/ring.rs"]
pub mod ring;
