//! Rules and Smart Mailboxes. docs/01 §8, docs/06 Phase 8.
//!
//! Both are the same question asked from two directions — "which stored messages match?" and
//! "does this arriving message match?" — so they share one predicate engine. docs/06 makes that
//! a requirement rather than a preference, and `predicate.rs` explains why at length.

pub mod predicate;

#[cfg(test)]
mod predicate_tests;
