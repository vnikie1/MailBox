//! Mail presentation: turning stored messages into something safe to show.
//!
//! Separate from `sync`, which is about getting mail *in*. This module is about what happens
//! to it on the way out — and the security boundary docs/03 §6 draws lives here.

pub mod detect;
pub mod render;
