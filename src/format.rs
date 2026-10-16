use crate::core::Value;
use crate::journal::Fields;
use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Current JSONL format version. Files without a version are legacy version 0.
pub const JSONL_FORMAT_VERSION: u32 = 1;

pub(crate) fn new_run_id() -> String {
    static PROCESS: OnceLock<String> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let process = PROCESS.get_or_init(|| {
        let random = RandomState::new();
        format!("{:016x}{:016x}", random.hash_one(0u8), random.hash_one(1u8))
    });
    format!("{process}-{:x}", NEXT.fetch_add(1, Ordering::Relaxed))
}

pub(crate) fn default_run_id() -> &'static str {
    static RUN: OnceLock<String> = OnceLock::new();
    RUN.get_or_init(new_run_id)
}

// JSON has no nonfinite numbers. Preserve them as explicit strings in the
// versioned wire format while leaving the core Value serialization unchanged.
pub(crate) struct JsonFields<'a>(pub &'a Fields);
impl Serialize for JsonFields<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in self.0.iter() {
            map.serialize_entry(key, &JsonValue(value))?;
        }
        map.end()
    }
}
struct JsonValue<'a>(&'a Value);
impl Serialize for JsonValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Value::Float(value) = self.0
            && !value.is_finite()
        {
            let value = if value.is_nan() {
                "NaN"
            } else if value.is_sign_positive() {
                "Infinity"
            } else {
                "-Infinity"
            };
            let mut fields = serializer.serialize_struct("Value", 2)?;
            fields.serialize_field("type", "Float")?;
            fields.serialize_field("value", value)?;
            fields.end()
        } else {
            self.0.serialize(serializer)
        }
    }
}

pub(crate) fn deserialize_fields<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Fields, D::Error> {
    let values = std::collections::HashMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let mut fields = Fields::new();
    for (key, value) in values {
        let special = if value.get("type").and_then(serde_json::Value::as_str) == Some("Float") {
            match value.get("value").and_then(serde_json::Value::as_str) {
                Some("NaN") => Some(Value::Float(f64::NAN)),
                Some("Infinity") => Some(Value::Float(f64::INFINITY)),
                Some("-Infinity") => Some(Value::Float(f64::NEG_INFINITY)),
                _ => None,
            }
        } else {
            None
        };
        let value = match special {
            Some(value) => value,
            None => serde_json::from_value(value).map_err(serde::de::Error::custom)?,
        };
        fields.insert(key, value);
    }
    Ok(fields)
}
