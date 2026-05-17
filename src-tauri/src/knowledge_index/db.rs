use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

const KNOWLEDGE_DB_VERSION: u32 = 6;

#[derive(Debug, Clone)]
pub struct DocumentCatalogRow {
    pub doc_id: String,
    pub doc_type: String,
    pub doc_path: String,
    pub parent_path: Option<String>,
    pub title: String,
    pub updated_at: i64,
    pub estimated_tokens: u64,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedDirectorySnapshotRow {
    pub managed_path: String,
    pub fingerprint: String,
    pub document_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedRetrievalSummaryCacheRow {
    pub managed_path: String,
    pub fingerprint: String,
    pub config_signature: String,
    pub total_docs: usize,
    pub lexical_enabled_docs: usize,
    pub vector_enabled_docs: usize,
    pub lexical_fresh_docs: usize,
    pub lexical_stale_docs: usize,
    pub fresh_enabled_docs: usize,
    pub chunk_count: usize,
    pub embedded_chunk_count: usize,
    pub embedded_doc_count: usize,
    pub updated_at: i64,
}

pub struct KnowledgeDb {
    conn: Arc<Mutex<Connection>>,
}

impl KnowledgeDb {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create knowledge db dir: {}", e))?;
        }
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open knowledge index db: {}", e))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;",
        )
        .map_err(|e| format!("Failed to configure knowledge db pragmas: {}", e))?;
        let schema_version = read_user_version(&conn).unwrap_or(0);
        migrate_schema(&conn, schema_version)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_or_recover(db_path: &Path) -> Result<Self, String> {
        match Self::open(db_path) {
            Ok(db) => Ok(db),
            Err(initial_err) => {
                if !has_db_artifacts(db_path) {
                    return Err(initial_err);
                }
                delete_db_artifacts(db_path);
                eprintln!("[Locus] knowledge index db init failed; cache artifacts removed");
                Self::open(db_path).map_err(|reopen_err| {
                    format!(
                        "{}; rebuilt knowledge index db cache but reopen still failed: {}",
                        initial_err, reopen_err
                    )
                })
            }
        }
    }

    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    pub fn get_index_state(&self, doc_id: &str) -> Result<Option<DocIndexState>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT doc_id, doc_type, doc_path, title_hash, summary_hash, body_hash,
                        rules_hash, index_version, embedding_backend, stale
                 FROM document_index_state
                 WHERE doc_id = ?1",
            )
            .map_err(|e| e.to_string())?;

        let result = stmt
            .query_row(params![doc_id], |row| {
                Ok(DocIndexState {
                    doc_id: row.get(0)?,
                    doc_type: row.get(1)?,
                    doc_path: row.get(2)?,
                    title_hash: row.get(3)?,
                    summary_hash: row.get(4)?,
                    body_hash: row.get(5)?,
                    rules_hash: row.get(6)?,
                    index_version: row.get(7)?,
                    embedding_backend: row.get(8)?,
                    stale: row.get(9)?,
                })
            })
            .ok();
        Ok(result)
    }

    pub fn upsert_index_state(&self, state: &DocIndexState) -> Result<(), String> {
        self.upsert_index_states(std::slice::from_ref(state))
    }

    pub fn delete_index_state(&self, doc_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM document_index_state WHERE doc_id = ?1",
            params![doc_id],
        )
        .map_err(|e| format!("Failed to delete knowledge index state: {}", e))?;
        Ok(())
    }

    pub fn list_all_index_states(&self) -> Result<Vec<DocIndexState>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT doc_id, doc_type, doc_path, title_hash, summary_hash, body_hash,
                        rules_hash, index_version, embedding_backend, stale
                 FROM document_index_state",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                Ok(DocIndexState {
                    doc_id: row.get(0)?,
                    doc_type: row.get(1)?,
                    doc_path: row.get(2)?,
                    title_hash: row.get(3)?,
                    summary_hash: row.get(4)?,
                    body_hash: row.get(5)?,
                    rules_hash: row.get(6)?,
                    index_version: row.get(7)?,
                    embedding_backend: row.get(8)?,
                    stale: row.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn replace_chunks(&self, doc_id: &str, chunks: &[ChunkRecord]) -> Result<Vec<i64>, String> {
        self.replace_chunks_and_embeddings(doc_id, chunks, None)
    }

    pub fn get_chunks(&self, doc_id: &str) -> Result<Vec<ChunkRow>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, doc_id, section, seq, text, text_hash
                 FROM document_chunks
                 WHERE doc_id = ?1
                 ORDER BY section, seq",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![doc_id], |row| {
                Ok(ChunkRow {
                    id: row.get(0)?,
                    doc_id: row.get(1)?,
                    section: row.get(2)?,
                    seq: row.get(3)?,
                    text: row.get(4)?,
                    text_hash: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn count_chunks_for_fresh_docs(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM document_chunks c
                 JOIN document_index_state s ON s.doc_id = c.doc_id
                 WHERE s.stale = 0",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn count_chunks_for_fresh_docs_with_prefix(
        &self,
        doc_type: &str,
        doc_path_prefix: &str,
    ) -> Result<usize, String> {
        let (exact_prefix, like_prefix) = doc_path_prefix_params(doc_path_prefix);
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM document_chunks c
                 JOIN document_index_state s ON s.doc_id = c.doc_id
                 WHERE s.stale = 0
                   AND s.doc_type = ?1
                   AND (s.doc_path = ?2 OR s.doc_path LIKE ?3)",
                params![doc_type, exact_prefix, like_prefix],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn count_chunks_for_doc(&self, doc_id: &str) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM document_chunks
                 WHERE doc_id = ?1",
                params![doc_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn upsert_embedding(
        &self,
        chunk_id: i64,
        vector: &[f32],
        dimension: usize,
    ) -> Result<(), String> {
        let blob = vector_to_blob(vector);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO document_embeddings (chunk_id, vector, dimension)
             VALUES (?1, ?2, ?3)",
            params![chunk_id, blob, dimension as i32],
        )
        .map_err(|e| format!("Failed to upsert knowledge embedding: {}", e))?;
        Ok(())
    }

    pub fn load_all_embeddings(&self) -> Result<Vec<EmbeddingRow>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT e.chunk_id, c.doc_id, c.section, c.seq, e.vector, e.dimension
                 FROM document_embeddings e
                 JOIN document_chunks c ON c.id = e.chunk_id
                 JOIN document_index_state s ON s.doc_id = c.doc_id
                 WHERE s.stale = 0",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                let blob: Vec<u8> = row.get(4)?;
                let dim: i32 = row.get(5)?;
                Ok(EmbeddingRow {
                    chunk_id: row.get(0)?,
                    doc_id: row.get(1)?,
                    section: row.get(2)?,
                    seq: row.get(3)?,
                    vector: blob_to_vector(&blob, dim as usize),
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn count_embeddings_for_fresh_docs(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM document_embeddings e
                 JOIN document_chunks c ON c.id = e.chunk_id
                 JOIN document_index_state s ON s.doc_id = c.doc_id
                 WHERE s.stale = 0",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn count_embeddings_for_fresh_docs_with_prefix(
        &self,
        doc_type: &str,
        doc_path_prefix: &str,
    ) -> Result<usize, String> {
        let (exact_prefix, like_prefix) = doc_path_prefix_params(doc_path_prefix);
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM document_embeddings e
                 JOIN document_chunks c ON c.id = e.chunk_id
                 JOIN document_index_state s ON s.doc_id = c.doc_id
                 WHERE s.stale = 0
                   AND s.doc_type = ?1
                   AND (s.doc_path = ?2 OR s.doc_path LIKE ?3)",
                params![doc_type, exact_prefix, like_prefix],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn count_distinct_docs_with_embeddings_for_fresh_docs(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT c.doc_id)
                 FROM document_embeddings e
                 JOIN document_chunks c ON c.id = e.chunk_id
                 JOIN document_index_state s ON s.doc_id = c.doc_id
                 WHERE s.stale = 0",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn count_distinct_docs_with_embeddings_for_fresh_docs_with_prefix(
        &self,
        doc_type: &str,
        doc_path_prefix: &str,
    ) -> Result<usize, String> {
        let (exact_prefix, like_prefix) = doc_path_prefix_params(doc_path_prefix);
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT c.doc_id)
                 FROM document_embeddings e
                 JOIN document_chunks c ON c.id = e.chunk_id
                 JOIN document_index_state s ON s.doc_id = c.doc_id
                 WHERE s.stale = 0
                   AND s.doc_type = ?1
                   AND (s.doc_path = ?2 OR s.doc_path LIKE ?3)",
                params![doc_type, exact_prefix, like_prefix],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn count_embeddings_for_doc(&self, doc_id: &str) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM document_embeddings e
                 JOIN document_chunks c ON c.id = e.chunk_id
                 WHERE c.doc_id = ?1",
                params![doc_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn delete_embeddings_for_doc(&self, doc_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM document_embeddings WHERE chunk_id IN
             (SELECT id FROM document_chunks WHERE doc_id = ?1)",
            params![doc_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn count_document_catalog_entries(&self) -> Result<usize, String> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM document_catalog", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        Ok(count.max(0) as usize)
    }

    pub fn list_document_catalog_entries(
        &self,
        doc_type: Option<&str>,
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        let conn = self.conn.lock().unwrap();
        let (sql, params) = if let Some(doc_type) = doc_type {
            (
                "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                 FROM document_catalog
                 WHERE doc_type = ?1
                 ORDER BY doc_type, doc_path, title, doc_id"
                    .to_string(),
                vec![doc_type.to_string()],
            )
        } else {
            (
                "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                 FROM document_catalog
                 ORDER BY doc_type, doc_path, title, doc_id"
                    .to_string(),
                Vec::new(),
            )
        };
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        query_document_catalog_rows(&mut stmt, params_from_iter(params.iter()))
    }

    pub fn list_document_catalog_entries_filtered(
        &self,
        doc_type: Option<&str>,
        path_prefix: Option<&str>,
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        if doc_type.is_none() && path_prefix.is_none() {
            return self.list_document_catalog_entries(None);
        }

        let conn = self.conn.lock().unwrap();
        let rows = match (doc_type, path_prefix) {
            (Some(doc_type), Some(path_prefix)) => {
                let (exact_prefix, like_prefix) = doc_path_prefix_params(path_prefix);
                let mut stmt = conn
                    .prepare(
                        "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                         FROM document_catalog
                         WHERE doc_type = ?1 AND (doc_path = ?2 OR doc_path LIKE ?3)
                         ORDER BY doc_type, doc_path, title, doc_id",
                    )
                    .map_err(|e| e.to_string())?;
                query_document_catalog_rows(
                    &mut stmt,
                    params![doc_type, exact_prefix, like_prefix],
                )?
            }
            (Some(doc_type), None) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                         FROM document_catalog
                         WHERE doc_type = ?1
                         ORDER BY doc_type, doc_path, title, doc_id",
                    )
                    .map_err(|e| e.to_string())?;
                query_document_catalog_rows(&mut stmt, params![doc_type])?
            }
            (None, Some(path_prefix)) => {
                let (exact_prefix, like_prefix) = doc_path_prefix_params(path_prefix);
                let mut stmt = conn
                    .prepare(
                        "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                         FROM document_catalog
                         WHERE doc_path = ?1 OR doc_path LIKE ?2
                         ORDER BY doc_type, doc_path, title, doc_id",
                    )
                    .map_err(|e| e.to_string())?;
                query_document_catalog_rows(&mut stmt, params![exact_prefix, like_prefix])?
            }
            (None, None) => unreachable!("handled above"),
        };
        Ok(rows)
    }

    pub fn list_document_catalog_entries_page(
        &self,
        doc_type: Option<&str>,
        path_prefix: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        let query_limit = limit.saturating_add(1).min(i64::MAX as usize) as i64;
        let query_offset = offset.min(i64::MAX as usize) as i64;
        let conn = self.conn.lock().unwrap();
        match (doc_type, path_prefix) {
            (Some(doc_type), Some(path_prefix)) => {
                let (exact_prefix, like_prefix) = doc_path_prefix_params(path_prefix);
                let mut stmt = conn
                    .prepare(
                        "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                         FROM document_catalog
                         WHERE doc_type = ?1 AND (doc_path = ?2 OR doc_path LIKE ?3)
                         ORDER BY doc_type, doc_path, title, doc_id
                         LIMIT ?4 OFFSET ?5",
                    )
                    .map_err(|e| e.to_string())?;
                query_document_catalog_rows(
                    &mut stmt,
                    params![
                        doc_type,
                        exact_prefix,
                        like_prefix,
                        query_limit,
                        query_offset
                    ],
                )
            }
            (Some(doc_type), None) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                         FROM document_catalog
                         WHERE doc_type = ?1
                         ORDER BY doc_type, doc_path, title, doc_id
                         LIMIT ?2 OFFSET ?3",
                    )
                    .map_err(|e| e.to_string())?;
                query_document_catalog_rows(&mut stmt, params![doc_type, query_limit, query_offset])
            }
            (None, Some(path_prefix)) => {
                let (exact_prefix, like_prefix) = doc_path_prefix_params(path_prefix);
                let mut stmt = conn
                    .prepare(
                        "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                         FROM document_catalog
                         WHERE doc_path = ?1 OR doc_path LIKE ?2
                         ORDER BY doc_type, doc_path, title, doc_id
                         LIMIT ?3 OFFSET ?4",
                    )
                    .map_err(|e| e.to_string())?;
                query_document_catalog_rows(
                    &mut stmt,
                    params![exact_prefix, like_prefix, query_limit, query_offset],
                )
            }
            (None, None) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                         FROM document_catalog
                         ORDER BY doc_type, doc_path, title, doc_id
                         LIMIT ?1 OFFSET ?2",
                    )
                    .map_err(|e| e.to_string())?;
                query_document_catalog_rows(&mut stmt, params![query_limit, query_offset])
            }
        }
    }

    pub fn get_document_catalog_entries(
        &self,
        doc_ids: &[String],
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        if doc_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = sql_placeholders(doc_ids.len());
        let sql = format!(
            "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
             FROM document_catalog
             WHERE doc_id IN ({})
             ORDER BY doc_type, doc_path, title, doc_id",
            placeholders
        );
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        query_document_catalog_rows(&mut stmt, params_from_iter(doc_ids.iter()))
    }

    pub fn find_document_catalog_entries_by_path(
        &self,
        doc_type: &str,
        doc_path: &str,
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                 FROM document_catalog
                 WHERE doc_type = ?1 AND doc_path = ?2
                 ORDER BY updated_at DESC, title, doc_id",
            )
            .map_err(|e| e.to_string())?;
        query_document_catalog_rows(&mut stmt, params![doc_type, doc_path])
    }

    pub fn list_document_catalog_entries_with_prefix(
        &self,
        doc_type: &str,
        doc_path_prefix: &str,
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        let (exact_prefix, like_prefix) = doc_path_prefix_params(doc_path_prefix);
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                 FROM document_catalog
                 WHERE doc_type = ?1 AND (doc_path = ?2 OR doc_path LIKE ?3)
                 ORDER BY doc_path, title, doc_id",
            )
            .map_err(|e| e.to_string())?;
        query_document_catalog_rows(&mut stmt, params![doc_type, exact_prefix, like_prefix])
    }

    pub fn list_document_catalog_directory_entries(
        &self,
        doc_type: &str,
        parent_path: Option<&str>,
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        let conn = self.conn.lock().unwrap();
        let rows = if let Some(parent_path) = parent_path {
            let mut stmt = conn
                .prepare(
                    "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                     FROM document_catalog
                     WHERE doc_type = ?1 AND parent_path = ?2
                     ORDER BY doc_path, title, doc_id",
                )
                .map_err(|e| e.to_string())?;
            query_document_catalog_rows(&mut stmt, params![doc_type, parent_path])?
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                     FROM document_catalog
                     WHERE doc_type = ?1 AND parent_path IS NULL
                     ORDER BY doc_path, title, doc_id",
                )
                .map_err(|e| e.to_string())?;
            query_document_catalog_rows(&mut stmt, params![doc_type])?
        };
        Ok(rows)
    }

    pub fn list_document_catalog_directory_entries_page(
        &self,
        doc_type: &str,
        parent_path: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<DocumentCatalogRow>, String> {
        let query_limit = limit.saturating_add(1).min(i64::MAX as usize) as i64;
        let query_offset = offset.min(i64::MAX as usize) as i64;
        let conn = self.conn.lock().unwrap();
        if let Some(parent_path) = parent_path {
            let mut stmt = conn
                .prepare(
                    "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                     FROM document_catalog
                     WHERE doc_type = ?1 AND parent_path = ?2
                     ORDER BY doc_path, title, doc_id
                     LIMIT ?3 OFFSET ?4",
                )
                .map_err(|e| e.to_string())?;
            query_document_catalog_rows(
                &mut stmt,
                params![doc_type, parent_path, query_limit, query_offset],
            )
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
                     FROM document_catalog
                     WHERE doc_type = ?1 AND parent_path IS NULL
                     ORDER BY doc_path, title, doc_id
                     LIMIT ?2 OFFSET ?3",
                )
                .map_err(|e| e.to_string())?;
            query_document_catalog_rows(&mut stmt, params![doc_type, query_limit, query_offset])
        }
    }

    pub fn upsert_document_catalog_entries(
        &self,
        entries: &[DocumentCatalogRow],
    ) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO document_catalog
                     (doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(doc_id) DO UPDATE SET
                        doc_type = excluded.doc_type,
                        doc_path = excluded.doc_path,
                        parent_path = excluded.parent_path,
                        title = excluded.title,
                        updated_at = excluded.updated_at,
                        estimated_tokens = excluded.estimated_tokens,
                        payload_json = excluded.payload_json",
                )
                .map_err(|e| e.to_string())?;
            for entry in entries {
                stmt.execute(params![
                    entry.doc_id,
                    entry.doc_type,
                    entry.doc_path,
                    entry.parent_path,
                    entry.title,
                    entry.updated_at,
                    entry.estimated_tokens as i64,
                    entry.payload_json,
                ])
                .map_err(|e| format!("Failed to upsert knowledge document catalog: {}", e))?;
            }
        }
        tx.commit()
            .map_err(|e| format!("Failed to commit knowledge document catalog upsert: {}", e))
    }

    pub fn delete_documents(&self, doc_ids: &[String]) -> Result<(), String> {
        if doc_ids.is_empty() {
            return Ok(());
        }

        let placeholders = sql_placeholders(doc_ids.len());
        let delete_embeddings_sql = format!(
            "DELETE FROM document_embeddings
             WHERE chunk_id IN (
                SELECT id FROM document_chunks WHERE doc_id IN ({})
             )",
            placeholders
        );
        let delete_chunks_sql = format!(
            "DELETE FROM document_chunks WHERE doc_id IN ({})",
            placeholders
        );
        let delete_states_sql = format!(
            "DELETE FROM document_index_state WHERE doc_id IN ({})",
            placeholders
        );
        let delete_catalog_sql = format!(
            "DELETE FROM document_catalog WHERE doc_id IN ({})",
            placeholders
        );

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(&delete_embeddings_sql, params_from_iter(doc_ids.iter()))
            .map_err(|e| format!("Failed to delete knowledge embeddings: {}", e))?;
        tx.execute(&delete_chunks_sql, params_from_iter(doc_ids.iter()))
            .map_err(|e| format!("Failed to delete knowledge chunks: {}", e))?;
        tx.execute(&delete_states_sql, params_from_iter(doc_ids.iter()))
            .map_err(|e| format!("Failed to delete knowledge index state: {}", e))?;
        tx.execute(&delete_catalog_sql, params_from_iter(doc_ids.iter()))
            .map_err(|e| format!("Failed to delete knowledge catalog rows: {}", e))?;
        tx.commit()
            .map_err(|e| format!("Failed to commit knowledge document removal: {}", e))
    }

    pub fn upsert_index_states(&self, states: &[DocIndexState]) -> Result<(), String> {
        if states.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO document_index_state
                     (doc_id, doc_type, doc_path, title_hash, summary_hash, body_hash,
                      rules_hash, index_version, embedding_backend, stale)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|e| e.to_string())?;
            for state in states {
                stmt.execute(params![
                    state.doc_id,
                    state.doc_type,
                    state.doc_path,
                    state.title_hash,
                    state.summary_hash,
                    state.body_hash,
                    state.rules_hash,
                    state.index_version,
                    state.embedding_backend,
                    state.stale,
                ])
                .map_err(|e| format!("Failed to upsert knowledge index state: {}", e))?;
            }
        }
        tx.commit()
            .map_err(|e| format!("Failed to commit knowledge index state upsert: {}", e))
    }

    pub fn list_index_states_with_prefix(
        &self,
        doc_type: &str,
        doc_path_prefix: &str,
    ) -> Result<Vec<DocIndexState>, String> {
        let (exact_prefix, like_prefix) = doc_path_prefix_params(doc_path_prefix);
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT doc_id, doc_type, doc_path, title_hash, summary_hash, body_hash,
                        rules_hash, index_version, embedding_backend, stale
                 FROM document_index_state
                 WHERE doc_type = ?1 AND (doc_path = ?2 OR doc_path LIKE ?3)",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![doc_type, exact_prefix, like_prefix], |row| {
                Ok(DocIndexState {
                    doc_id: row.get(0)?,
                    doc_type: row.get(1)?,
                    doc_path: row.get(2)?,
                    title_hash: row.get(3)?,
                    summary_hash: row.get(4)?,
                    body_hash: row.get(5)?,
                    rules_hash: row.get(6)?,
                    index_version: row.get(7)?,
                    embedding_backend: row.get(8)?,
                    stale: row.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn list_docs_missing_embeddings_with_prefix(
        &self,
        doc_type: &str,
        doc_path_prefix: &str,
    ) -> Result<HashSet<String>, String> {
        let (exact_prefix, like_prefix) = doc_path_prefix_params(doc_path_prefix);
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT s.doc_id
                 FROM document_index_state s
                 LEFT JOIN document_chunks c ON c.doc_id = s.doc_id
                 LEFT JOIN document_embeddings e ON e.chunk_id = c.id
                 WHERE s.doc_type = ?1 AND (s.doc_path = ?2 OR s.doc_path LIKE ?3)
                 GROUP BY s.doc_id
                 HAVING COUNT(c.id) > 0 AND COUNT(e.chunk_id) < COUNT(c.id)",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![doc_type, exact_prefix, like_prefix], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<HashSet<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn get_managed_retrieval_summary_cache(
        &self,
        managed_path: &str,
    ) -> Result<Option<ManagedRetrievalSummaryCacheRow>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT managed_path, fingerprint, config_signature, total_docs,
                        lexical_enabled_docs, vector_enabled_docs, lexical_fresh_docs,
                        lexical_stale_docs, fresh_enabled_docs, chunk_count,
                        embedded_chunk_count, embedded_doc_count, updated_at
                 FROM managed_retrieval_summary_cache
                 WHERE managed_path = ?1",
            )
            .map_err(|e| e.to_string())?;
        let row = stmt
            .query_row(params![managed_path], |row| {
                Ok(ManagedRetrievalSummaryCacheRow {
                    managed_path: row.get(0)?,
                    fingerprint: row.get(1)?,
                    config_signature: row.get(2)?,
                    total_docs: row.get::<_, i64>(3)?.max(0) as usize,
                    lexical_enabled_docs: row.get::<_, i64>(4)?.max(0) as usize,
                    vector_enabled_docs: row.get::<_, i64>(5)?.max(0) as usize,
                    lexical_fresh_docs: row.get::<_, i64>(6)?.max(0) as usize,
                    lexical_stale_docs: row.get::<_, i64>(7)?.max(0) as usize,
                    fresh_enabled_docs: row.get::<_, i64>(8)?.max(0) as usize,
                    chunk_count: row.get::<_, i64>(9)?.max(0) as usize,
                    embedded_chunk_count: row.get::<_, i64>(10)?.max(0) as usize,
                    embedded_doc_count: row.get::<_, i64>(11)?.max(0) as usize,
                    updated_at: row.get(12)?,
                })
            })
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(row)
    }

    pub fn get_managed_directory_snapshot(
        &self,
        managed_path: &str,
    ) -> Result<Option<ManagedDirectorySnapshotRow>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT managed_path, fingerprint, document_count
                 FROM managed_directory_snapshot
                 WHERE managed_path = ?1",
            )
            .map_err(|e| e.to_string())?;
        let row = stmt
            .query_row(params![managed_path], |row| {
                Ok(ManagedDirectorySnapshotRow {
                    managed_path: row.get(0)?,
                    fingerprint: row.get(1)?,
                    document_count: row.get::<_, i64>(2)?.max(0) as usize,
                })
            })
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(row)
    }

    pub fn upsert_managed_directory_snapshot(
        &self,
        snapshot: &ManagedDirectorySnapshotRow,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO managed_directory_snapshot (managed_path, fingerprint, document_count)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(managed_path) DO UPDATE SET
                fingerprint = excluded.fingerprint,
                document_count = excluded.document_count",
            params![
                snapshot.managed_path,
                snapshot.fingerprint,
                snapshot.document_count as i64,
            ],
        )
        .map_err(|e| format!("Failed to upsert managed directory snapshot: {}", e))?;
        Ok(())
    }

    pub fn delete_managed_directory_snapshot(&self, managed_path: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM managed_directory_snapshot WHERE managed_path = ?1",
            params![managed_path],
        )
        .map_err(|e| format!("Failed to delete managed directory snapshot: {}", e))?;
        Ok(())
    }

    pub fn upsert_managed_retrieval_summary_cache(
        &self,
        summary: &ManagedRetrievalSummaryCacheRow,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO managed_retrieval_summary_cache (
                managed_path, fingerprint, config_signature, total_docs,
                lexical_enabled_docs, vector_enabled_docs, lexical_fresh_docs,
                lexical_stale_docs, fresh_enabled_docs, chunk_count,
                embedded_chunk_count, embedded_doc_count, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(managed_path) DO UPDATE SET
                fingerprint = excluded.fingerprint,
                config_signature = excluded.config_signature,
                total_docs = excluded.total_docs,
                lexical_enabled_docs = excluded.lexical_enabled_docs,
                vector_enabled_docs = excluded.vector_enabled_docs,
                lexical_fresh_docs = excluded.lexical_fresh_docs,
                lexical_stale_docs = excluded.lexical_stale_docs,
                fresh_enabled_docs = excluded.fresh_enabled_docs,
                chunk_count = excluded.chunk_count,
                embedded_chunk_count = excluded.embedded_chunk_count,
                embedded_doc_count = excluded.embedded_doc_count,
                updated_at = excluded.updated_at",
            params![
                summary.managed_path,
                summary.fingerprint,
                summary.config_signature,
                summary.total_docs as i64,
                summary.lexical_enabled_docs as i64,
                summary.vector_enabled_docs as i64,
                summary.lexical_fresh_docs as i64,
                summary.lexical_stale_docs as i64,
                summary.fresh_enabled_docs as i64,
                summary.chunk_count as i64,
                summary.embedded_chunk_count as i64,
                summary.embedded_doc_count as i64,
                summary.updated_at,
            ],
        )
        .map_err(|e| format!("Failed to upsert managed retrieval summary cache: {}", e))?;
        Ok(())
    }

    pub fn delete_managed_retrieval_summary_cache(&self, managed_path: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM managed_retrieval_summary_cache WHERE managed_path = ?1",
            params![managed_path],
        )
        .map_err(|e| format!("Failed to delete managed retrieval summary cache: {}", e))?;
        Ok(())
    }

    pub fn replace_chunks_and_embeddings(
        &self,
        doc_id: &str,
        chunks: &[ChunkRecord],
        embeddings: Option<&[Vec<f32>]>,
    ) -> Result<Vec<i64>, String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM document_embeddings WHERE chunk_id IN
             (SELECT id FROM document_chunks WHERE doc_id = ?1)",
            params![doc_id],
        )
        .map_err(|e| format!("Failed to clear knowledge embeddings: {}", e))?;
        tx.execute(
            "DELETE FROM document_chunks WHERE doc_id = ?1",
            params![doc_id],
        )
        .map_err(|e| format!("Failed to clear knowledge chunks: {}", e))?;

        let mut ids = Vec::with_capacity(chunks.len());
        {
            let mut chunk_stmt = tx
                .prepare(
                    "INSERT INTO document_chunks (doc_id, section, seq, text, text_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(|e| e.to_string())?;
            let mut embedding_stmt = if embeddings.is_some() {
                Some(
                    tx.prepare(
                        "INSERT OR REPLACE INTO document_embeddings (chunk_id, vector, dimension)
                         VALUES (?1, ?2, ?3)",
                    )
                    .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };

            for (index, chunk) in chunks.iter().enumerate() {
                chunk_stmt
                    .execute(params![
                        doc_id,
                        chunk.section,
                        chunk.seq,
                        chunk.text,
                        chunk.text_hash
                    ])
                    .map_err(|e| format!("Failed to insert knowledge chunk: {}", e))?;
                let chunk_id = tx.last_insert_rowid();
                ids.push(chunk_id);

                if let (Some(vectors), Some(stmt)) = (embeddings, embedding_stmt.as_mut()) {
                    if let Some(vector) = vectors.get(index) {
                        stmt.execute(params![
                            chunk_id,
                            vector_to_blob(vector),
                            vector.len() as i32
                        ])
                        .map_err(|e| format!("Failed to insert knowledge embedding: {}", e))?;
                    }
                }
            }
        }

        tx.commit()
            .map_err(|e| format!("Failed to commit knowledge chunk update: {}", e))?;
        Ok(ids)
    }

    pub fn apply_document_updates(
        &self,
        updates: &[DocumentPersistUpdate<'_>],
    ) -> Result<(), String> {
        if updates.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut clear_embeddings_stmt = tx
                .prepare(
                    "DELETE FROM document_embeddings WHERE chunk_id IN
                     (SELECT id FROM document_chunks WHERE doc_id = ?1)",
                )
                .map_err(|e| e.to_string())?;
            let mut clear_chunks_stmt = tx
                .prepare("DELETE FROM document_chunks WHERE doc_id = ?1")
                .map_err(|e| e.to_string())?;
            let mut chunk_stmt = tx
                .prepare(
                    "INSERT INTO document_chunks (doc_id, section, seq, text, text_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(|e| e.to_string())?;
            let mut embedding_stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO document_embeddings (chunk_id, vector, dimension)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| e.to_string())?;
            let mut state_stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO document_index_state
                     (doc_id, doc_type, doc_path, title_hash, summary_hash, body_hash,
                      rules_hash, index_version, embedding_backend, stale)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|e| e.to_string())?;

            for update in updates {
                clear_embeddings_stmt
                    .execute(params![update.state.doc_id.as_str()])
                    .map_err(|e| format!("Failed to clear knowledge embeddings: {}", e))?;
                clear_chunks_stmt
                    .execute(params![update.state.doc_id.as_str()])
                    .map_err(|e| format!("Failed to clear knowledge chunks: {}", e))?;

                for (index, chunk) in update.chunks.iter().enumerate() {
                    chunk_stmt
                        .execute(params![
                            update.state.doc_id.as_str(),
                            chunk.section,
                            chunk.seq,
                            chunk.text,
                            chunk.text_hash
                        ])
                        .map_err(|e| format!("Failed to insert knowledge chunk: {}", e))?;
                    let chunk_id = tx.last_insert_rowid();

                    if let Some(vectors) = update.embeddings {
                        if let Some(vector) = vectors.get(index) {
                            embedding_stmt
                                .execute(params![
                                    chunk_id,
                                    vector_to_blob(vector),
                                    vector.len() as i32
                                ])
                                .map_err(|e| {
                                    format!("Failed to insert knowledge embedding: {}", e)
                                })?;
                        }
                    }
                }

                state_stmt
                    .execute(params![
                        update.state.doc_id,
                        update.state.doc_type,
                        update.state.doc_path,
                        update.state.title_hash,
                        update.state.summary_hash,
                        update.state.body_hash,
                        update.state.rules_hash,
                        update.state.index_version,
                        update.state.embedding_backend,
                        update.state.stale,
                    ])
                    .map_err(|e| format!("Failed to upsert knowledge index state: {}", e))?;
            }
        }
        tx.commit()
            .map_err(|e| format!("Failed to commit knowledge batch update: {}", e))
    }

    pub fn apply_embedding_backfill_updates(
        &self,
        updates: &[EmbeddingBackfillPersistUpdate<'_>],
    ) -> Result<(), String> {
        if updates.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut clear_embeddings_stmt = tx
                .prepare(
                    "DELETE FROM document_embeddings WHERE chunk_id IN
                     (SELECT id FROM document_chunks WHERE doc_id = ?1)",
                )
                .map_err(|e| e.to_string())?;
            let mut embedding_stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO document_embeddings (chunk_id, vector, dimension)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| e.to_string())?;
            let mut state_stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO document_index_state
                     (doc_id, doc_type, doc_path, title_hash, summary_hash, body_hash,
                      rules_hash, index_version, embedding_backend, stale)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|e| e.to_string())?;

            for update in updates {
                clear_embeddings_stmt
                    .execute(params![update.state.doc_id.as_str()])
                    .map_err(|e| format!("Failed to clear knowledge embeddings: {}", e))?;

                if let Some(vectors) = update.embeddings {
                    for (index, chunk) in update.chunks.iter().enumerate() {
                        if let Some(vector) = vectors.get(index) {
                            embedding_stmt
                                .execute(params![
                                    chunk.id,
                                    vector_to_blob(vector),
                                    vector.len() as i32
                                ])
                                .map_err(|e| {
                                    format!("Failed to insert knowledge embedding: {}", e)
                                })?;
                        }
                    }
                }

                state_stmt
                    .execute(params![
                        update.state.doc_id,
                        update.state.doc_type,
                        update.state.doc_path,
                        update.state.title_hash,
                        update.state.summary_hash,
                        update.state.body_hash,
                        update.state.rules_hash,
                        update.state.index_version,
                        update.state.embedding_backend,
                        update.state.stale,
                    ])
                    .map_err(|e| format!("Failed to upsert knowledge index state: {}", e))?;
            }
        }
        tx.commit()
            .map_err(|e| format!("Failed to commit knowledge embedding backfill: {}", e))
    }
}

