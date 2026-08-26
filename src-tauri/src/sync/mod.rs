//! The IMAP sync engine. docs/03-architecture.md §5, docs/06 Phase 5.

pub mod backoff;
pub mod bodies;
pub mod engine;
pub mod envelope;
pub mod fetch;
pub mod mailboxes;
pub mod persist;
pub mod session;
pub mod threading;

#[cfg(test)]
mod threading_tests;
