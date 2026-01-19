use super::value::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Fields {
    data: HashMap<String, Value>,
}

impl Fields {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.data.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.data.iter()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl FromIterator<(String, Value)> for Fields {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        Fields {
            data: iter.into_iter().collect(),
        }
    }
}