fn has_db_artifacts(db_path: &Path) -> bool {
    db_path.exists()
        || sqlite_sidecar_path(db_path, "-wal").exists()
        || sqlite_sidecar_path(db_path, "-shm").exists()
        || sqlite_sidecar_path(db_path, "-journal").exists()
}

fn delete_db_artifacts(db_path: &Path) {
    let _ = std::fs::remove_file(db_path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(sqlite_sidecar_path(db_path, suffix));
    }
}

fn sqlite_sidecar_path(db_path: &Path, suffix: &str) -> std::path::PathBuf {
    let file_name = db_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("knowledge_index.db");
    db_path.with_file_name(format!("{}{}", file_name, suffix))
}

fn read_user_version(conn: &Connection) -> Result<u32, String> {
    conn.query_row("PRAGMA user_version", [], |row| {
        row.get::<_, i64>(0).map(|value| value as u32)
    })
    .map_err(|e| format!("Failed to read knowledge db schema version: {}", e))
}

#[derive(Debug, Clone)]
pub struct DocIndexState {
    pub doc_id: String,
    pub doc_type: String,
    pub doc_path: String,
    pub title_hash: Vec<u8>,
    pub summary_hash: Vec<u8>,
    pub body_hash: Vec<u8>,
    pub rules_hash: Vec<u8>,
    pub index_version: i32,
    pub embedding_backend: String,
    pub stale: i32,
}

