//! Pure trace analysis: no I/O, no clocks.

pub mod baseline;
pub mod critical_path;
pub mod fingerprint;
pub mod model;
pub mod rootcause;
pub mod story;
pub mod summary;
pub mod tree;

#[cfg(test)]
pub(crate) mod testutil;
