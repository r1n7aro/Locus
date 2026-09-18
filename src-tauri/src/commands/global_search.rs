//! Interactive literal search. Work is paged on blocking workers; only short
//! excerpts cross IPC. No writes to the conversation database or its schema.
use crate::{
    error::AppError,
    knowledge_index,
    knowledge_store::{self, KnowledgeType},
    session::store::SessionStore,
    workspace_service::{ProjectRegistry, WorkspaceRef},
    AppKnowledgeDir,
};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, State};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlobalSearchSource {
    KnowledgeTitle,
    KnowledgeContent,
    SessionTitle,
    SessionContent,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchHit {
    kind: &'static str,
    id: String,
    title: String,
    excerpt: String,
    field: &'static str,
    doc_type: Option<KnowledgeType>,
    path: Option<String>,
    message_id: Option<String>,
    checkout_id: Option<String>,
    archived: bool,
    /// Epoch milliseconds, consistent for documents and conversations.
    modified_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchPage {
    matches: Vec<GlobalSearchHit>,
    next_cursor: Option<String>,
}

// Limit concurrent scans across windows. The frontend also keeps at most one
// request per source family in flight and retires stale generations.
static WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[tauri::command]
pub async fn global_search(
    workspace_ref: WorkspaceRef,
    query: String,
    source: GlobalSearchSource,
    archived: Option<bool>,
    cursor: Option<String>,
    app_handle: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
    store: State<'_, Arc<SessionStore>>,
    app_knowledge_dir: State<'_, AppKnowledgeDir>,
) -> Result<GlobalSearchPage, AppError> {
    let query = query.trim().to_string();
    if query.is_empty() || query.chars().count() > 200 || query.contains('\0') {
        return Err("Search query must contain 1..200 characters and no NUL"
            .to_string()
            .into());
    }
    let scope = super::knowledge::resolve_knowledge_workspace_scope(
        registry.inner().as_ref(),
        &workspace_ref,
    )?;
    let store = store.inner().clone();
    let app_root = app_knowledge_dir.0.as_ref().clone();
    let is_knowledge = matches!(
        source,
        GlobalSearchSource::KnowledgeTitle | GlobalSearchSource::KnowledgeContent
    );
    let working_dir = scope.runtime().root().to_string_lossy().into_owned();
    let db_path = if is_knowledge {
        let index = scope
            .runtime()
            .knowledge_index(&app_handle)
            .map_err(AppError::from)?;
        // Catalog bootstrap is shared with the knowledge explorer and reconciles
        // stale disk entries once per runtime, independently of semantic search.
        knowledge_index::ensure_document_catalog_available(
            &working_dir,
            app_root.as_ref(),
            index.clone(),
        )
        .await?;
        let db = index.db();
        let conn = db.conn().lock().map_err(|e| e.to_string())?;
        Some(PathBuf::from(
            conn.path().ok_or("Knowledge database is unavailable")?,
        ))
    } else {
        None
    };
    let permit = WORKERS.acquire().await.map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if let Some(path) = db_path {
            return search_knowledge_page(
                &path,
                &working_dir,
                app_root.as_ref(),
                &query,
                matches!(source, GlobalSearchSource::KnowledgeTitle),
                cursor.as_deref(),
            );
        }
        let archived = archived.unwrap_or(false);
        let titles = matches!(source, GlobalSearchSource::SessionTitle);
        let page = store.search_session_history_fields(
            scope.runtime().project_id().as_str(),
            &query,
            archived,
            None,
            20,
            cursor.as_deref(),
            titles,
            !titles,
            false,
            true,
        )?;
        Ok(GlobalSearchPage {
            matches: page
                .matches
                .into_iter()
                .map(|hit| GlobalSearchHit {
                    kind: "session",
                    id: hit.session_id,
                    title: bounded_title(&hit.session_title, &query),
                    excerpt: hit.excerpt,
                    field: if titles { "title" } else { "content" },
                    doc_type: None,
                    path: None,
                    message_id: hit.message_id,
                    checkout_id: hit.default_checkout_id,
                    archived,
                    modified_at: hit.updated_at.saturating_mul(1000),
                })
                .collect(),
            next_cursor: page.next_cursor,
        })
    })
    .await
    .map_err(|e| format!("Search worker failed: {e}"))?
    .map_err(Into::into)
}

