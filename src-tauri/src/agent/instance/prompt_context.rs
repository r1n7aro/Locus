use crate::unity_project_config::{load_tag_manager, BUILTIN_TAGS};

pub(super) fn parse_tag_manager(
    project_dir: &std::path::Path,
) -> (Vec<(usize, String)>, Vec<(usize, String)>) {
    let config = match load_tag_manager(project_dir) {
        Ok(config) => config,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let layers = config
        .named_layers()
        .map(|(index, name)| (index, name.to_string()))
        .collect();

    let mut tags: Vec<(usize, String)> = BUILTIN_TAGS
        .iter()
        .enumerate()
        .map(|(index, name)| (index, (*name).to_string()))
        .collect();
    for (index, name) in config.custom_tags.into_iter().enumerate() {
        tags.push((BUILTIN_TAGS.len() + index, name));
    }

    (tags, layers)
}

pub(super) fn parse_physics_config(
    project_dir: &std::path::Path,
    layers: &[(usize, String)],
) -> String {
    let dynamics_path = project_dir
        .join("ProjectSettings")
        .join("DynamicsManager.asset");
    let physics2d_path = project_dir
        .join("ProjectSettings")
        .join("Physics2DSettings.asset");

    let matrix_3d = read_collision_matrix(&dynamics_path);
    let matrix_2d = read_collision_matrix(&physics2d_path);

    let default_all: u32 = 0xFFFFFFFF;
    let is_3d_custom = !matrix_3d.is_empty() && matrix_3d.iter().any(|&v| v != default_all);
    let is_2d_custom = !matrix_2d.is_empty() && matrix_2d.iter().any(|&v| v != default_all);

    let mut parts: Vec<String> = Vec::new();

    match (is_3d_custom, is_2d_custom) {
        (true, true) => {
            parts.push(format_collision_matrix("3D", &matrix_3d, layers));
            parts.push(format_collision_matrix("2D", &matrix_2d, layers));
        }
        (true, false) => {
            parts.push(format_collision_matrix("3D", &matrix_3d, layers));
        }
        (false, true) => {
            parts.push(format_collision_matrix("2D", &matrix_2d, layers));
        }
        (false, false) => {
            parts.push("3D & 2D collision matrices: Default (all layers collide)".to_string());
        }
    }

    parts.join("\n")
}

fn read_collision_matrix(path: &std::path::Path) -> Vec<u32> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut matrix: Vec<u32> = Vec::new();
    let mut in_matrix = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("m_LayerCollisionMatrix:") {
            let rest = trimmed
                .strip_prefix("m_LayerCollisionMatrix:")
                .unwrap()
                .trim();
            if !rest.is_empty() {
                matrix = decode_collision_matrix_hex(rest);
                if !matrix.is_empty() {
                    return matrix;
                }
            }
            in_matrix = true;
            continue;
        }

        if in_matrix {
            if let Some(val_str) = trimmed.strip_prefix("- ") {
                let val_str = val_str.trim();
                if let Ok(v) = val_str.parse::<u32>() {
                    matrix.push(v);
                } else if let Ok(v) = u32::from_str_radix(val_str, 16) {
                    matrix.push(v);
                }
            } else if !trimmed.starts_with('-') && trimmed.contains(':') && !trimmed.is_empty() {
                break;
            }
        }
    }

    matrix
}

fn decode_collision_matrix_hex(hex: &str) -> Vec<u32> {
    let bytes = hex.trim().as_bytes();
    if bytes.is_empty() || bytes.len() % 8 != 0 {
        return Vec::new();
    }

    let mut matrix = Vec::with_capacity(bytes.len() / 8);
    for chunk in bytes.chunks_exact(8) {
        let mut word = [0u8; 4];
        for (index, byte) in word.iter_mut().enumerate() {
            let start = index * 2;
            let Ok(pair) = std::str::from_utf8(&chunk[start..start + 2]) else {
                return Vec::new();
            };
            let Ok(value) = u8::from_str_radix(pair, 16) else {
                return Vec::new();
            };
            *byte = value;
        }
        matrix.push(u32::from_le_bytes(word));
    }
    matrix
}

