use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use tantivy::collector::TopDocs;
use tantivy::indexer::{IndexWriterOptions, UserOperation};
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, Term};

const TANTIVY_INDEX_FORMAT: &str = "tantivy-0.26-ngram-v2";
const TANTIVY_STAMP_FILE: &str = "locus_knowledge_tantivy_format";
const DEFAULT_WRITER_MEMORY_BUDGET_BYTES: usize = 150_000_000;
const UNITY_IMPORT_BULK_WRITER_THREADS: usize = 1;
const UNITY_IMPORT_BULK_WRITER_MEMORY_BUDGET_PER_THREAD_BYTES: usize = 512_000_000;
const UNITY_IMPORT_BULK_MERGE_THREADS: usize = 1;

#[derive(Debug, Clone)]
pub struct LexicalDocumentRecord {
    pub doc_id: String,
    pub title: String,
    pub path: String,
    pub keywords: String,
    pub chunks: Vec<(String, i32, String)>,
}

#[derive(Debug, Clone)]
pub struct LexicalHit {
    pub doc_id: String,
    pub section: String,
    pub title: String,
    pub path: String,
    pub score: f32,
    pub snippet: String,
    pub matched_terms: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexicalWriterProfile {
    Default,
    UnityImportBulk,
}

struct WriterHandle {
    writer: Option<IndexWriter>,
    profile: LexicalWriterProfile,
}

pub struct KnowledgeTantivyIndex {
    index_dir: PathBuf,
    index: Index,
    reader: IndexReader,
    writer: Mutex<WriterHandle>,
    f_doc_id: Field,
    f_title: Field,
    f_body: Field,
    f_path: Field,
    f_keywords: Field,
    f_seq: Field,
    f_section: Field,
}

pub struct LexicalBulkWriterGuard<'a> {
    index: &'a KnowledgeTantivyIndex,
    guard: MutexGuard<'a, WriterHandle>,
}

impl LexicalBulkWriterGuard<'_> {
    pub fn apply_grouped_batch(
        &mut self,
        removed_doc_ids: &[String],
        replaced_doc_ids: &[String],
        docs: &[LexicalDocumentRecord],
    ) -> Result<(), String> {
        self.index.apply_grouped_batch_locked(
            &mut self.guard,
            removed_doc_ids,
            replaced_doc_ids,
            docs,
        )
    }
}

impl Drop for LexicalBulkWriterGuard<'_> {
    fn drop(&mut self) {
        if let Err(error) = self
            .index
            .ensure_writer_profile(&mut self.guard, LexicalWriterProfile::Default)
        {
            eprintln!(
                "[KnowledgeIndex] failed to restore default tantivy writer profile: {}",
                error
            );
        }
    }
}

impl KnowledgeTantivyIndex {
    pub fn open(library_dir: &Path) -> Result<Self, String> {
        let index_dir = library_dir.join("knowledge_tantivy_index");
        std::fs::create_dir_all(&index_dir)
            .map_err(|e| format!("Failed to create knowledge tantivy dir: {}", e))?;

        let stamp_path = index_dir.join(TANTIVY_STAMP_FILE);
        let has_existing_payload = std::fs::read_dir(&index_dir)
            .map(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .map(|name| name != TANTIVY_STAMP_FILE)
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if has_existing_payload {
            let stamp = std::fs::read_to_string(&stamp_path).ok();
            let stamp_value = stamp.as_deref().map(str::trim);
            if stamp_value != Some(TANTIVY_INDEX_FORMAT) {
                return Err(format!(
                    "Knowledge tantivy format mismatch (expected {:?}, found {:?}); needs rebuild",
                    TANTIVY_INDEX_FORMAT, stamp_value
                ));
            }
        }

        let mut schema_builder = Schema::builder();
        let text_opts_stored = TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("cjk")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored();
        let text_opts_body = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("cjk")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );

        let f_doc_id = schema_builder.add_text_field("doc_id", STRING | STORED);
        let f_title = schema_builder.add_text_field("title", text_opts_stored.clone());
        let f_body = schema_builder.add_text_field("body", text_opts_body.clone());
        let f_path = schema_builder.add_text_field("path", text_opts_stored.clone());
        let f_keywords = schema_builder.add_text_field("keywords", text_opts_body);
        let f_seq = schema_builder.add_u64_field("seq", INDEXED | STORED);
        let f_section = schema_builder.add_text_field("section", STRING | STORED);
        let schema = schema_builder.build();

        let mmap_dir = tantivy::directory::MmapDirectory::open(&index_dir)
            .map_err(|e| format!("Failed to open knowledge tantivy mmap dir: {}", e))?;
        let index = Index::open_or_create(mmap_dir, schema)
            .map_err(|e| format!("Failed to open/create knowledge tantivy index: {}", e))?;
        let cjk_tokenizer = NgramTokenizer::new(2, 3, false)
            .map_err(|e| format!("Failed to configure knowledge tantivy tokenizer: {}", e))?;
        index.tokenizers().register(
            "cjk",
            TextAnalyzer::builder(cjk_tokenizer)
                .filter(LowerCaser)
                .build(),
        );

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| format!("Failed to create knowledge tantivy reader: {}", e))?;
        let writer = Self::build_writer(&index, LexicalWriterProfile::Default)?;

