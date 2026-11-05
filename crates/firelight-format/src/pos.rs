//! [`P<T>`]: a value with the line and column it came from.
//!
//! Deserializes through `serde_saphyr::Spanned`, and has the JSON Schema of
//! `T`, so the YAML input types keep both their positions and a clean
//! generated schema.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct P<T> {
    pub value: T,
    pub line: u32,
    pub col: u32,
}

impl<T> P<T> {
    pub fn new(value: T, line: u32, col: u32) -> Self {
        Self { value, line, col }
    }

    /// The same position with another value.
    pub fn map<U>(&self, value: U) -> P<U> {
        P {
            value,
            line: self.line,
            col: self.col,
        }
    }

    /// Shift by the lines before the YAML (an entry's front matter starts on
    /// line 2 of its file).
    pub fn shift(&mut self, lines: u32) {
        if self.line > 0 {
            self.line += lines;
        }
    }
}

impl<T> std::ops::Deref for P<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for P<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = serde_saphyr::Spanned::<T>::deserialize(d)?;
        let loc = s.referenced;
        Ok(P {
            value: s.value,
            line: u32::try_from(loc.line()).unwrap_or(u32::MAX),
            col: u32::try_from(loc.column()).unwrap_or(u32::MAX),
        })
    }
}

impl<T: JsonSchema> JsonSchema for P<T> {
    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        T::schema_id()
    }

    fn inline_schema() -> bool {
        T::inline_schema()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        T::json_schema(generator)
    }
}
