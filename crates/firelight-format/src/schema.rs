//! JSON Schemas for every `.firelight/` YAML file (§5.6), generated from the
//! input types in [`crate::raw`]. Published under `schema/v1/` and checked
//! for drift in CI (`firelight schema`, `just schema`).

use schemars::{JsonSchema, generate::SchemaSettings};
use serde_json::{Value, json};

use crate::raw::*;

fn one<T: JsonSchema>(file: &str, extra: impl FnOnce(&mut Value)) -> (String, String) {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let mut v = serde_json::to_value(generator.into_root_schema_for::<T>()).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert(
            "$id".into(),
            json!(format!("https://firelight.oss.life.li/schema/v1/{file}")),
        );
        // `format` is always 1 in v1.
        if let Some(p) = o.get_mut("properties").and_then(Value::as_object_mut) {
            p.insert(
                "format".into(),
                json!({ "const": 1, "description": "The format version: always 1." }),
            );
        }
    }
    extra(&mut v);
    let mut s = serde_json::to_string_pretty(&v).unwrap_or_default();
    s.push('\n');
    (file.to_string(), s)
}

fn type_const(v: &mut Value, t: &str) {
    if let Some(p) = v.get_mut("properties").and_then(Value::as_object_mut) {
        p.insert("type".into(), json!({ "const": t }));
    }
}

/// `(file name, pretty JSON)` for every schema, in a stable order.
pub fn all() -> Vec<(String, String)> {
    vec![
        one::<RepoYaml>("repo.json", |_| {}),
        one::<LinksYaml>("links.json", |_| {}),
        one::<CardsIslandYaml>("island-cards.json", |v| type_const(v, "cards")),
        one::<LinksIslandYaml>("island-links.json", |v| type_const(v, "links")),
        one::<TemplateIslandYaml>("island-template.json", |v| type_const(v, "template")),
        one::<CardsContributionYaml>("island-cards-contribution.json", |v| {
            type_const(v, "cards-contribution")
        }),
        one::<EntryYaml>("entry.json", |_| {}),
        one::<GoldenYaml>("golden.json", |_| {}),
        one::<SpacesYaml>("spaces.json", |_| {}),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn schemas_are_json_with_ids() {
        for (name, text) in super::all() {
            let v: serde_json::Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(v["$id"].as_str().is_some_and(|s| s.ends_with(&name)));
            assert_eq!(v["properties"]["format"]["const"], 1, "{name}");
            assert_eq!(
                v["additionalProperties"], false,
                "{name}: unknown keys are errors"
            );
        }
    }
}
