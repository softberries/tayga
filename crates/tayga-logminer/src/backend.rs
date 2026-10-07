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
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "off" => Ok(Self::Off),
            "scalar" => Ok(Self::Scalar),
            "parallel" => Ok(Self::Parallel),
            other => Err(format!(
                "unknown fingerprinter {other:?}: expected off, scalar or parallel"
            )),
        }
    }
}

impl Backend {
    /// The fingerprinter (`None` for `off`) and the name of the backend in use.
    pub fn build(self) -> (Option<Arc<dyn BatchFingerprinter>>, &'static str) {
        match self {
            Self::Off => (None, "off"),
            Self::Scalar => (Some(Arc::new(ScalarFingerprinter)), "scalar"),
            Self::Parallel => (Some(Arc::new(ParallelFingerprinter)), "parallel"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_backends_and_names_the_allowed_ones() {
        for (s, b) in [
            ("off", Backend::Off),
            ("scalar", Backend::Scalar),
            ("parallel", Backend::Parallel),
        ] {
            assert_eq!(s.parse::<Backend>(), Ok(b));
        }
        let e = "simd".parse::<Backend>().unwrap_err();
        assert!(e.contains("off, scalar or parallel"), "{e}");
    }

    #[test]
    fn off_builds_nothing_and_the_cpu_backends_build_themselves() {
        assert!(Backend::Off.build().0.is_none());
        for (b, name) in [(Backend::Scalar, "scalar"), (Backend::Parallel, "parallel")] {
            let (f, used) = b.build();
            assert_eq!((f.map(|f| f.name()), used), (Some(name), name));
        }
    }
}
