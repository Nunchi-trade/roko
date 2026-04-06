//! Pointer lifecycle management: GC policy for evicting stale pointers.

pub mod gc;

pub use gc::{PointerGcPolicy, PointerMeta};
