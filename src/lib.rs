//! Library surface for `cmdtrail`. Exists so `examples/storage_bench.rs`
//! (and any future tooling) can call the exact same `db`/`rank` code the
//! `cmdtrail` binary ships, instead of a parallel reimplementation that
//! could quietly drift from what's actually running in production.

pub mod db;
pub mod duration;
pub mod git;
pub mod ignore;
pub mod picker;
pub mod rank;
