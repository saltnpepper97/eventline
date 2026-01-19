use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Value {
    String(String),
    Int(i64),
    Uint(u64),
    Float(f64),
    Bool(bool),
    DurationMs(u64),
    Null,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::String(_) => "string",
            Value::Int(_) => "int",
            Value::Uint(_) => "uint",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::DurationMs(_) => "duration_ms",
            Value::Null => "null",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => f.write_str(s),
            Value::Int(v) => write!(f, "{v}"),
            Value::Uint(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::DurationMs(ms) => write!(f, "{ms}ms"),
            Value::Null => f.write_str("null"),
        }
    }
}

// Conversions

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Value::Uint(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<std::time::Duration> for Value {
    fn from(v: std::time::Duration) -> Self {
        Value::DurationMs(v.as_millis() as u64)
    }
}
