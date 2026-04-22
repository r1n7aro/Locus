use super::types::{parse_guid_hex, Guid};

pub fn extract_guid(content: &[u8]) -> Option<Guid> {
    let text = String::from_utf8_lossy(content);
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("guid:") {
            let hex = rest.trim();
            if hex.len() >= 32 {
                return parse_guid_hex(&hex[..32]);
            }
        }
    }
    None
}

fn parse_bool_flag(value: &str) -> Option<bool> {
    match value.trim() {
        "1" | "true" | "True" | "TRUE" | "yes" | "on" => Some(true),
        "0" | "false" | "False" | "FALSE" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub fn extract_importer_name(content: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(content);
    for line in text.lines() {
        let trimmed = line.trim();
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(name) = trimmed.strip_suffix(':') {
            if name.ends_with("Importer") {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub fn extract_alpha_is_transparency(content: &[u8]) -> Option<bool> {
    let text = String::from_utf8_lossy(content);
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("alphaIsTransparency:") {
            return parse_bool_flag(rest);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_guid_basic() {
        let content = b"fileFormatVersion: 2\nguid: abcdef0123456789abcdef0123456789\n";
        let guid = extract_guid(content).unwrap();
        assert_eq!(
            guid,
            [
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45,
                0x67, 0x89
            ]
        );
    }

    #[test]
    fn test_extract_guid_with_extra_content() {
        let content = b"fileFormatVersion: 2\nguid: 00112233445566778899aabbccddeeff\nNativeFormatImporter:\n";
        let guid = extract_guid(content).unwrap();
        assert_eq!(guid[0], 0x00);
        assert_eq!(guid[15], 0xff);
    }

    #[test]
    fn test_extract_guid_missing() {
        let content = b"fileFormatVersion: 2\nno guid here\n";
        assert!(extract_guid(content).is_none());
    }

    #[test]
    fn test_extract_guid_invalid_hex() {
        let content = b"guid: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\n";
        assert!(extract_guid(content).is_none());
    }

    #[test]
    fn test_extract_importer_name() {
        let content = b"fileFormatVersion: 2\nguid: 00112233445566778899aabbccddeeff\nTextureImporter:\n  alphaIsTransparency: 1\n";
        assert_eq!(
            extract_importer_name(content),
            Some("TextureImporter".to_string())
        );
    }

    #[test]
    fn test_extract_alpha_is_transparency_true_and_false() {
        let enabled = b"TextureImporter:\n  alphaIsTransparency: 1\n";
        let disabled = b"TextureImporter:\n  alphaIsTransparency: 0\n";
        assert_eq!(extract_alpha_is_transparency(enabled), Some(true));
        assert_eq!(extract_alpha_is_transparency(disabled), Some(false));
    }

    #[test]
    fn test_extract_alpha_is_transparency_missing() {
        let content = b"TextureImporter:\n  mipmaps:\n    enableMipMap: 0\n";
        assert_eq!(extract_alpha_is_transparency(content), None);
    }
}
