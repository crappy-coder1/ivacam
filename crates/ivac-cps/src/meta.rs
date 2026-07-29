//! Post metadata — what `inspect_post` extracts from a `.cps` top-level
//! eval so UIs can render a properties form without running a program.
//!
//! These types cross the transport boundary (server `/posts` routes,
//! tauri/wasm inspect commands land in cps.8), so they carry schemars
//! derives and stable camelCase JSON like the pipeline wire types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Identity + property sheet of one post script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PostMeta {
    /// The post's `description` global (display name).
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub vendor: String,
    /// Output file extension the post declares (e.g. `"nc"`).
    #[serde(default)]
    pub extension: String,
    /// `CAPABILITY_*` bitmask from the post's `capabilities` global.
    #[serde(default)]
    pub capabilities: u32,
    /// User-tunable properties in declaration order.
    #[serde(default)]
    pub properties: Vec<PropertyMeta>,
}

/// One entry of the post's `properties` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PropertyMeta {
    /// The key in the `properties` object — what overrides address.
    pub name: String,
    /// Human-facing label (`title` in the post, falls back to `name`).
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub kind: PropertyKind,
    pub default: PropertyValue,
}

/// Control shape for one property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PropertyKind {
    Bool,
    Number,
    Integer,
    Enum { values: Vec<EnumValueMeta> },
    String,
}

/// One selectable value of an enum property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnumValueMeta {
    /// Stored value (`id` in the post's `values` list).
    pub id: String,
    /// Display label.
    pub title: String,
}

/// A property's default (and override) payload. Untagged: the JSON is
/// the bare primitive, mirroring ivac-core's `CpsParamValue` wire type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PropertyValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin the JSON shape the frontend form generator (cps.9) consumes.
    #[test]
    fn wire_shape() {
        let meta = PostMeta {
            description: "Generic FANUC".into(),
            vendor: "ivacam".into(),
            extension: "nc".into(),
            capabilities: 1,
            properties: vec![
                PropertyMeta {
                    name: "useRadius".into(),
                    title: "Radius arcs".into(),
                    description: "Use R instead of IJK".into(),
                    kind: PropertyKind::Bool,
                    default: PropertyValue::Bool(false),
                },
                PropertyMeta {
                    name: "safePositionMethod".into(),
                    title: "Safe retracts".into(),
                    description: String::new(),
                    kind: PropertyKind::Enum {
                        values: vec![EnumValueMeta {
                            id: "G28".into(),
                            title: "G28".into(),
                        }],
                    },
                    default: PropertyValue::Text("G28".into()),
                },
            ],
        };
        let json = serde_json::to_value(&meta).expect("serialize");
        assert_eq!(json["properties"][0]["kind"]["type"], "bool");
        assert_eq!(json["properties"][0]["default"], false);
        assert_eq!(json["properties"][1]["kind"]["type"], "enum");
        assert_eq!(json["properties"][1]["kind"]["values"][0]["id"], "G28");
        let back: PostMeta = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, meta);
    }
}
