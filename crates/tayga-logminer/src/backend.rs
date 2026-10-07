//! `logminer.fingerprinter`: the fingerprint backend in front of Drain (sub-project 4 spec §3.6).

use std::str::FromStr;
use std::sync::Arc;
use tayga_drain::fingerprint::{BatchFingerprinter, ScalarFingerprinter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// No cache: every line through the Drain tree (the kill switch).
    Off,
    Scalar,
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "off" => Ok(Self::Off),
            "scalar" => Ok(Self::Scalar),
            other => Err(format!(
                "unknown fingerprinter {other:?}: expected off or scalar"
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_backends_and_names_the_allowed_ones() {
        assert_eq!("off".parse::<Backend>(), Ok(Backend::Off));
        assert_eq!("scalar".parse::<Backend>(), Ok(Backend::Scalar));
        let e = "simd".parse::<Backend>().unwrap_err();
        assert!(e.contains("off or scalar"), "{e}");
    }

    #[test]
    fn off_builds_nothing_and_scalar_builds_itself() {
        assert!(Backend::Off.build().0.is_none());
        let (f, used) = Backend::Scalar.build();
        assert_eq!((f.map(|f| f.name()), used), (Some("scalar"), "scalar"));
    }
}
