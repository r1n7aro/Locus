//! Markdown-local image paths, shared by import, rendering and explorer renames.
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use serde_json::{Map, Value};
use std::path::{Component, Path, PathBuf};

/// Canonical form of a workspace-relative folder setting: `/` separators,
/// no empty or `.` segments, no trailing slash. Empty input stays empty
/// (unset). Absolute paths, `..` and names Windows cannot store are rejected
/// so the folder always stays inside the workspace.
pub fn normalize_workspace_folder(value: &str) -> Result<String, String> {
    let text = value.trim().replace('\\', "/");
    if text.is_empty() {
        return Ok(String::new());
    }
    if text.starts_with('/') || Path::new(&text).is_absolute() {
        return Err("must be a folder relative to the workspace".to_string());
    }
    let mut segments = Vec::new();
    for segment in text.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return Err("must stay inside the workspace".to_string()),
            _ if segment.chars().any(|ch| {
                ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            }) =>
            {
                return Err(format!("'{segment}' is not a valid folder name"));
            }
            _ if segment != segment.trim_end_matches([' ', '.']) => {
                return Err(format!("'{segment}' must not end with a space or a dot"));
            }
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        return Err("must name a folder inside the workspace".to_string());
    }
    Ok(segments.join("/"))
}

fn folder_key(folder: &str) -> String {
    if cfg!(windows) {
        folder.to_lowercase()
    } else {
        folder.to_owned()
    }
}

/// Canonical rule map `{ document folder: image folder }`. Both sides are
/// required workspace-relative folders; two spellings of one document
/// folder are rejected because only one of them could ever apply.
pub fn normalize_image_rules(rules: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut normalized = Map::with_capacity(rules.len());
    let mut seen = std::collections::HashSet::new();
    for (documents, images) in rules {
        let documents = normalize_workspace_folder(documents)
            .map_err(|reason| format!("document folder '{documents}' {reason}"))?;
        let images_text = images.as_str().unwrap_or_default();
        let images = normalize_workspace_folder(images_text)
            .map_err(|reason| format!("image folder '{images_text}' {reason}"))?;
        if documents.is_empty() || images.is_empty() {
            return Err("each rule needs a document folder and an image folder".to_string());
        }
        if !seen.insert(folder_key(&documents)) {
            return Err(format!(
                "document folder '{documents}' has more than one rule"
            ));
        }
        normalized.insert(documents, Value::from(images));
    }
    Ok(normalized)
}

/// Workspace-relative folder that receives images pasted into the document
/// at `document` (workspace-relative, `/` separated). The rule with the
/// longest matching document folder wins; otherwise the project's default
/// folder applies. `None` means the project has not chosen a folder yet.
pub fn image_folder_for(
    document: &str,
    default_folder: &str,
    rules: &Map<String, Value>,
) -> Option<String> {
    let document = folder_key(&document.replace('\\', "/"));
    let parent = document.rsplit_once('/').map_or("", |(parent, _)| parent);
    rules
        .iter()
        .filter_map(|(folder, images)| Some((folder_key(folder), images.as_str()?)))
        .filter(|(folder, images)| {
            !folder.is_empty()
                && !images.is_empty()
                && (parent == folder
                    || parent
                        .strip_prefix(folder.as_str())
                        .is_some_and(|rest| rest.starts_with('/')))
        })
        .max_by_key(|(folder, _)| folder.len())
        .map(|(_, images)| images.to_owned())
        .or_else(|| (!default_folder.is_empty()).then(|| default_folder.to_owned()))
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            part => result.push(part.as_os_str()),
        }
    }
    result
}

