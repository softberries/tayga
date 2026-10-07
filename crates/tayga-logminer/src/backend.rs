//! `logminer.fingerprinter`: the fingerprint backend in front of Drain (sub-project 4 spec §3.6).

use std::str::FromStr;
use std::sync::Arc;
use tayga_drain::fingerprint::{BatchFingerprinter, ScalarFingerprinter};
use tayga_drain::parallel::ParallelFingerprinter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// No cache: every line through the Drain tree (the kill switch).
    Off,
    Scalar,
    Parallel,
    Gpu,
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "off" => Ok(Self::Off),
            "scalar" => Ok(Self::Scalar),
            "parallel" => Ok(Self::Parallel),
            "gpu" => Ok(Self::Gpu),
            other => Err(format!(
                "unknown fingerprinter {other:?}: expected off, scalar, parallel or gpu"
            )),
        }
    }
}

impl Backend {
    /// `gpu` needs a build with the `gpu` feature (Docker images have none).
    pub fn check_build(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self != Self::Gpu || cfg!(feature = "gpu"),
            "logminer.fingerprinter = \"gpu\" needs a build with the gpu feature"
        );
        Ok(())
    }

    /// The fingerprinter (`None` for `off`) and the name of the backend in use: `gpu` falls back
    /// to `scalar` without a usable adapter.
    pub fn build(self) -> (Option<Arc<dyn BatchFingerprinter>>, &'static str) {
        match self {
            Self::Off => (None, "off"),
            Self::Scalar => (Some(Arc::new(ScalarFingerprinter)), "scalar"),
            Self::Parallel => (Some(Arc::new(ParallelFingerprinter)), "parallel"),
            Self::Gpu => gpu(),
        }
    }
}

#[cfg(feature = "gpu")]
fn gpu() -> (Option<Arc<dyn BatchFingerprinter>>, &'static str) {
    match tayga_drain::gpu::GpuFingerprinter::new() {
        Some(g) => {
            tracing::info!(adapter = g.adapter(), "GPU fingerprinter");
            (Some(Arc::new(g)), "gpu")
        }
        None => {
            tracing::warn!("no usable GPU adapter: fingerprinting on the CPU (scalar)");
            (Some(Arc::new(ScalarFingerprinter)), "scalar")
        }
    }
}

/// Without the feature `check_build` refuses `gpu` at startup; scalar keeps `build` total.
#[cfg(not(feature = "gpu"))]
fn gpu() -> (Option<Arc<dyn BatchFingerprinter>>, &'static str) {
    (Some(Arc::new(ScalarFingerprinter)), "scalar")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_four_backends_and_names_the_allowed_ones() {
        for (s, b) in [
            ("off", Backend::Off),
            ("scalar", Backend::Scalar),
            ("parallel", Backend::Parallel),
            ("gpu", Backend::Gpu),
        ] {
            assert_eq!(s.parse::<Backend>(), Ok(b));
        }
        let e = "simd".parse::<Backend>().unwrap_err();
        assert!(e.contains("off, scalar, parallel or gpu"), "{e}");
    }

    #[test]
    fn off_builds_nothing_and_the_cpu_backends_build_themselves() {
        assert!(Backend::Off.build().0.is_none());
        for (b, name) in [(Backend::Scalar, "scalar"), (Backend::Parallel, "parallel")] {
            let (f, used) = b.build();
            assert_eq!((f.map(|f| f.name()), used), (Some(name), name));
        }
    }

    #[cfg(not(feature = "gpu"))]
    #[test]
    fn gpu_needs_the_feature() {
        let e = Backend::Gpu.check_build().unwrap_err().to_string();
        assert!(e.contains("needs a build with the gpu feature"), "{e}");
        assert!(Backend::Scalar.check_build().is_ok());
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_uses_the_adapter_or_falls_back_to_scalar() {
        assert!(Backend::Gpu.check_build().is_ok());
        let (f, used) = Backend::Gpu.build();
        assert_eq!(f.map(|f| f.name()), Some(used));
        assert!(used == "gpu" || used == "scalar", "{used}");
    }
}