#[derive(Debug, Clone)]
pub struct ChunkRecord {
    pub section: String,
    pub seq: i32,
    pub text: String,
    pub text_hash: Vec<u8>,
}

pub struct DocumentPersistUpdate<'a> {
    pub state: &'a DocIndexState,
    pub chunks: &'a [ChunkRecord],
    pub embeddings: Option<&'a [Vec<f32>]>,
}

pub struct EmbeddingBackfillPersistUpdate<'a> {
    pub state: &'a DocIndexState,
    pub chunks: &'a [ChunkRow],
    pub embeddings: Option<&'a [Vec<f32>]>,
}

#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub id: i64,
    pub doc_id: String,
    pub section: String,
    pub seq: i32,
    pub text: String,
    pub text_hash: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct EmbeddingRow {
    pub chunk_id: i64,
    pub doc_id: String,
    pub section: String,
    pub seq: i32,
    pub vector: Vec<f32>,
}

fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(vector.len() * 4);
    for &value in vector {
        buf.extend_from_slice(&value.to_le_bytes());
    }
    buf
}

fn blob_to_vector(blob: &[u8], dim: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(dim);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    out
}

fn create_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS document_index_state (
            doc_id TEXT PRIMARY KEY,
            doc_type TEXT NOT NULL,
            doc_path TEXT NOT NULL,
            title_hash BLOB NOT NULL,
            summary_hash BLOB NOT NULL,
            body_hash BLOB NOT NULL,
            rules_hash BLOB NOT NULL,
            index_version INTEGER NOT NULL,
            embedding_backend TEXT NOT NULL,
            stale INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS document_chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            doc_id TEXT NOT NULL,
            section TEXT NOT NULL,
            seq INTEGER NOT NULL,
            text TEXT NOT NULL,
            text_hash BLOB NOT NULL,
            UNIQUE(doc_id, section, seq)
        );

        CREATE TABLE IF NOT EXISTS document_embeddings (
            chunk_id INTEGER PRIMARY KEY REFERENCES document_chunks(id) ON DELETE CASCADE,
            vector BLOB NOT NULL,
            dimension INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS document_catalog (
            doc_id TEXT PRIMARY KEY,
            doc_type TEXT NOT NULL,
            doc_path TEXT NOT NULL,
            parent_path TEXT,
            title TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            estimated_tokens INTEGER NOT NULL DEFAULT 0,
            payload_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS managed_directory_snapshot (
            managed_path TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL,
            document_count INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS managed_retrieval_summary_cache (
            managed_path TEXT PRIMARY KEY,
            fingerprint TEXT NOT NULL,
            config_signature TEXT NOT NULL,
            total_docs INTEGER NOT NULL,
            lexical_enabled_docs INTEGER NOT NULL,
            vector_enabled_docs INTEGER NOT NULL,
            lexical_fresh_docs INTEGER NOT NULL,
            lexical_stale_docs INTEGER NOT NULL,
            fresh_enabled_docs INTEGER NOT NULL,
            chunk_count INTEGER NOT NULL,
            embedded_chunk_count INTEGER NOT NULL,
            embedded_doc_count INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_document_chunks_doc ON document_chunks(doc_id);
        CREATE INDEX IF NOT EXISTS idx_document_state_stale ON document_index_state(stale);
        CREATE INDEX IF NOT EXISTS idx_document_catalog_type_path
            ON document_catalog(doc_type, doc_path);
        CREATE INDEX IF NOT EXISTS idx_document_catalog_type_parent_path
            ON document_catalog(doc_type, parent_path, doc_path);",
    )
    .map_err(|e| format!("Failed to create knowledge index tables: {}", e))?;
    Ok(())
}

fn migrate_schema(conn: &Connection, schema_version: u32) -> Result<(), String> {
    if schema_version > KNOWLEDGE_DB_VERSION {
        return Err(format!(
            "Knowledge db schema version {} is newer than supported version {}",
            schema_version, KNOWLEDGE_DB_VERSION
        ));
    }

    if schema_version == 0 {
        create_tables(conn)?;
    }

    if schema_version < 2 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS document_catalog (
                doc_id TEXT PRIMARY KEY,
                doc_type TEXT NOT NULL,
                doc_path TEXT NOT NULL,
                parent_path TEXT,
                title TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                estimated_tokens INTEGER NOT NULL DEFAULT 0,
                payload_json TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_document_catalog_type_path
                ON document_catalog(doc_type, doc_path);
            CREATE INDEX IF NOT EXISTS idx_document_catalog_type_parent_path
                ON document_catalog(doc_type, parent_path, doc_path);",
        )
        .map_err(|e| format!("Failed to migrate knowledge db schema to v2: {}", e))?;
    }

    if schema_version < 3 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS managed_directory_snapshot (
                managed_path TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                document_count INTEGER NOT NULL
            );",
        )
        .map_err(|e| format!("Failed to migrate knowledge db schema to v3: {}", e))?;
    }

    if schema_version < 4 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS managed_retrieval_summary_cache (
                managed_path TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                config_signature TEXT NOT NULL,
                total_docs INTEGER NOT NULL,
                lexical_enabled_docs INTEGER NOT NULL,
                vector_enabled_docs INTEGER NOT NULL,
                lexical_fresh_docs INTEGER NOT NULL,
                lexical_stale_docs INTEGER NOT NULL,
                fresh_enabled_docs INTEGER NOT NULL,
                chunk_count INTEGER NOT NULL,
                embedded_chunk_count INTEGER NOT NULL,
                embedded_doc_count INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
        .map_err(|e| format!("Failed to migrate knowledge db schema to v4: {}", e))?;
    }

    if schema_version < 5 {
        ensure_document_catalog_parent_path_column(conn)?;
    }

    if schema_version < 6 {
        remove_document_catalog_scope_column(conn)?;
    }

    conn.execute_batch(&format!("PRAGMA user_version = {}", KNOWLEDGE_DB_VERSION))
        .map_err(|e| format!("Failed to set knowledge db schema version: {}", e))
}