pub fn path_key(path: &Path) -> String {
    let value = normalize(path).to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

pub fn decode_path(value: &str) -> String {
    let mut bytes = Vec::new();
    let input = value.as_bytes();
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let Ok(hex) = std::str::from_utf8(&input[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    bytes.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        bytes.push(input[i]);
        i += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| value.to_owned())
}

pub fn relative_url(document: &Path, image: &Path) -> String {
    let base = normalize(document.parent().unwrap_or(Path::new("")));
    let target = normalize(image);
    let from: Vec<_> = base.components().collect();
    let to: Vec<_> = target.components().collect();
    let shared = from
        .iter()
        .zip(&to)
        .take_while(|(a, b)| {
            if cfg!(windows) {
                a.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
            } else {
                a == b
            }
        })
        .count();
    let mut parts = vec!["..".to_string(); from.len() - shared];
    parts.extend(
        to[shared..]
            .iter()
            .map(|p| p.as_os_str().to_string_lossy().into_owned()),
    );
    parts
        .join("/")
        .chars()
        .map(|c| match c {
            '%' => "%25".into(),
            ' ' => "%20".into(),
            '#' => "%23".into(),
            '?' => "%3F".into(),
            '(' => "%28".into(),
            ')' => "%29".into(),
            '<' => "%3C".into(),
            '>' => "%3E".into(),
            _ => c.to_string(),
        })
        .collect()
}

pub fn resolve_local(root: &Path, document: &Path, source: &str) -> Option<PathBuf> {
    if source.starts_with('#')
        || source.contains("://")
        || source.starts_with("data:")
        || source.starts_with("blob:")
    {
        return None;
    }
    let path = decode_path(source.split(['#', '?']).next().unwrap_or(source)).replace('\\', "/");
    let path = Path::new(&path);
    if path.is_absolute() {
        return Some(normalize(path));
    }
    let local = normalize(&document.parent().unwrap_or(root).join(path));
    // Preserve legacy workspace-root image paths; newly imported paths are relative.
    if local.exists() || source.starts_with('.') {
        Some(local)
    } else {
        Some(normalize(&root.join(path)))
    }
}

fn destination_range(raw: &str, definition: bool) -> Option<std::ops::Range<usize>> {
    let bytes = raw.as_bytes();
    let mut depth = 0;
    let mut i = usize::from(raw.starts_with('!'));
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'[' {
            depth += 1;
        }
        if bytes[i] == b']' {
            depth -= 1;
            if depth == 0 {
                i += 1;
                break;
            }
        }
        i += 1;
    }
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    if bytes.get(i) != Some(&if definition { b':' } else { b'(' }) {
        return None;
    }
    i += 1;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    let angle = bytes.get(i) == Some(&b'<');
    if angle {
        i += 1;
    }
    let start = i;
    let mut parens = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if angle && c == b'>' {
            break;
        }
        if !angle {
            if c.is_ascii_whitespace() || (c == b')' && parens == 0) {
                break;
            }
            if c == b'(' {
                parens += 1;
            }
            if c == b')' {
                parens -= 1;
            }
        }
        i += 1;
    }
    (i > start).then_some(start..i)
}

pub fn rewrite_image_links(
    text: &str,
    root: &Path,
    document: &Path,
    old: &Path,
    new: &Path,
) -> String {
    let parser = Parser::new_ext(text, Options::all());
    let definitions: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, d)| (d.span.clone(), d.dest.to_string()))
        .collect();
    let mut edits = Vec::new();
    let mut add = |span: std::ops::Range<usize>, dest: &str, definition: bool| {
        // The old file may already have moved. Try document-relative resolution
        // before the legacy workspace-relative form without requiring existence.
        if dest.contains("://") || dest.starts_with("data:") || dest.starts_with("blob:") {
            return;
        }
        let suffix_at = dest.find(['#', '?']).unwrap_or(dest.len());
        let decoded = decode_path(&dest[..suffix_at]).replace('\\', "/");
        let p = Path::new(&decoded);
        let relative = normalize(&document.parent().unwrap_or(root).join(p));
        let legacy = normalize(&root.join(p));
        if path_key(&relative) != path_key(old)
            && (relative.exists() || path_key(&legacy) != path_key(old))
        {
            return;
        }
        if let Some(range) = destination_range(&text[span.clone()], definition) {
            edits.push((
                span.start + range.start..span.start + range.end,
                format!("{}{}", relative_url(document, new), &dest[suffix_at..]),
            ));
        }
    };
    for (event, span) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Image { link_type: LinkType::Inline, dest_url, .. })
            | Event::Start(Tag::Link { link_type: LinkType::Inline, dest_url, .. }) => add(span, &dest_url, false),
            _ => {},
        }
    }
    // Definitions can be shared by images and ordinary links; update the target once.
    for (span, dest) in definitions {
        add(span, &dest, true);
    }
    edits.sort_by_key(|(range, _)| range.start);
    edits.dedup_by_key(|(range, _)| range.start);
    let mut result = text.to_owned();
    for (range, replacement) in edits.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    result
}

pub struct ReferenceUpdate {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
}

pub fn image_reference_updates(
    root: &Path,
    old: &Path,
    new: &Path,
) -> Result<Vec<ReferenceUpdate>, String> {
    let mut updates = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || !matches!(
                    entry.file_name().to_str().unwrap_or(""),
                    ".git" | "node_modules" | "Library" | "Temp" | "target" | "obj" | "Logs"
                )
        })
    {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().is_file()
            || !entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        {
            continue;
        }
        let before = std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
        let after = rewrite_image_links(&before, root, entry.path(), old, new);
        if before != after {
            updates.push(ReferenceUpdate {
                path: entry.path().to_path_buf(),
                before,
                after,
            });
        }
    }
    Ok(updates)
}

pub fn rollback_updates(updates: &[ReferenceUpdate]) -> Result<(), String> {
    for entry in updates.iter().rev() {
        if std::fs::read_to_string(&entry.path).map_err(|e| e.to_string())? != entry.after {
            return Err(format!(
                "Document changed during rename: {}",
                entry.path.display()
            ));
        }
        crate::config::atomic_write_config(&entry.path, entry.before.as_bytes())?;
    }
    Ok(())
}