        if let Err(e) = std::fs::write(&stamp_path, TANTIVY_INDEX_FORMAT) {
            eprintln!(
                "[Locus] warning: failed to write knowledge tantivy stamp {}: {}",
                stamp_path.display(),
                e
            );
        }

        Ok(Self {
            index_dir,
            index,
            reader,
            writer: Mutex::new(WriterHandle {
                writer: Some(writer),
                profile: LexicalWriterProfile::Default,
            }),
            f_doc_id,
            f_title,
            f_body,
            f_path,
            f_keywords,
            f_seq,
            f_section,
        })
    }

    fn build_writer(index: &Index, profile: LexicalWriterProfile) -> Result<IndexWriter, String> {
        match profile {
            LexicalWriterProfile::Default => index
                .writer(DEFAULT_WRITER_MEMORY_BUDGET_BYTES)
                .map_err(|e| format!("Failed to create knowledge tantivy writer: {}", e)),
            LexicalWriterProfile::UnityImportBulk => index
                .writer_with_options(
                    IndexWriterOptions::builder()
                        .num_worker_threads(UNITY_IMPORT_BULK_WRITER_THREADS)
                        .memory_budget_per_thread(
                            UNITY_IMPORT_BULK_WRITER_MEMORY_BUDGET_PER_THREAD_BYTES,
                        )
                        .num_merge_threads(UNITY_IMPORT_BULK_MERGE_THREADS)
                        .build(),
                )
                .map_err(|e| format!("Failed to create Unity import bulk tantivy writer: {}", e)),
        }
    }

    fn ensure_writer_profile(
        &self,
        handle: &mut WriterHandle,
        profile: LexicalWriterProfile,
    ) -> Result<(), String> {
        if handle.profile == profile && handle.writer.is_some() {
            return Ok(());
        }
        handle.writer.take();
        handle.writer = Some(Self::build_writer(&self.index, profile)?);
        handle.profile = profile;
        Ok(())
    }

    fn apply_grouped_batch_locked(
        &self,
        handle: &mut WriterHandle,
        removed_doc_ids: &[String],
        replaced_doc_ids: &[String],
        docs: &[LexicalDocumentRecord],
    ) -> Result<(), String> {
        if removed_doc_ids.is_empty() && replaced_doc_ids.is_empty() && docs.is_empty() {
            return Ok(());
        }

        let writer = handle
            .writer
            .as_mut()
            .ok_or_else(|| "Knowledge tantivy writer is unavailable".to_string())?;

        let total_chunk_ops = docs.iter().map(|doc| doc.chunks.len()).sum::<usize>();
        let mut ops =
            Vec::with_capacity(removed_doc_ids.len() + replaced_doc_ids.len() + total_chunk_ops);
        for doc_id in removed_doc_ids {
            ops.push(UserOperation::Delete(Term::from_field_text(
                self.f_doc_id,
                doc_id,
            )));
        }
        for doc_id in replaced_doc_ids {
            ops.push(UserOperation::Delete(Term::from_field_text(
                self.f_doc_id,
                doc_id,
            )));
        }
        for doc in docs {
            for (section, seq, text) in &doc.chunks {
                ops.push(UserOperation::Add(doc!(
                    self.f_doc_id => doc.doc_id.clone(),
                    self.f_title => doc.title.clone(),
                    self.f_body => text.clone(),
                    self.f_path => doc.path.clone(),
                    self.f_keywords => doc.keywords.clone(),
                    self.f_seq => *seq as u64,
                    self.f_section => section.clone(),
                )));
            }
        }

        writer
            .run(ops.into_iter())
            .map_err(|e| format!("Failed to run grouped knowledge tantivy batch: {}", e))?;
        writer
            .commit()
            .map_err(|e| format!("Failed to commit knowledge tantivy batch: {}", e))?;
        Ok(())
    }

    pub fn open_or_recover(library_dir: &Path) -> Result<Self, String> {
        match Self::open(library_dir) {
            Ok(index) => Ok(index),
            Err(initial_err) => {
                if is_lock_busy_error(&initial_err) {
                    return Err(initial_err);
                }
                let index_dir = library_dir.join("knowledge_tantivy_index");
                if !index_dir.exists() {
                    return Err(initial_err);
                }
                let backup_dir = quarantine_index_dir(&index_dir).map_err(|quarantine_err| {
                    format!(
                        "{}; failed to quarantine knowledge tantivy dir: {}",
                        initial_err, quarantine_err
                    )
                })?;
                eprintln!(
                    "[Locus] quarantined knowledge tantivy index: {} -> {}",
                    index_dir.display(),
                    backup_dir.display()
                );
                Self::open(library_dir).map_err(|reopen_err| {
                    format!(
                        "{}; recreated knowledge tantivy index but reopen still failed: {}",
                        initial_err, reopen_err
                    )
                })
            }
        }
    }

    pub fn index_doc(
        &self,
        doc_id: &str,
        title: &str,
        path: &str,
        keywords: &str,
        chunks: &[(String, i32, String)],
    ) -> Result<(), String> {
        self.index_docs(&[LexicalDocumentRecord {
            doc_id: doc_id.to_string(),
            title: title.to_string(),
            path: path.to_string(),
            keywords: keywords.to_string(),
            chunks: chunks.to_vec(),
        }])
    }

    pub fn apply_batch(
        &self,
        removed_doc_ids: &[String],
        docs: &[LexicalDocumentRecord],
    ) -> Result<(), String> {
        let replaced_doc_ids = docs
            .iter()
            .map(|doc| doc.doc_id.clone())
            .collect::<Vec<_>>();
        let mut handle = self.writer.lock().unwrap();
        self.ensure_writer_profile(&mut handle, LexicalWriterProfile::Default)?;
        self.apply_grouped_batch_locked(&mut handle, removed_doc_ids, &replaced_doc_ids, docs)
    }

    pub fn unity_import_bulk_writer(&self) -> Result<LexicalBulkWriterGuard<'_>, String> {
        let mut guard = self.writer.lock().unwrap();
        self.ensure_writer_profile(&mut guard, LexicalWriterProfile::UnityImportBulk)?;
        Ok(LexicalBulkWriterGuard { index: self, guard })
    }

    pub fn index_docs(&self, docs: &[LexicalDocumentRecord]) -> Result<(), String> {
        self.apply_batch(&[], docs)
            .map_err(|e| e.replace("batch", "index"))
    }

    pub fn remove_doc(&self, doc_id: &str) -> Result<(), String> {
        self.remove_docs(&[doc_id.to_string()])
    }

    pub fn remove_docs(&self, doc_ids: &[String]) -> Result<(), String> {
        self.apply_batch(doc_ids, &[])
            .map_err(|e| e.replace("batch", "removal"))
    }

    pub fn search(
        &self,
        query_text: &str,
        limit: usize,
        boost_title: f32,
    ) -> Result<Vec<LexicalHit>, String> {
        self.reader.reload().map_err(|e| e.to_string())?;
        let searcher = self.reader.searcher();
        let mut query_parser = QueryParser::for_index(
            &self.index,
            vec![self.f_title, self.f_body, self.f_path, self.f_keywords],
        );
        query_parser.set_field_boost(self.f_title, boost_title);
        let query = query_parser
            .parse_query(query_text)
            .map_err(|e| format!("Failed to parse knowledge tantivy query: {}", e))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit).order_by_score())
            .map_err(|e| format!("Knowledge tantivy search failed: {}", e))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let retrieved: tantivy::TantivyDocument = searcher
                .doc(address)
                .map_err(|e| format!("Failed to retrieve knowledge tantivy doc: {}", e))?;
            let body = get_text(&retrieved, self.f_body);
            hits.push(LexicalHit {
                doc_id: get_text(&retrieved, self.f_doc_id),
                section: get_text(&retrieved, self.f_section),
                title: get_text(&retrieved, self.f_title),
                path: get_text(&retrieved, self.f_path),
                score,
                snippet: truncate_snippet(&body, 220),
                matched_terms: Vec::new(),
            });
        }

        Ok(hits)
    }

    pub fn indexed_entry_count(&self) -> Result<usize, String> {
        self.reader.reload().map_err(|e| e.to_string())?;
        Ok(self.reader.searcher().num_docs() as usize)
    }

    pub fn index_dir(&self) -> &Path {
        &self.index_dir
    }
}