fn format_collision_matrix(label: &str, matrix: &[u32], layers: &[(usize, String)]) -> String {
    let default_all: u32 = 0xFFFFFFFF;

    let all_default = matrix.iter().all(|&v| v == default_all);

    if all_default || matrix.is_empty() {
        return format!(
            "Physics {}: Default collision matrix (all layers collide)",
            label
        );
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Physics {}: Custom collision matrix — ignored pairs:",
        label
    ));

    for &(idx, ref name) in layers {
        if idx >= matrix.len() {
            continue;
        }
        let bits = matrix[idx];
        if bits == default_all {
            continue;
        }
        let mut ignored: Vec<String> = Vec::new();
        for &(other_idx, ref other_name) in layers {
            if other_idx > 31 {
                continue;
            }
            if bits & (1u32 << other_idx) == 0 {
                ignored.push(format!("{}:{}", other_idx, other_name));
            }
        }
        if !ignored.is_empty() {
            lines.push(format!(
                "  {}:{} ignores [{}]",
                idx,
                name,
                ignored.join(", ")
            ));
        }
    }

    lines.join("\n")
}

pub(super) fn detect_input_system(project_dir: &std::path::Path) -> String {
    let path = project_dir
        .join("ProjectSettings")
        .join("ProjectSettings.asset");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return "Unknown".to_string(),
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("activeInputHandler:") {
            return match val.trim() {
                "0" => "Legacy Input Manager".to_string(),
                "1" => "New Input System (com.unity.inputsystem)".to_string(),
                "2" => "Both (Legacy + New Input System)".to_string(),
                other => format!("Unknown ({})", other),
            };
        }
    }

    "Legacy Input Manager (default)".to_string()
}

pub(super) fn detect_render_pipeline(project_dir: &std::path::Path) -> String {
    let manifest_path = project_dir.join("Packages").join("manifest.json");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap_or_default();

    let has_urp = manifest.contains("com.unity.render-pipelines.universal");
    let has_hdrp = manifest.contains("com.unity.render-pipelines.high-definition");

    let graphics_path = project_dir
        .join("ProjectSettings")
        .join("GraphicsSettings.asset");
    let graphics = std::fs::read_to_string(&graphics_path).unwrap_or_default();

    let has_custom_pipeline = graphics.lines().any(|line| {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("m_CustomRenderPipeline:") {
            let rest = rest.trim();
            !(rest.is_empty() || rest.contains("fileID: 0,") || rest == "{fileID: 0}")
        } else {
            false
        }
    });

    match (has_urp, has_hdrp, has_custom_pipeline) {
        (true, false, _) => "URP (Universal Render Pipeline)".to_string(),
        (false, true, _) => "HDRP (High Definition Render Pipeline)".to_string(),
        (true, true, _) => "URP + HDRP (both packages present)".to_string(),
        (false, false, true) => "Custom SRP".to_string(),
        (false, false, false) => "Built-in Render Pipeline (BIRP)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_layers_keep_unity_indices_and_omit_empty_slots() {
        let temp = tempfile::tempdir().unwrap();
        let settings = temp.path().join("ProjectSettings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("TagManager.asset"),
            r#"TagManager:
  tags: []
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
"#,
        )
        .unwrap();

        let (_, layers) = parse_tag_manager(temp.path());
        assert_eq!(
            layers,
            vec![
                (0, "Default".to_string()),
                (1, "TransparentFX".to_string()),
                (2, "Ignore Raycast".to_string()),
                (4, "Water".to_string()),
                (5, "UI".to_string()),
                (6, "Ground".to_string()),
            ]
        );
    }

    #[test]
    fn collision_matrix_hex_words_are_little_endian() {
        assert_eq!(
            decode_collision_matrix_hex("ffdfffffc8c0ffff"),
            vec![0xffff_dfff, 0xffff_c0c8]
        );
        assert!(decode_collision_matrix_hex("not-hex").is_empty());
    }
}
