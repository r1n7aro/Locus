use super::*;

pub(crate) fn is_csv_document(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".csv")
}

pub(crate) fn is_knowledge_document_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "csv")
    )
}

pub(super) fn is_csv_view_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".csv.view"))
}

pub(super) fn csv_document_id(doc_type: KnowledgeType, path: &str) -> String {
    format!(
        "kd_csv_{}",
        blake3::hash(format!("{}:{path}", doc_type.as_str()).as_bytes()).to_hex()
    )
}

pub(super) fn parse_csv_document(
    content: &str,
    doc_type: KnowledgeType,
    path: &str,
) -> Result<KnowledgeDocument, String> {
    let path = normalize_relative_path(path)?;
    if doc_type == KnowledgeType::Skill {
        return Err("CSV documents cannot be executable skills".to_string());
    }
    let mut doc = KnowledgeDocument {
        inject_agents: crate::knowledge_store::default_inject_agents(),
        id: csv_document_id(doc_type, &path),
        title: default_document_title_from_path(&path)?,
        path,
        doc_type,
        inject_mode: default_document_inject_mode_for_type(doc_type),
        inherit_inject_mode: true,
        inject_mode_source: type_default_config_source(),
        summary_enabled: false,
        command_enabled: false,
        read_only: false,
        ai_edit_mode: KnowledgeAiEditMode::Inherit,
        ai_maintained: default_ai_maintained_for_type(doc_type),
        storage_source: KnowledgeStorageSource::Project,
        inherit_ai_config: true,
        ai_config_source: type_default_config_source(),
        explicit_maintenance_rules: false,
        external_source: None,
        skill_enabled: None,
        skill_surface: None,
        command_trigger: None,
        argument_hint: None,
        tools: Vec::new(),
        summary: None,
        body: content.to_string(),
        maintenance_rules: None,
        created_at: 0,
        updated_at: 0,
    };
    resolve_document_inheritance(None, &mut doc)?;
    ensure_maintenance_rules(&mut doc);
    Ok(doc)
}

fn view_path(csv: &Path) -> PathBuf {
    let mut name = csv.as_os_str().to_os_string();
    name.push(".view");
    PathBuf::from(name)
}

pub(super) fn save_moved_document(
    working_dir: &str,
    doc: KnowledgeDocument,
    old_type: KnowledgeType,
    old_path: &str,
) -> Result<KnowledgeDocument, String> {
    let old_file = document_path(working_dir, old_type, old_path)?;
    let moved = doc.doc_type != old_type || doc.path != old_path;
    let new_file = document_path(working_dir, doc.doc_type, &doc.path)?;
    let old_view = view_path(&old_file);
    let new_view = view_path(&new_file);
    if moved && is_csv_document(old_path) && (new_file.exists() || new_view.exists()) {
        return Err("The target CSV or its view already exists".to_string());
    }
    let saved = save_document(working_dir, doc)?;
    if moved && is_csv_document(old_path) && old_view.is_file() {
        if let Err(error) = std::fs::rename(&old_view, &new_view) {
            let _ = std::fs::remove_file(&new_file);
            return Err(format!(
                "Failed to move CSV view; source CSV is preserved: {error}"
            ));
        }
    }
    if moved && old_file.is_file() {
        if let Err(error) = std::fs::remove_file(&old_file) {
            if is_csv_document(old_path) && new_view.is_file() {
                let _ = std::fs::rename(&new_view, &old_view);
            }
            return Err(format!(
                "Failed to remove original document after saving the new path: {error}"
            ));
        }
    }
    Ok(saved)
}

pub(super) fn remove_csv_view(csv: &Path) -> Result<(), String> {
    let view = view_path(csv);
    if view.is_file() {
        std::fs::remove_file(view)
            .map_err(|error| format!("Failed to delete CSV view: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_is_indexed_read_and_saved_without_frontmatter_or_whitespace_changes() {
        let root = tempfile::tempdir().unwrap();
        let working_dir = root.path().to_str().unwrap();
        ensure_knowledge_roots(working_dir).unwrap();
        let path = document_path(working_dir, KnowledgeType::Design, "items.csv").unwrap();
        let original = "\u{feff}id,name,\r\n001,\"line\r\ntext\",\r\n\r\n";
        std::fs::write(&path, original).unwrap();
        let doc = load_document_by_path(working_dir, KnowledgeType::Design, "items.csv").unwrap();
        assert_eq!(doc.body, original);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(render_document_for_filesystem_read(&doc).unwrap(), original);
        save_document(working_dir, doc.clone()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(
            doc.id,
            load_document_by_path(working_dir, KnowledgeType::Design, "items.csv")
                .unwrap()
                .id
        );
        assert_eq!(
            prepare_generic_knowledge_write(
                working_dir,
                KnowledgeType::Design,
                "items.csv",
                original
            )
            .unwrap()
            .content,
            original
        );
        let markdown = prepare_generic_knowledge_write(
            working_dir,
            KnowledgeType::Design,
            "notes.md",
            "# Notes\n",
        )
        .unwrap();
        std::fs::write(path.with_file_name("notes.md"), markdown.content).unwrap();
        std::fs::write(view_path(&path), "schema: locus.csv-view.v1\n").unwrap();
        let items = list_documents(working_dir, Some(KnowledgeType::Design), None).unwrap();
        assert!(items.iter().any(|item| item.path == "items.csv"));
        assert!(items.iter().any(|item| item.path == "notes.md"));
        assert!(!items.iter().any(|item| item.path.ends_with(".view")));
    }

    #[test]
    fn csv_generic_write_rejects_executable_skill_documents() {
        let root = tempfile::tempdir().unwrap();
        let error = prepare_generic_knowledge_write(
            root.path().to_str().unwrap(),
            KnowledgeType::Skill,
            "workflow.csv",
            "step,action\n1,inspect\n",
        )
        .expect_err("CSV must not be registered as an executable Skill");
        assert_eq!(error, "CSV documents cannot be executable skills");
    }

    #[test]
    fn csv_rename_moves_view_and_rejects_existing_target() {
        let root = tempfile::tempdir().unwrap();
        let working_dir = root.path().to_str().unwrap();
        ensure_knowledge_roots(working_dir).unwrap();
        let old = document_path(working_dir, KnowledgeType::Design, "old.csv").unwrap();
        std::fs::write(&old, "id\n001\n").unwrap();
        std::fs::write(view_path(&old), "schema: locus.csv-view.v1\n").unwrap();
        let mut doc = load_document_by_path(working_dir, KnowledgeType::Design, "old.csv").unwrap();
        doc.path = "new.csv".to_string();
        let saved =
            save_moved_document(working_dir, doc, KnowledgeType::Design, "old.csv").unwrap();
        let new_path = document_path(working_dir, KnowledgeType::Design, "new.csv").unwrap();
        assert!(!old.exists());
        assert!(!view_path(&old).exists());
        assert!(view_path(&new_path).exists());
        assert_eq!(
            saved.id,
            load_document_by_path(working_dir, KnowledgeType::Design, "new.csv")
                .unwrap()
                .id
        );
        let mut again = saved;
        again.path = "taken.csv".to_string();
        std::fs::write(new_path.with_file_name("taken.csv"), "untouched").unwrap();
        assert!(save_moved_document(working_dir, again, KnowledgeType::Design, "new.csv").is_err());
        assert!(new_path.exists());
    }
}