fn excerpt(text: &str, query: &str) -> Option<String> {
    let start = text
        .to_ascii_lowercase()
        .find(&query.to_ascii_lowercase())?;
    let left = text[..start]
        .char_indices()
        .rev()
        .nth(48)
        .map_or(0, |(i, _)| i);
    let right = text[start + query.len()..]
        .char_indices()
        .nth(160)
        .map_or(text.len(), |(i, _)| start + query.len() + i);
    Some(format!(
        "{}{}{}",
        if left > 0 { "…" } else { "" },
        &text[left..right],
        if right < text.len() { "…" } else { "" }
    ))
}

fn bounded_title(title: &str, query: &str) -> String {
    if title.chars().take(201).count() <= 200 {
        return title.to_string();
    }
    excerpt(title, query).unwrap_or_else(|| title.chars().take(200).collect::<String>() + "…")
}

fn preview(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(240) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

fn search_knowledge_page(
    db_path: &std::path::Path,
    working_dir: &str,
    app_root: Option<&PathBuf>,
    query: &str,
    titles: bool,
    cursor: Option<&str>,
) -> Result<GlobalSearchPage, String> {
    // A doc ID is a keyset boundary, never a path supplied to the filesystem.
    if cursor.is_some_and(|value| value.len() > 1024) {
        return Err("Invalid search cursor".into());
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(250))
        .map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT doc_id, doc_type, doc_path, title, updated_at FROM document_catalog WHERE doc_id > ?1 ORDER BY doc_id LIMIT 256")
        .map_err(|e| e.to_string())?;
    let entries = stmt
        .query_map(params![cursor.unwrap_or("")], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    drop(conn);
    let started = Instant::now();
    let count = entries.len();
    let mut matches = Vec::new();
    let mut last = None;
    let mut bytes = 0;
    for (index, (id, kind, path, title, mut modified_at)) in entries.into_iter().enumerate() {
        let doc_type: KnowledgeType =
            serde_json::from_value(serde_json::Value::String(kind)).map_err(|e| e.to_string())?;
        // Only open title-matched documents when searching titles. Their second
        // line is a body match or a content preview, never a copy of the title.
        let snippet = if titles && excerpt(&title, query).is_none() {
            None
        } else {
            match knowledge_store::load_document_by_path_with_app_root(
                working_dir,
                app_root,
                doc_type,
                &path,
            ) {
                Ok(document) => {
                    modified_at = document.updated_at;
                    bytes += document.body.len();
                    excerpt(&document.body, query)
                        .or_else(|| {
                            document
                                .summary
                                .as_deref()
                                .and_then(|text| excerpt(text, query))
                        })
                        .or_else(|| {
                            document
                                .maintenance_rules
                                .as_deref()
                                .and_then(|text| excerpt(text, query))
                        })
                        .or_else(|| {
                            titles.then(|| {
                                preview(
                                    document
                                        .summary
                                        .as_deref()
                                        .filter(|text| !text.trim().is_empty())
                                        .unwrap_or(&document.body),
                                )
                            })
                        })
                }
                // Watcher reconciliation may lag a concurrent deletion.
                Err(error)
                    if error.contains("not found")
                        || error.contains("os error 2")
                        || error.contains("os error 3") =>
                {
                    None
                }
                Err(error) => return Err(error),
            }
        };
        last = Some(id.clone());
        if let Some(excerpt) = snippet {
            matches.push(GlobalSearchHit {
                kind: "knowledge",
                id,
                title: bounded_title(&title, query),
                excerpt,
                field: if titles { "title" } else { "content" },
                doc_type: Some(doc_type),
                path: Some(path),
                message_id: None,
                checkout_id: None,
                archived: false,
                modified_at,
            });
        }
        if matches.len() >= 20
            || started.elapsed() >= Duration::from_millis(60)
            || bytes >= 4 * 1024 * 1024
        {
            return Ok(GlobalSearchPage {
                matches,
                next_cursor: if index + 1 < count || count == 256 {
                    last
                } else {
                    None
                },
            });
        }
    }
    Ok(GlobalSearchPage {
        matches,
        next_cursor: if count == 256 { last } else { None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snippets_preserve_unicode_and_literal_punctuation() {
        let text = format!("{}Shader(foo)中文🙂{}", "前".repeat(200), "后".repeat(300));
        let snippet = excerpt(&text, "shader(foo)中文🙂").unwrap();
        assert!(snippet.contains("Shader(foo)中文🙂"));
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
        assert!(snippet.chars().count() < 240);
        assert!(excerpt(&text, "shader.*").is_none());
    }

    #[test]
    fn knowledge_search_reads_unindexed_content_and_resumes_titles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let db_path = dir.path().join("catalog.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE document_catalog(doc_id TEXT PRIMARY KEY, doc_type TEXT, doc_path TEXT, title TEXT, updated_at INTEGER)").unwrap();
        for i in 0..45 {
            let path = format!("document-{i:03}.md");
            let file = knowledge_store::document_path(&root, KnowledgeType::Design, &path).unwrap();
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                file,
                format!("---\nid: doc-{i:03}\ntitle: 标题 {i}\n---\n正文 Needle 中文🙂 {i}"),
            )
            .unwrap();
            conn.execute(
                "INSERT INTO document_catalog VALUES (?1, 'design', ?2, ?3, 1700000000123)",
                params![format!("doc-{i:03}"), path, format!("标题 {i}")],
            )
            .unwrap();
        }
        let mut cursor = None;
        let mut ids = std::collections::HashSet::new();
        loop {
            let page =
                search_knowledge_page(&db_path, &root, None, "标题", true, cursor.as_deref())
                    .unwrap();
            assert!(page.matches.len() <= 20);
            for hit in page.matches {
                let document = knowledge_store::load_document_by_path_with_app_root(
                    &root,
                    None,
                    KnowledgeType::Design,
                    hit.path.as_deref().unwrap(),
                )
                .unwrap();
                assert_eq!(hit.modified_at, document.updated_at);
                assert_eq!(hit.excerpt, document.body.trim());
                assert!(ids.insert(hit.id));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 45);
        let page =
            search_knowledge_page(&db_path, &root, None, "needle 中文🙂", false, None).unwrap();
        assert!(!page.matches.is_empty());
        for hit in &page.matches {
            let document = knowledge_store::load_document_by_path_with_app_root(
                &root,
                None,
                KnowledgeType::Design,
                hit.path.as_deref().unwrap(),
            )
            .unwrap();
            assert_eq!(hit.modified_at, document.updated_at);
        }
        assert!(page
            .matches
            .iter()
            .all(|hit| hit.excerpt.contains("Needle 中文🙂")));
        assert!(
            search_knowledge_page(&db_path, &root, None, "needle", true, None)
                .unwrap()
                .matches
                .is_empty()
        );
        assert!(
            search_knowledge_page(&db_path, &root, None, "%', OR 1=1--", true, None)
                .unwrap()
                .matches
                .is_empty()
        );
    }

    #[test]
    fn knowledge_title_hits_prefer_content_matches_then_summary_or_body_preview() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let db_path = dir.path().join("catalog.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE document_catalog(doc_id TEXT PRIMARY KEY, doc_type TEXT, doc_path TEXT, title TEXT, updated_at INTEGER)").unwrap();
        for (id, summary, body) in [
            (
                "a",
                "summary: Summary preview\n",
                format!("{}Needle body match", "前".repeat(300)),
            ),
            ("b", "summary: Summary preview\n", "Body preview".into()),
            ("c", "", format!("正文 {}", "中文🙂".repeat(200))),
            ("d", "", String::new()),
        ] {
            let path = format!("{id}.md");
            let file = knowledge_store::document_path(&root, KnowledgeType::Design, &path).unwrap();
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                file,
                format!("---\nid: {id}\ntitle: Needle document\n{summary}---\n## Content\n{body}"),
            )
            .unwrap();
            conn.execute(
                "INSERT INTO document_catalog VALUES (?1, 'design', ?2, 'Needle document', 1)",
                params![id, path],
            )
            .unwrap();
        }
        let mut hits = Vec::new();
        let mut cursor = None;
        loop {
            let page =
                search_knowledge_page(&db_path, &root, None, "needle", true, cursor.as_deref())
                    .unwrap();
            hits.extend(page.matches);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(hits.len(), 4);
        assert!(hits[0].excerpt.starts_with('…'));
        assert!(hits[0].excerpt.contains("Needle body match"));
        assert!(!hits[0].excerpt.contains("Summary preview"));
        assert_eq!(hits[1].excerpt, "Summary preview");
        assert!(hits[2].excerpt.starts_with("正文 中文🙂"));
        assert_eq!(hits[2].excerpt.chars().count(), 241);
        assert!(hits[2].excerpt.ends_with('…'));
        assert!(hits[3].excerpt.is_empty());
        let page = search_knowledge_page(&db_path, &root, None, "needle", false, None).unwrap();
        assert_eq!(page.matches.len(), 1);
        assert_eq!(page.matches[0].id, "a");
    }
}
