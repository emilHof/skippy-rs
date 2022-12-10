//! A lock free skip list.
//!
//! The purpose of this crate is to provide a skip list that can be used in concurrent applications.
#![warn(
    // missing_debug_implementations,
    rust_2018_idioms,
    unreachable_pub
)]
pub mod concurrent_skiplist;
pub mod skiplist;
