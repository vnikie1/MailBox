//! The IMAP sync engine. docs/03-architecture.md §5, docs/06 Phase 5.

pub mod backoff;
pub mod bodies;
pub mod engine;
pub mod envelope;
pub mod events;
pub mod fetch;
pub mod idle;
pub mod mailboxes;
pub mod ops;
pub mod persist;
pub mod session;
pub mod threading;

#[cfg(test)]
mod threading_tests;
