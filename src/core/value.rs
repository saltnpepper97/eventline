//! Structured field values for events.
//!
//! This module provides a type-safe way to attach structured data to events,
//! enabling better querying, filtering, and analysis.

use serde::{Serialize, Deserialize};
use std::collections::BTreeMap;
use std::fmt;

/// A strongly-typed value that can be attached to an event as a field.
///
/// Values are designed to be:
/// - Serializable (for JSON/binary output)
/// - Comparable (for filtering/querying)
/// - Human-readable (for text rendering)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// String value
    String(String),
    /// Signed integer
    Int(i64),
    /// Unsigned integer
    Uint(u64),
    /// Floating point number
    Float(f64),
    /// Boolean value
    Bool(bool),
    /// Duration in milliseconds
    Duration(u64),
    /// Null/absent value
    Null,
}

impl Value {
    /// Returns true if this value is considered "truthy" for filtering.
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Uint(u) => *u != 0,
            Value::Float(f) => *f != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Duration(d) => *d != 0,
            Value::Null => false,
        }
    }

    /// Get the type name of this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::String(_) => "string",
            Value::Int(_) => "int",
            Value::Uint(_) => "uint",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Duration(_) => "duration",
            Value::Null => "null",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => write!(f, "{}", s),
            Value::Int(i) => write!(f, "{}", i),
            Value::Uint(u) => write!(f, "{}", u),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Duration(ms) => {
                if *ms < 1000 {
                    write!(f, "{}ms", ms)
                } else if *ms < 60_000 {
                    write!(f, "{:.2}s", *ms as f64 / 1000.0)
                } else {
                    write!(f, "{:.2}m", *ms as f64 / 60_000.0)
                }
            }
            Value::Null => write!(f, "null"),
        }
    }
}

// Conversion traits for ergonomic API
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Value::Int(i as i64)
    }
}

impl From<u64> for Value {
    fn from(u: u64) -> Self {
        Value::Uint(u)
    }
}

impl From<u32> for Value {
    fn from(u: u32) -> Self {
        Value::Uint(u as u64)
    }
}

impl From<usize> for Value {
    fn from(u: usize) -> Self {
        Value::Uint(u as u64)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}

impl From<f32> for Value {
    fn from(f: f32) -> Self {
        Value::Float(f as f64)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<std::time::Duration> for Value {
    fn from(d: std::time::Duration) -> Self {
        Value::Duration(d.as_millis() as u64)
    }
}

/// A collection of named fields attached to an event.
///
/// Fields are stored in a `BTreeMap` to ensure consistent ordering
/// for both rendering and serialization.
pub type Fields = BTreeMap<String, Value>;

/// Trait for types that can be converted into a Fields collection.
///
/// This allows for flexible field construction APIs.
pub trait IntoFields {
    fn into_fields(self) -> Fields;
}

impl IntoFields for Fields {
    fn into_fields(self) -> Fields {
        self
    }
}

impl IntoFields for Vec<(String, Value)> {
    fn into_fields(self) -> Fields {
        self.into_iter().collect()
    }
}

impl<const N: usize> IntoFields for [(String, Value); N] {
    fn into_fields(self) -> Fields {
        self.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_display() {
        assert_eq!(Value::String("test".into()).to_string(), "test");
        assert_eq!(Value::Int(42).to_string(), "42");
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Duration(500).to_string(), "500ms");
        assert_eq!(Value::Duration(1500).to_string(), "1.50s");
    }

    #[test]
    fn test_value_conversions() {
        assert_eq!(Value::from("test"), Value::String("test".into()));
        assert_eq!(Value::from(42i32), Value::Int(42));
        assert_eq!(Value::from(42u64), Value::Uint(42));
        assert_eq!(Value::from(true), Value::Bool(true));
    }

    #[test]
    fn test_fields_ordering() {
        let mut fields = Fields::new();
        fields.insert("zebra".into(), Value::Int(3));
        fields.insert("alpha".into(), Value::Int(1));
        fields.insert("beta".into(), Value::Int(2));

        let keys: Vec<_> = fields.keys().collect();
        assert_eq!(keys, vec!["alpha", "beta", "zebra"]);
    }
}
