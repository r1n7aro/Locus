use keyring::Entry;

const SERVICE: &str = "locus";
const CHUNK_MARKER_PREFIX: &str = "__locus_chunked_v1__:";
const MAX_SECRET_BYTES_PER_ENTRY: usize = 2000;

fn entry(key: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, key).map_err(|e| format!("Keychain entry error: {}", e))
}

fn set_secret_raw(key: &str, value: &[u8]) -> Result<(), String> {
    entry(key)?
        .set_secret(value)
        .map_err(|e| format!("Keychain set error ({}): {}", key, e))
}

fn get_secret_raw(key: &str) -> Result<Option<Vec<u8>>, String> {
    match entry(key)?.get_secret() {
        Ok(val) => Ok(Some(val)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Keychain get error ({}): {}", key, e)),
    }
}

fn delete_secret_raw(key: &str) -> Result<(), String> {
    match entry(key)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Keychain delete error ({}): {}", key, e)),
    }
}

fn chunk_key(key: &str, index: usize) -> String {
    format!("{key}#chunk#{index}")
}

fn parse_chunk_count(raw: &[u8]) -> Option<usize> {
    let marker = String::from_utf8(raw.to_vec()).ok()?;
    marker.strip_prefix(CHUNK_MARKER_PREFIX)?.parse().ok()
}

fn split_secret_chunks(value: &str, max_bytes: usize) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut current_bytes = 0usize;

    for (index, ch) in value.char_indices() {
        let char_bytes = ch.len_utf8();
        if current_bytes + char_bytes > max_bytes && index > start {
            chunks.push(value[start..index].to_string());
            start = index;
            current_bytes = 0;
        }
        current_bytes += char_bytes;
    }

    if start < value.len() {
        chunks.push(value[start..].to_string());
    }

    chunks
}

fn delete_chunked_secret(key: &str, chunk_count: usize) -> Result<(), String> {
    for index in 0..chunk_count {
        delete_secret_raw(&chunk_key(key, index))?;
    }
    Ok(())
}

/// Store a secret string in the OS keychain.
pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    if let Some(existing) = get_secret_raw(key)? {
        if let Some(chunk_count) = parse_chunk_count(&existing) {
            delete_chunked_secret(key, chunk_count)?;
        }
    }

    if value.as_bytes().len() <= MAX_SECRET_BYTES_PER_ENTRY {
        return set_secret_raw(key, value.as_bytes());
    }

    let chunks = split_secret_chunks(value, MAX_SECRET_BYTES_PER_ENTRY);
    let mut written_chunk_count = 0usize;
    for (index, chunk) in chunks.iter().enumerate() {
        if let Err(error) = set_secret_raw(&chunk_key(key, index), chunk.as_bytes()) {
            let _ = delete_chunked_secret(key, written_chunk_count);
            return Err(error);
        }
        written_chunk_count += 1;
    }

    if let Err(error) = set_secret_raw(
        key,
        format!("{CHUNK_MARKER_PREFIX}{}", chunks.len()).as_bytes(),
    ) {
        let _ = delete_chunked_secret(key, chunks.len());
        return Err(error);
    }

    Ok(())
}

/// Retrieve a secret string from the OS keychain.
/// Returns `Ok(None)` if the entry does not exist.
pub fn get_secret(key: &str) -> Result<Option<String>, String> {
    let Some(raw) = get_secret_raw(key)? else {
        return Ok(None);
    };

    if let Some(chunk_count) = parse_chunk_count(&raw) {
        let mut combined = Vec::new();
        for index in 0..chunk_count {
            let Some(chunk) = get_secret_raw(&chunk_key(key, index))? else {
                return Err(format!(
                    "Keychain get error ({}): missing secret chunk {} of {}",
                    key,
                    index + 1,
                    chunk_count
                ));
            };
            combined.extend_from_slice(&chunk);
        }
        return String::from_utf8(combined).map(Some).map_err(|e| {
            format!(
                "Keychain get error ({}): secret is not valid UTF-8: {}",
                key, e
            )
        });
    }

    String::from_utf8(raw).map(Some).map_err(|e| {
        format!(
            "Keychain get error ({}): secret is not valid UTF-8: {}",
            key, e
        )
    })
}

/// Delete a secret from the OS keychain.
/// Silently succeeds if the entry does not exist.
pub fn delete_secret(key: &str) -> Result<(), String> {
    if let Some(raw) = get_secret_raw(key)? {
        if let Some(chunk_count) = parse_chunk_count(&raw) {
            delete_chunked_secret(key, chunk_count)?;
        }
    }
    delete_secret_raw(key)
}

// ── Key constants ──

pub const KEY_OPENROUTER: &str = "openrouter_api_key";
pub const KEY_CLAUDE_TOKENS: &str = "claude_tokens";
pub const KEY_CODEX_TOKENS: &str = "codex_tokens";
pub const KEY_PLUGIN_GITHUB_TOKEN: &str = "plugin_github_token";

/// Provider key keychain name: "provider/{id}"
pub fn provider_key_name(provider_id: &str) -> String {
    format!("provider/{}", provider_id)
}

/// Custom endpoint key keychain name: "endpoint/{id}"
pub fn endpoint_key_name(endpoint_id: &str) -> String {
    format!("endpoint/{}", endpoint_id)
}
