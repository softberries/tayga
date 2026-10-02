//! Pure trace analysis: no I/O, no clocks.

pub mod model;
pub mod rootcause;
pub mod tree;

#[cfg(test)]
pub(crate) mod testutil;
