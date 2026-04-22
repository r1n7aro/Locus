use std::collections::HashMap;

use regex::Regex;
use std::sync::OnceLock;

use crate::diff::types::*;

// ── Shader property parsing ──

#[derive(Debug, Clone)]
pub(crate) struct ShaderProperty {
    pub(crate) name: String,
    pub(crate) display_name: String,
    pub(crate) prop_type: String,
    pub(crate) order: usize,
}

fn shader_property_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Matches: [optional attributes] _Name ("Display Name", Type) = ...
        // Also matches without attributes: _Name ("Display Name", Type) = ...
        Regex::new(
            r#"(?m)^\s*(?:\[[^\]]+\]\s*)*(\w+)\s*\(\s*"([^"]*)"\s*,\s*((?:[^()]|\([^)]*\))+)\)\s*="#,
        )
        .unwrap()
    })
}

fn properties_block_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Match "Properties" followed by balanced braces (simple nesting)
        Regex::new(r"(?s)Properties\s*\{").unwrap()
    })
}

/// Parse a Unity shader source to extract the Properties block.
/// Returns a list of properties in definition order.
pub(crate) fn parse_shader_properties(content: &str) -> Vec<ShaderProperty> {
    // Find the Properties { ... } block
    let Some(prop_match) = properties_block_regex().find(content) else {
        return Vec::new();
    };
    let start = prop_match.end();
    // Find the matching closing brace (handle nesting)
    let mut depth = 1i32;
    let mut end = start;
    for (i, ch) in content[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Vec::new();
    }

    let block = &content[start..end];
    let mut props = Vec::new();
    for caps in shader_property_regex().captures_iter(block) {
        let name = caps
            .get(1)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let display_name = caps
            .get(2)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let raw_type = caps
            .get(3)
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();
        let prop_type = normalize_shader_type(&raw_type);
        props.push(ShaderProperty {
            name,
            display_name,
            prop_type,
            order: props.len(),
        });
    }
    props
}

/// Normalize shader property types to friendly display names.
fn normalize_shader_type(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("Range") {
        return "Float".into();
    }
    match trimmed {
        "2D" | "2DArray" => "Texture".into(),
        "3D" => "Texture3D".into(),
        "Cube" | "CubeArray" => "Cubemap".into(),
        "Color" => "Color".into(),
        "Vector" => "Vector".into(),
        "Float" => "Float".into(),
        "Int" | "Integer" => "Int".into(),
        other => other.to_string(),
    }
}

// ── Material field restructuring ──

/// Material serialization groups that hold shader properties.
const MATERIAL_PROP_GROUPS: &[(&str, &str)] = &[
    ("m_Colors", "Color"),
    ("m_Floats", "Float"),
    ("m_Ints", "Int"),
    ("m_TexEnvs", "Texture"),
];

/// Restructure a material's inspector fields:
/// 1. Move "Saved Properties" to the front
/// 2. Flatten type-based groups (m_Colors, m_Floats, m_TexEnvs, m_Ints)
/// 3. Order by shader property definition order
/// 4. Add property type annotations (fieldType)
pub(crate) fn restructure_material_fields(
    fields: &mut Vec<InspectorField>,
    shader_props: &[ShaderProperty],
) {
    // Find the "m_SavedProperties" field
    let Some(sp_idx) = fields
        .iter()
        .position(|f| f.property_path == "m_SavedProperties")
    else {
        return;
    };

    let saved_props = &mut fields[sp_idx];

    // Build shader order lookup: property name → (order, type, display_name)
    let shader_lookup: HashMap<&str, (&ShaderProperty,)> = shader_props
        .iter()
        .map(|p| (p.name.as_str(), (p,)))
        .collect();

    // Collect all property items from type groups
    let mut flat_items: Vec<(usize, String, InspectorField)> = Vec::new();
    // Also track which children are type groups so we can remove them
    let mut non_group_children: Vec<InspectorField> = Vec::new();

    for child in saved_props.children.drain(..) {
        let group_type = MATERIAL_PROP_GROUPS
            .iter()
            .find(|(name, _)| child.property_path.ends_with(name));

        if let Some((_, fallback_type)) = group_type {
            // This is a type group (m_Colors, m_Floats, etc.)
            // Extract individual properties from it
            for item in child.children {
                // When list matching collapsed the item, item.label is
                // already the property name. When it didn't run (newly
                // added file), the item is still [N] with a single child
                // — unwrap it to flatten the display.
                let (mut target, prop_name) =
                    if item.label.starts_with('[') && item.children.len() == 1 {
                        let mut children = item.children;
                        let inner = children.swap_remove(0);
                        let raw = inner
                            .property_path
                            .rsplit('.')
                            .next()
                            .unwrap_or(&inner.label)
                            .to_string();
                        (inner, raw)
                    } else {
                        let name = item.label.clone();
                        (item, name)
                    };

                let (order, prop_type, display_name) =
                    if let Some((sp,)) = shader_lookup.get(prop_name.as_str()) {
                        (
                            sp.order,
                            sp.prop_type.clone(),
                            Some(sp.display_name.clone()),
                        )
                    } else {
                        (usize::MAX, fallback_type.to_string(), None)
                    };

                target.field_type = Some(prop_type);
                if let Some(dn) = display_name {
                    if !dn.is_empty() {
                        target.label = dn;
                    }
                }

                flat_items.push((order, prop_name, target));
            }
        } else if child.property_path.ends_with("serializedVersion") {
            // Skip serializedVersion — internal detail
            continue;
        } else {
            non_group_children.push(child);
        }
    }

    // Sort by shader definition order, then alphabetically for unmatched
    flat_items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    // Rebuild children: flattened properties first, then any non-group items
    saved_props.children = flat_items
        .into_iter()
        .map(|(_, _, field)| field)
        .chain(non_group_children)
        .collect();
    saved_props.label = "Saved Properties".into();

    // Move m_SavedProperties to the front (index 0)
    if sp_idx > 0 {
        let saved = fields.remove(sp_idx);
        fields.insert(0, saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_shader_properties() {
        let shader_source = r#"
Shader "Custom/MyShader"
{
    Properties
    {
        [HDR] _Color ("Main Color", Color) = (1,1,1,1)
        _MainTex ("Albedo", 2D) = "white" {}
        _Glossiness ("Smoothness", Range(0, 1)) = 0.5
        _Metallic ("Metallic", Float) = 0.0
        [Toggle] _UseEmission ("Use Emission", Float) = 0
        _BumpMap ("Normal Map", 2D) = "bump" {}
        _Mode ("Rendering Mode", Int) = 0
    }
    SubShader { }
}
"#;
        let props = parse_shader_properties(shader_source);
        assert_eq!(props.len(), 7);
        assert_eq!(props[0].name, "_Color");
        assert_eq!(props[0].display_name, "Main Color");
        assert_eq!(props[0].prop_type, "Color");
        assert_eq!(props[0].order, 0);

        assert_eq!(props[1].name, "_MainTex");
        assert_eq!(props[1].prop_type, "Texture");

        assert_eq!(props[2].name, "_Glossiness");
        assert_eq!(props[2].prop_type, "Float"); // Range → Float

        assert_eq!(props[6].name, "_Mode");
        assert_eq!(props[6].prop_type, "Int");
    }

    #[test]
    fn test_parse_shader_empty() {
        let props = parse_shader_properties("no properties block here");
        assert!(props.is_empty());
    }
}