fn get_text(doc: &tantivy::TantivyDocument, field: Field) -> String {
    doc.get_first(field)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut end = max_chars;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

fn is_lock_busy_error(error: &str) -> bool {
    error.contains("Failed to acquire Lockfile")
        || error.contains("LockBusy")
        || error.contains("index.lock")
        || error.contains("(os error 32)")
        || error.contains("used by another process")
        || error.contains("另一个程序正在使用此文件")
}

fn quarantine_index_dir(index_dir: &Path) -> Result<PathBuf, String> {
    let parent = index_dir.parent().ok_or_else(|| {
        format!(
            "Knowledge tantivy index has no parent directory: {}",
            index_dir.display()
        )
    })?;
    let dir_name = index_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "Invalid knowledge tantivy index dir name: {}",
                index_dir.display()
            )
        })?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let backup_dir = parent.join(format!(
        "{}.corrupt-{}-{}",
        dir_name,
        stamp,
        std::process::id()
    ));
    std::fs::rename(index_dir, &backup_dir).map_err(|e| {
        format!(
            "Failed to move {} to {}: {}",
            index_dir.display(),
            backup_dir.display(),
            e
        )
    })?;
    Ok(backup_dir)
}

#[cfg(test)]
mod tests {
    use super::KnowledgeTantivyIndex;
    use tempfile::tempdir;

    #[test]
    fn open_or_recover_returns_lock_busy_without_quarantining_index_dir() {
        let dir = tempdir().expect("create temp library dir");
        let _first = KnowledgeTantivyIndex::open_or_recover(dir.path()).expect("open first index");

        let err = match KnowledgeTantivyIndex::open_or_recover(dir.path()) {
            Ok(_) => panic!("second open should fail with a lock error"),
            Err(err) => err,
        };

        assert!(err.contains("Failed to create knowledge tantivy writer"));
        assert!(
            err.contains("Failed to acquire Lockfile")
                || err.contains("LockBusy")
                || err.contains("index.lock")
        );

        let index_dir = dir.path().join("knowledge_tantivy_index");
        assert!(index_dir.is_dir());

        let quarantined_dirs = std::fs::read_dir(dir.path())
            .expect("read library dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.starts_with("knowledge_tantivy_index.corrupt-"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(quarantined_dirs, 0);
    }

    #[test]
    fn is_lock_busy_error_matches_windows_file_in_use_message() {
        assert!(super::is_lock_busy_error(
            "Failed to open/create knowledge tantivy index: An IO error occurred: '另一个程序正在使用此文件，进程无法访问。 (os error 32)'"
        ));
    }
}
