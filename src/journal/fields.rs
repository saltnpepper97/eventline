use super::super::core::value::Value;
use std::collections::HashMap;

/// Key-value metadata attached to a log record.
///
/// ### Allocation strategy
///
/// The inner `HashMap` is heap-allocated lazily — plain `info!("…")` calls
/// pass `Fields::new()` which holds `None` and costs nothing beyond a word on
/// the stack.  The allocation only happens when `.insert()` is first called.
///
/// This matters because the overwhelming majority of log calls carry no
/// fields at all.
#[derive(Debug, Clone, Default)]
pub struct Fields {
    data: Option<HashMap<String, Value>>,
}

impl Fields {
    /// Create an empty `Fields`.  No heap allocation occurs.
    #[inline]
    pub fn new() -> Self {
        Self { data: None }
    }

    /// Insert a key-value pair.
    ///
    /// The inner `HashMap` is allocated on the first call.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.data
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.as_ref()?.get(key)
    }

    /// Iterate over all key-value pairs.  Returns nothing for empty `Fields`
    /// without touching the heap.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        // `Option<HashMap>` implements `IntoIterator` via the option combinators.
        self.data.iter().flat_map(|m| m.iter())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.as_ref().map_or(0, |m| m.len())
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.as_ref().map_or(true, |m| m.is_empty())
    }
}

impl FromIterator<(String, Value)> for Fields {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        let data: HashMap<String, Value> = iter.into_iter().collect();
        if data.is_empty() {
            Self { data: None }
        } else {
            Self { data: Some(data) }
        }
    }
}
