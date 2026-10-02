use crate::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
use crate::otlp::resource::v1::Resource;

pub const UNKNOWN_SERVICE: &str = "unknown_service";

/// Flattens an OTLP value to a string: scalars as-is, bytes as hex,
/// arrays and kvlists as JSON.
pub fn any_value_to_string(v: &AnyValue) -> String {
    match &v.value {
        Some(Value::StringValue(s)) => s.clone(),
        Some(Value::BoolValue(b)) => b.to_string(),
        Some(Value::IntValue(i)) => i.to_string(),
        Some(Value::DoubleValue(d)) => d.to_string(),
        Some(Value::BytesValue(b)) => hex::encode(b),
        Some(Value::ArrayValue(a)) => {
            let items: Vec<String> = a.values.iter().map(any_value_to_string).collect();
            serde_json::to_string(&items).unwrap_or_default()
        }
        Some(Value::KvlistValue(kv)) => {
            let map: serde_json::Map<String, serde_json::Value> = kv
                .values
                .iter()
                .map(|p| {
                    let s = p
                        .value
                        .as_ref()
                        .map(any_value_to_string)
                        .unwrap_or_default();
                    (p.key.clone(), serde_json::Value::String(s))
                })
                .collect();
            serde_json::Value::Object(map).to_string()
        }
        // Dictionary-encoded strings are only used by the profiles signal.
        Some(Value::StringValueStrindex(_)) | None => String::new(),
    }
}

pub fn attrs_to_pairs(attrs: &[KeyValue]) -> Vec<(String, String)> {
    attrs
        .iter()
        .map(|kv| {
            (
                kv.key.clone(),
                kv.value
                    .as_ref()
                    .map(any_value_to_string)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

pub fn service_name(resource: Option<&Resource>) -> String {
    resource
        .and_then(|r| r.attributes.iter().find(|kv| kv.key == "service.name"))
        .and_then(|kv| kv.value.as_ref())
        .map(any_value_to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| UNKNOWN_SERVICE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::common::v1::{ArrayValue, KeyValueList};

    fn kv(key: &str, value: Value) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: Some(AnyValue { value: Some(value) }),
            ..Default::default()
        }
    }

    #[test]
    fn scalars() {
        assert_eq!(
            any_value_to_string(&AnyValue {
                value: Some(Value::StringValue("a".into()))
            }),
            "a"
        );
        assert_eq!(
            any_value_to_string(&AnyValue {
                value: Some(Value::IntValue(-3))
            }),
            "-3"
        );
        assert_eq!(
            any_value_to_string(&AnyValue {
                value: Some(Value::BoolValue(true))
            }),
            "true"
        );
        assert_eq!(
            any_value_to_string(&AnyValue {
                value: Some(Value::DoubleValue(1.5))
            }),
            "1.5"
        );
        assert_eq!(
            any_value_to_string(&AnyValue {
                value: Some(Value::BytesValue(vec![0xff]))
            }),
            "ff"
        );
        assert_eq!(any_value_to_string(&AnyValue { value: None }), "");
    }

    #[test]
    fn composites_are_json() {
        let arr = AnyValue {
            value: Some(Value::ArrayValue(ArrayValue {
                values: vec![
                    AnyValue {
                        value: Some(Value::StringValue("x".into())),
                    },
                    AnyValue {
                        value: Some(Value::IntValue(2)),
                    },
                ],
            })),
        };
        assert_eq!(any_value_to_string(&arr), r#"["x","2"]"#);
        let map = AnyValue {
            value: Some(Value::KvlistValue(KeyValueList {
                values: vec![kv("k", Value::StringValue("v".into()))],
            })),
        };
        assert_eq!(any_value_to_string(&map), r#"{"k":"v"}"#);
    }

    #[test]
    fn service_name_from_resource_or_default() {
        let r = Resource {
            attributes: vec![kv("service.name", Value::StringValue("checkout".into()))],
            ..Default::default()
        };
        assert_eq!(service_name(Some(&r)), "checkout");
        assert_eq!(service_name(Some(&Resource::default())), UNKNOWN_SERVICE);
        assert_eq!(service_name(None), UNKNOWN_SERVICE);
    }

    #[test]
    fn pairs_keep_order() {
        let p = attrs_to_pairs(&[
            kv("a", Value::IntValue(1)),
            kv("b", Value::StringValue("x".into())),
        ]);
        assert_eq!(
            p,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "x".to_string())
            ]
        );
    }
}
