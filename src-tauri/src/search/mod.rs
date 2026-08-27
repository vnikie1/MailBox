//! Search. docs/01 §7, docs/06 Phase 9.
//!
//! Three layers, deliberately separate:
//!
//! * `query` parses what the user typed into a structured [`query::Query`]. Pure, and knows
//!   nothing about SQL or FTS5.
//! * `compile` turns that into a statement with bound parameters.
//! * `rank` orders what comes back.
//!
//! The split exists because the parser is where hostile input arrives and the compiler is where
//! SQL is assembled, and keeping one function that did both would put user text and statement
//! text in the same place — which is the shape every injection has.

pub mod compile;
pub mod query;
pub mod rank;
pub mod run;
pub mod suggest;