fn map_document_catalog_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentCatalogRow> {
    Ok(DocumentCatalogRow {
        doc_id: row.get(0)?,
        doc_type: row.get(1)?,
        doc_path: row.get(2)?,
        parent_path: row.get(3)?,
        title: row.get(4)?,
        updated_at: row.get(5)?,
        estimated_tokens: row.get::<_, i64>(6)?.max(0) as u64,
        payload_json: row.get(7)?,
    })
}

fn query_document_catalog_rows<P: rusqlite::Params>(
    stmt: &mut rusqlite::Statement<'_>,
    params: P,
) -> Result<Vec<DocumentCatalogRow>, String> {
    let rows = stmt
        .query_map(params, map_document_catalog_row)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn ensure_document_catalog_parent_path_column(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(document_catalog)")
        .map_err(|e| e.to_string())?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if !columns.iter().any(|column| column == "parent_path") {
        conn.execute(
            "ALTER TABLE document_catalog ADD COLUMN parent_path TEXT",
            [],
        )
        .map_err(|e| format!("Failed to add document_catalog.parent_path: {}", e))?;
    }

    let mut select_stmt = conn
        .prepare("SELECT doc_id, doc_path FROM document_catalog")
        .map_err(|e| e.to_string())?;
    let rows = select_stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    for (doc_id, doc_path) in rows {
        conn.execute(
            "UPDATE document_catalog SET parent_path = ?2 WHERE doc_id = ?1",
            params![doc_id, parent_path_for_doc_path(&doc_path)],
        )
        .map_err(|e| format!("Failed to backfill document_catalog.parent_path: {}", e))?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_document_catalog_type_parent_path
            ON document_catalog(doc_type, parent_path, doc_path);",
    )
    .map_err(|e| format!("Failed to create document_catalog parent_path index: {}", e))
}

fn remove_document_catalog_scope_column(conn: &Connection) -> Result<(), String> {
    let has_scope_column = conn
        .prepare("PRAGMA table_info(document_catalog)")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .any(|name| name == "scope");
    if !has_scope_column {
        return Ok(());
    }

    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_document_catalog_type_path;
         DROP INDEX IF EXISTS idx_document_catalog_type_parent_path;

         CREATE TABLE document_catalog_next (
            doc_id TEXT PRIMARY KEY,
            doc_type TEXT NOT NULL,
            doc_path TEXT NOT NULL,
            parent_path TEXT,
            title TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            estimated_tokens INTEGER NOT NULL DEFAULT 0,
            payload_json TEXT NOT NULL
         );

         INSERT INTO document_catalog_next
            (doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json)
         SELECT doc_id, doc_type, doc_path, parent_path, title, updated_at, estimated_tokens, payload_json
         FROM document_catalog;

         DROP TABLE document_catalog;
         ALTER TABLE document_catalog_next RENAME TO document_catalog;

         CREATE INDEX IF NOT EXISTS idx_document_catalog_type_path
            ON document_catalog(doc_type, doc_path);
         CREATE INDEX IF NOT EXISTS idx_document_catalog_type_parent_path
            ON document_catalog(doc_type, parent_path, doc_path);",
    )
    .map_err(|e| format!("Failed to remove document_catalog.scope: {}", e))
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn doc_path_prefix_params(doc_path_prefix: &str) -> (String, String) {
    let exact = doc_path_prefix.trim().trim_matches('/').replace('\\', "/");
    let like = format!("{}/%", exact);
    (exact, like)
}

fn parent_path_for_doc_path(doc_path: &str) -> Option<String> {
    let normalized = doc_path.trim().trim_matches('/').replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }
    normalized
        .rsplit_once('/')
        .map(|(parent, _)| parent.trim_matches('/').to_string())
        .filter(|parent| !parent.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentCatalogRow, KnowledgeDb, ManagedDirectorySnapshotRow, KNOWLEDGE_DB_VERSION,
    };
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn create_v1_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE document_index_state (
                doc_id TEXT PRIMARY KEY,
                doc_type TEXT NOT NULL,
                doc_path TEXT NOT NULL,
                title_hash BLOB NOT NULL,
                summary_hash BLOB NOT NULL,
                body_hash BLOB NOT NULL,
                rules_hash BLOB NOT NULL,
                index_version INTEGER NOT NULL,
                embedding_backend TEXT NOT NULL,
                stale INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE document_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                doc_id TEXT NOT NULL,
                section TEXT NOT NULL,
                seq INTEGER NOT NULL,
                text TEXT NOT NULL,
                text_hash BLOB NOT NULL,
                UNIQUE(doc_id, section, seq)
            );

            CREATE TABLE document_embeddings (
                chunk_id INTEGER PRIMARY KEY REFERENCES document_chunks(id) ON DELETE CASCADE,
                vector BLOB NOT NULL,
                dimension INTEGER NOT NULL
            );

            CREATE INDEX idx_document_chunks_doc ON document_chunks(doc_id);
            CREATE INDEX idx_document_state_stale ON document_index_state(stale);
            PRAGMA user_version = 1;",
        )
        .expect("create v1 schema");
    }

    fn create_v4_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE document_catalog (
                doc_id TEXT PRIMARY KEY,
                doc_type TEXT NOT NULL,
                doc_path TEXT NOT NULL,
                scope TEXT NOT NULL,
                title TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                estimated_tokens INTEGER NOT NULL DEFAULT 0,
                payload_json TEXT NOT NULL
            );

            CREATE INDEX idx_document_catalog_type_path
                ON document_catalog(doc_type, doc_path);

            CREATE TABLE managed_directory_snapshot (
                managed_path TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                document_count INTEGER NOT NULL
            );

            CREATE TABLE managed_retrieval_summary_cache (
                managed_path TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                config_signature TEXT NOT NULL,
                total_docs INTEGER NOT NULL,
                lexical_enabled_docs INTEGER NOT NULL,
                vector_enabled_docs INTEGER NOT NULL,
                lexical_fresh_docs INTEGER NOT NULL,
                lexical_stale_docs INTEGER NOT NULL,
                fresh_enabled_docs INTEGER NOT NULL,
                chunk_count INTEGER NOT NULL,
                embedded_chunk_count INTEGER NOT NULL,
                embedded_doc_count INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            PRAGMA user_version = 4;",
        )
        .expect("create v4 schema");
    }

    #[test]
    fn open_migrates_v1_schema_to_latest_without_dropping_existing_rows() {
        let dir = tempdir().expect("temp dir");
        let db_path = dir.path().join("knowledge_index.db");
        let conn = Connection::open(&db_path).expect("open raw db");
        create_v1_schema(&conn);
        conn.execute(
            "INSERT INTO document_index_state
             (doc_id, doc_type, doc_path, title_hash, summary_hash, body_hash, rules_hash, index_version, embedding_backend, stale)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                "doc-1",
                "reference",
                "unity/api/application.md",
                vec![1_u8],
                vec![2_u8],
                vec![3_u8],
                vec![4_u8],
                1_i32,
                "none",
                0_i32,
            ],
        )
        .expect("seed state");
        drop(conn);

        let db = KnowledgeDb::open_or_recover(&db_path).expect("open migrated db");
        assert_eq!(db.list_all_index_states().expect("list states").len(), 1);
        assert_eq!(
            db.count_document_catalog_entries().expect("catalog count"),
            0
        );

        let conn = Connection::open(&db_path).expect("reopen raw db");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("read user version");
        assert_eq!(version as u32, KNOWLEDGE_DB_VERSION);
        let catalog_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'document_catalog'",
                [],
                |row| row.get(0),
            )
            .expect("check catalog table");
        assert_eq!(catalog_exists, 1);
        let snapshot_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'managed_directory_snapshot'",
                [],
                |row| row.get(0),
            )
            .expect("check snapshot table");
        assert_eq!(snapshot_exists, 1);
    }

    #[test]
    fn document_catalog_round_trip_preserves_payload() {
        let dir = tempdir().expect("temp dir");
        let db_path = dir.path().join("knowledge_index.db");
        let db = KnowledgeDb::open_or_recover(&db_path).expect("open db");
        let row = DocumentCatalogRow {
            doc_id: "doc-1".to_string(),
            doc_type: "reference".to_string(),
            doc_path: "unity/api/application.md".to_string(),
            parent_path: Some("unity/api".to_string()),
            title: "application".to_string(),
            updated_at: 42,
            estimated_tokens: 128,
            payload_json: "{\"id\":\"doc-1\"}".to_string(),
        };

        db.upsert_document_catalog_entries(std::slice::from_ref(&row))
            .expect("upsert catalog");
        let loaded = db
            .get_document_catalog_entries(&["doc-1".to_string()])
            .expect("load catalog");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].doc_id, row.doc_id);
        assert_eq!(loaded[0].doc_type, row.doc_type);
        assert_eq!(loaded[0].doc_path, row.doc_path);
        assert_eq!(loaded[0].parent_path, row.parent_path);
        assert_eq!(loaded[0].estimated_tokens, row.estimated_tokens);
        assert_eq!(loaded[0].payload_json, row.payload_json);
    }

    #[test]
    fn open_migrates_v4_catalog_and_backfills_parent_path() {
        let dir = tempdir().expect("temp dir");
        let db_path = dir.path().join("knowledge_index.db");
        let conn = Connection::open(&db_path).expect("open raw db");
        create_v4_schema(&conn);
        conn.execute(
            "INSERT INTO document_catalog
             (doc_id, doc_type, doc_path, scope, title, updated_at, estimated_tokens, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "doc-1",
                "reference",
                "unity/api/application.md",
                "project",
                "application",
                42_i64,
                128_i64,
                "{\"id\":\"doc-1\"}",
            ],
        )
        .expect("seed v4 catalog row");
        drop(conn);

        let db = KnowledgeDb::open_or_recover(&db_path).expect("open migrated db");
        let rows = db
            .list_document_catalog_directory_entries("reference", Some("unity/api"))
            .expect("directory rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].parent_path.as_deref(), Some("unity/api"));
        assert_eq!(rows[0].doc_path, "unity/api/application.md");
    }

    #[test]
    fn managed_directory_snapshot_round_trip_preserves_fingerprint() {
        let dir = tempdir().expect("temp dir");
        let db_path = dir.path().join("knowledge_index.db");
        let db = KnowledgeDb::open_or_recover(&db_path).expect("open db");
        let row = ManagedDirectorySnapshotRow {
            managed_path: "reference/unity-official-docs".to_string(),
            fingerprint: "fp-123".to_string(),
            document_count: 19494,
        };

        db.upsert_managed_directory_snapshot(&row)
            .expect("upsert managed snapshot");
        let loaded = db
            .get_managed_directory_snapshot(&row.managed_path)
            .expect("load managed snapshot")
            .expect("snapshot row");

        assert_eq!(loaded, row);
    }
}