pub fn apply_updates(updates: &[ReferenceUpdate]) -> Result<(), String> {
    for (i, entry) in updates.iter().enumerate() {
        let result = (|| {
            if std::fs::read_to_string(&entry.path).map_err(|e| e.to_string())? != entry.before {
                return Err(format!(
                    "Document changed during rename: {}",
                    entry.path.display()
                ));
            }
            crate::config::atomic_write_config(&entry.path, entry.after.as_bytes())
        })();
        if let Err(error) = result {
            rollback_updates(&updates[..i])?;
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_folders_are_normalized_and_kept_inside_the_workspace() {
        assert_eq!(normalize_workspace_folder("").unwrap(), "");
        assert_eq!(normalize_workspace_folder("  ").unwrap(), "");
        assert_eq!(
            normalize_workspace_folder(" ./docs\\images//shots/ ").unwrap(),
            "docs/images/shots"
        );
        assert_eq!(
            normalize_workspace_folder("Locus/attachments/images").unwrap(),
            "Locus/attachments/images"
        );
        for invalid in [
            "..",
            "docs/../..",
            "/abs",
            "C:/abs",
            "C:docs",
            "a/b?",
            "docs./x",
            ".",
        ] {
            assert!(
                normalize_workspace_folder(invalid).is_err(),
                "{invalid} should be rejected"
            );
        }
    }

    #[test]
    fn image_rules_need_both_folders_and_unique_document_folders() {
        let rules = json!({ "docs\\design/": "docs/design/images", "Locus/knowledge": "./Locus/attachments/knowledge" });
        assert_eq!(
            Value::Object(normalize_image_rules(rules.as_object().unwrap()).unwrap()),
            json!({ "docs/design": "docs/design/images", "Locus/knowledge": "Locus/attachments/knowledge" })
        );
        assert!(normalize_image_rules(json!({ "docs": "" }).as_object().unwrap()).is_err());
        assert!(normalize_image_rules(json!({ "docs": "../out" }).as_object().unwrap()).is_err());
        assert!(normalize_image_rules(json!({ ".": "images" }).as_object().unwrap()).is_err());
        assert!(
            normalize_image_rules(json!({ "docs": "a", "docs/": "b" }).as_object().unwrap())
                .is_err()
        );
    }

    #[test]
    fn longest_matching_rule_wins_before_the_default_folder() {
        let rules = json!({
            "docs": "docs/images",
            "docs/design": "docs/design/images",
            "Locus/knowledge": "Locus/attachments/knowledge",
        });
        let rules = rules.as_object().unwrap();
        let folder = |document: &str, default: &str| image_folder_for(document, default, rules);
        assert_eq!(
            folder("docs/design/a.md", "").as_deref(),
            Some("docs/design/images")
        );
        assert_eq!(
            folder("docs/design/sub/b.md", "").as_deref(),
            Some("docs/design/images")
        );
        assert_eq!(folder("docs/guide.md", "").as_deref(), Some("docs/images"));
        // A folder name that merely starts with a rule's name does not match.
        assert_eq!(folder("docs-old/a.md", "assets").as_deref(), Some("assets"));
        assert_eq!(folder("README.md", "assets").as_deref(), Some("assets"));
        assert_eq!(folder("README.md", ""), None);
        assert_eq!(
            folder("Locus/knowledge/design/x.md", "").as_deref(),
            Some("Locus/attachments/knowledge")
        );
        if cfg!(windows) {
            assert_eq!(
                folder("Docs/Design/a.md", "").as_deref(),
                Some("docs/design/images")
            );
        }
    }

    #[test]
    fn rewrites_only_image_destinations_and_reference_definitions() {
        let root = Path::new("/project");
        let doc = root.join("docs/readme.md");
        let old = root.join("Locus/attachments/images/old.png");
        let new = root.join("Locus/attachments/images/new name.png");
        let text = "![label](../Locus/attachments/images/old.png \"title\")\n![x][id]\n\n[id]: <../Locus/attachments/images/old.png>\n`![x](../Locus/attachments/images/old.png)`\n```md\n![x](../Locus/attachments/images/old.png)\n```\n![remote](https://example.org/old.png)";
        let actual = rewrite_image_links(text, root, &doc, &old, &new);
        assert!(
            actual.starts_with("![label](../Locus/attachments/images/new%20name.png \"title\")")
        );
        assert!(actual.contains("[id]: <../Locus/attachments/images/new%20name.png>"));
        assert!(actual.contains("`![x](../Locus/attachments/images/old.png)`"));
        assert!(actual.contains("```md\n![x](../Locus/attachments/images/old.png)"));
        assert!(actual.contains("https://example.org/old.png"));
    }
    #[test]
    fn rename_updates_nested_documents_and_can_rollback() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/a.md"), "![x](../old.png)").unwrap();
        let updates =
            image_reference_updates(root, &root.join("old.png"), &root.join("new.png")).unwrap();
        assert_eq!(updates.len(), 1);
        apply_updates(&updates).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("docs/a.md")).unwrap(),
            "![x](../new.png)"
        );
        rollback_updates(&updates).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("docs/a.md")).unwrap(),
            "![x](../old.png)"
        );
    }
}
