//! Edits the flagd JSON file that the demo's flagd watches.

use anyhow::{Context, anyhow, bail};
use std::path::Path;

/// Sets `defaultVariant` to `variant`. flagd evaluates `targeting` before `defaultVariant`, so a
/// flag whose targeting is a top-level `{"if": [condition, then, else]}` with a string `then`
/// (as the demo's `productCatalogFailure`) also gets `then` set to `variant`; condition and
/// `else` are left untouched. Other targeting shapes are not modified.
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
        bail!(
            "flag {name} has no variant {variant}; available: {}",
            available.join(", ")
        );
    }
    if let Some(then) = flag
        .get_mut("targeting")
        .and_then(|t| t.get_mut("if"))
        .and_then(|i| i.as_array_mut())
        .filter(|a| a.len() == 3 && a[1].is_string())
        .map(|a| &mut a[1])
    {
        *then = serde_json::Value::String(variant.to_owned());
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
        let p =
            std::env::temp_dir().join(format!("tayga-flags-{name}-{}.json", std::process::id()));
        std::fs::write(&p, DOC).unwrap();
        p
    }

    #[test]
    fn sets_default_variant_and_preserves_other_fields() {
        let p = write_tmp("ok");
        set_flag(&p, "paymentFailure", "100%").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["flags"]["paymentFailure"]["defaultVariant"], "100%");
        assert_eq!(v["flags"]["paymentFailure"]["description"], "d");
        assert_eq!(v["$schema"], "x");
    }

    #[test]
    fn sets_then_branch_of_targeting_if() {
        let doc = r#"{"flags":{"f":{"state":"ENABLED","variants":{"on":true,"off":false},"defaultVariant":"off","targeting":{"if":[{"==":[{"var":"product_id"},"X"]},"off","off"]}}}}"#;
        let p =
            std::env::temp_dir().join(format!("tayga-flags-targeting-{}.json", std::process::id()));
        std::fs::write(&p, doc).unwrap();
        set_flag(&p, "f", "on").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let f = &v["flags"]["f"];
        assert_eq!(f["defaultVariant"], "on");
        assert_eq!(f["targeting"]["if"][1], "on");
        assert_eq!(f["targeting"]["if"][2], "off");
        assert_eq!(
            f["targeting"]["if"][0],
            serde_json::json!({"==": [{"var": "product_id"}, "X"]})
        );
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
        let err = set_flag(&p, "paymentFailure", "37%")
            .unwrap_err()
            .to_string();
        assert!(err.contains("100%") && err.contains("off"), "{err}");
    }
}
