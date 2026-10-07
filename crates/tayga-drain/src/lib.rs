//! Log template mining (Drain) and alert rules. Pure: no I/O.

pub mod detect;
pub mod drain;
pub mod fingerprint;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod parallel;
pub mod preprocess;
