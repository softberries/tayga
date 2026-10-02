//! Edits the flagd JSON file that the demo's flagd watches.

use anyhow::{Context, anyhow, bail};
use std::path::Path;

pub fn set_flag(path: &Path, name: &str, variant: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut doc: serde_json::Value = serde_json::from_str(&text)?;
    let flag = doc
        .get_mut("flags")
        .and_then(|f| f.get_mut(name))
        .ok_or_else(|| anyhow!("unknown flag {name}"))?;
    let variants = flag
        .get("variants")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("flag {name} has no variants"))?;
    if !variants.contains_key(variant) {
        let available: Vec<&str> = variants.keys().map(String::as_str).collect();
        bail!("flag {name} has no variant {variant}; available: {}", available.join(", "));
    }
    flag["defaultVariant"] = serde_json::Value::String(variant.to_owned());
    // Written in place (not rename) so flagd's file watcher on the bind mount sees the change.
    std::fs::write(path, serde_json::to_string_pretty(&doc)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"{"$schema":"x","flags":{"paymentFailure":{"description":"d","state":"ENABLED","variants":{"100%":1,"off":0},"defaultVariant":"off"}}}"#;

    fn write_tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("tayga-flags-{name}-{}.json", std::process::id()));
        std::fs::write(&p, DOC).unwrap();
        p
    }

    #[test]
    fn sets_default_variant_and_preserves_other_fields() {
        let p = write_tmp("ok");
        set_flag(&p, "paymentFailure", "100%").unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["flags"]["paymentFailure"]["defaultVariant"], "100%");
        assert_eq!(v["flags"]["paymentFailure"]["description"], "d");
        assert_eq!(v["$schema"], "x");
    }

    #[test]
    fn rejects_unknown_flag() {
        let p = write_tmp("unknown");
        let err = set_flag(&p, "nope", "on").unwrap_err().to_string();
        assert!(err.contains("unknown flag nope"), "{err}");
    }

    #[test]
    fn rejects_unknown_variant_and_lists_available() {
        let p = write_tmp("variant");
        let err = set_flag(&p, "paymentFailure", "37%").unwrap_err().to_string();
        assert!(err.contains("100%") && err.contains("off"), "{err}");
    }
}
