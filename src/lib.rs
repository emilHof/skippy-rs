//! A lock free skip list.
//!
//! The purpose of this crate is to provide a skip list that can be used in concurrent applications.
#![warn(
    // missing_debug_implementations,
    rust_2018_idioms,
    unreachable_pub
)]
pub mod collections;
pub mod internal;
pub mod skiplist;

pub use collections::priority_queue::PriorityQueue;
pub use internal::skiplist::SkipList;
pub use internal::sync::SkipList as SSkipList;
