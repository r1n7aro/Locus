use std::path::Path;

pub(crate) const BUILTIN_TAGS: &[&str] = &[
    "Untagged",
    "Respawn",
    "Finish",
    "EditorOnly",
    "MainCamera",
    "Player",
    "GameController",
];

const UNITY_LAYER_COUNT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TagManagerConfig {
    pub custom_tags: Vec<String>,
    /// Unity layer slots in their serialized order. Empty slots remain present
    /// so every named layer keeps the same index as `GameObject.layer`.
    pub layer_slots: Vec<Option<String>>,
}

impl TagManagerConfig {
    /// Only named layers are exposed to prompt and validation consumers. The
    /// index still comes from the full 32-slot table.
    pub fn named_layers(&self) -> impl Iterator<Item = (usize, &str)> {
        self.layer_slots
            .iter()
            .enumerate()
            .filter_map(|(index, name)| name.as_deref().map(|name| (index, name)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequenceSection {
    None,
    Tags,
    Layers,
}

pub(crate) fn load_tag_manager(project_dir: &Path) -> std::io::Result<TagManagerConfig> {
    let path = project_dir.join("ProjectSettings").join("TagManager.asset");
    let content = std::fs::read_to_string(path)?;
    Ok(parse_tag_manager_content(&content))
}

fn parse_tag_manager_content(content: &str) -> TagManagerConfig {
    let mut custom_tags = Vec::new();
    let mut layer_slots = Vec::new();
    let mut section = SequenceSection::None;
    let mut section_indent = 0usize;

    for line in content.lines() {
        let trimmed = line.trim();
        let indent = line.len().saturating_sub(line.trim_start().len());

        if trimmed == "tags:" {
            section = SequenceSection::Tags;
            section_indent = indent;
            continue;
        }
        if trimmed == "tags: []" {
            section = SequenceSection::None;
            continue;
        }
        if trimmed == "layers:" {
            section = SequenceSection::Layers;
            section_indent = indent;
            continue;
        }

        let sequence_value = trimmed
            .strip_prefix("- ")
            .or_else(|| (trimmed == "-").then_some(""));

        // Unity serializes `layers`, `m_SortingLayers`, and other TagManager
        // fields as siblings beneath `TagManager`. Stop at the next sibling
        // key so sorting-layer entries cannot leak into the layer table.
        if section != SequenceSection::None
            && sequence_value.is_none()
            && indent <= section_indent
            && trimmed.contains(':')
        {
            section = SequenceSection::None;
        }

        let Some(value) = sequence_value else {
            continue;
        };

        match section {
            SequenceSection::Tags => {
                let tag = value.trim();
                if !tag.is_empty() {
                    custom_tags.push(tag.to_string());
                }
            }
            SequenceSection::Layers => {
                if layer_slots.len() < UNITY_LAYER_COUNT {
                    let name = value.trim();
                    layer_slots.push((!name.is_empty()).then(|| name.to_string()));
                }
            }
            SequenceSection::None => {}
        }
    }

    layer_slots.resize_with(UNITY_LAYER_COUNT, || None);
    TagManagerConfig {
        custom_tags,
        layer_slots,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_parser_preserves_empty_slots_and_stops_before_sorting_layers() {
        let config = parse_tag_manager_content(
            r#"%YAML 1.1
TagManager:
  serializedVersion: 2
  tags:
  - Enemy
  layers:
  - Default
  - TransparentFX
  - Ignore Raycast
  -
  - Water
  - UI
  - Ground
  -
  m_SortingLayers:
  - name: Default
    uniqueID: 0
"#,
        );

        assert_eq!(config.custom_tags, vec!["Enemy"]);
        assert_eq!(config.layer_slots.len(), UNITY_LAYER_COUNT);
        assert_eq!(config.layer_slots[3], None);
        assert_eq!(config.layer_slots[6].as_deref(), Some("Ground"));

        let named: Vec<_> = config.named_layers().collect();
        assert_eq!(
            named,
            vec![
                (0, "Default"),
                (1, "TransparentFX"),
                (2, "Ignore Raycast"),
                (4, "Water"),
                (5, "UI"),
                (6, "Ground"),
            ]
        );
        assert!(!named.iter().any(|(_, name)| name.contains("name:")));
    }

    #[test]
    fn layer_parser_keeps_all_empty_slots_out_of_named_layers() {
        let config = parse_tag_manager_content(
            r#"TagManager:
  tags: []
  layers:
  - Default
  -
  -
  m_SortingLayers: []
"#,
        );

        assert_eq!(config.layer_slots[0].as_deref(), Some("Default"));
        assert_eq!(config.layer_slots[1], None);
        assert_eq!(config.layer_slots[31], None);
        assert_eq!(
            config.named_layers().collect::<Vec<_>>(),
            vec![(0, "Default")]
        );
    }
}
