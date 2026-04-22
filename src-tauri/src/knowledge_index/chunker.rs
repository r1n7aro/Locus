use super::db::ChunkRecord;

const MAX_CHUNK_CHARS: usize = 1000;
const MIN_CHUNK_CHARS: usize = 50;

pub fn chunk_document(
    title: &str,
    summary: Option<&str>,
    rules: Option<&str>,
    body: &str,
) -> Vec<ChunkRecord> {
    let mut chunks = Vec::new();
    if let Some(summary) = summary.filter(|value| !value.trim().is_empty()) {
        chunks.extend(chunk_section(
            &format!("# {}\n\n{}", title.trim(), summary.trim()),
            "summary",
        ));
    }
    if let Some(rules) = rules.filter(|value| !value.trim().is_empty()) {
        chunks.extend(chunk_section(
            &format!(
                "# {}\n\n## Maintenance Rules\n{}",
                title.trim(),
                rules.trim()
            ),
            "maintenanceRules",
        ));
    }
    chunks.extend(chunk_section(
        &format!("# {}\n\n{}", title.trim(), body.trim()),
        "body",
    ));
    chunks
}

fn chunk_section(text: &str, section: &str) -> Vec<ChunkRecord> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    if section != "body" || text.len() <= MAX_CHUNK_CHARS {
        return vec![make_chunk(section, 0, text)];
    }

    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut seq = 0;

    for paragraph in paragraphs {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }

        if current.len() + paragraph.len() + 2 > MAX_CHUNK_CHARS && !current.is_empty() {
            chunks.push(make_chunk(section, seq, &current));
            seq += 1;
            current.clear();
        }

        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(paragraph);
    }

    if !current.is_empty() {
        if current.len() < MIN_CHUNK_CHARS && !chunks.is_empty() {
            let last = chunks.last_mut().expect("last chunk");
            let merged = format!("{}\n\n{}", last.text, current);
            *last = make_chunk(section, last.seq, &merged);
        } else {
            chunks.push(make_chunk(section, seq, &current));
        }
    }

    if chunks.is_empty() {
        chunks.push(make_chunk(section, 0, text));
    }

    chunks
}

fn make_chunk(section: &str, seq: i32, text: &str) -> ChunkRecord {
    ChunkRecord {
        section: section.to_string(),
        seq,
        text: text.to_string(),
        text_hash: blake3::hash(text.as_bytes()).as_bytes().to_vec(),
    }
}
