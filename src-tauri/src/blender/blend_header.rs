//! `.blend` file header: the Blender version that saved a file.
//!
//! * 4.x and earlier: 12 bytes `BLENDER` + pointer size (`_` 4, `-` 8) +
//!   endianness (`v` little, `V` big) + 3 digits (`402` = 4.2).
//! * 5.0 and later: 17 bytes `BLENDER17-01v0500` — header size, format
//!   version, endianness, 4 digits (`0500` = 5.0).
//! * Files can be gzip (`1F 8B`, before 3.0) or zstd (`28 B5 2F FD`, 3.0+)
//!   compressed; only the first bytes are decompressed.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendCompression {
    None,
    Gzip,
    Zstd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendHeader {
    pub major: u32,
    pub minor: u32,
    /// `4.2`, `5.0`.
    pub version: String,
    pub pointer_size: u8,
    pub little_endian: bool,
    pub compression: BlendCompression,
    /// 12 (classic) or 17 (5.0 format).
    pub header_size: u8,
}

impl BlendHeader {
    pub fn version_tuple(&self) -> (u32, u32) {
        (self.major, self.minor)
    }
}

const MAGIC: &[u8] = b"BLENDER";

/// Parse the first bytes of an uncompressed `.blend`.
pub fn parse_header(bytes: &[u8], compression: BlendCompression) -> Result<BlendHeader, String> {
    if bytes.len() < 12 || &bytes[..7] != MAGIC {
        return Err("not a .blend file".to_string());
    }
    let digit = |byte: u8| -> Result<u32, String> {
        (byte as char)
            .to_digit(10)
            .ok_or_else(|| "invalid version digits in .blend header".to_string())
    };
    if bytes[7].is_ascii_digit() {
        // BLENDER17-01v0500
        if bytes.len() < 17 {
            return Err("truncated .blend header".to_string());
        }
        let header_size = digit(bytes[7])? * 10 + digit(bytes[8])?;
        if header_size != 17 || bytes[9] != b'-' {
            return Err(format!("unsupported .blend header format (size {header_size})"));
        }
        let little_endian = match bytes[12] {
            b'v' => true,
            b'V' => false,
            _ => return Err("invalid endianness in .blend header".to_string()),
        };
        let major = digit(bytes[13])? * 10 + digit(bytes[14])?;
        let minor = digit(bytes[15])? * 10 + digit(bytes[16])?;
        return Ok(BlendHeader {
            major,
            minor,
            version: format!("{major}.{minor}"),
            pointer_size: 8,
            little_endian,
            compression,
            header_size: 17,
        });
    }
    let pointer_size = match bytes[7] {
        b'_' => 4,
        b'-' => 8,
        _ => return Err("invalid pointer size in .blend header".to_string()),
    };
    let little_endian = match bytes[8] {
        b'v' => true,
        b'V' => false,
        _ => return Err("invalid endianness in .blend header".to_string()),
    };
    let major = digit(bytes[9])?;
    let minor = digit(bytes[10])? * 10 + digit(bytes[11])?;
    Ok(BlendHeader {
        major,
        minor,
        version: format!("{major}.{minor}"),
        pointer_size,
        little_endian,
        compression,
        header_size: 12,
    })
}

/// Read and parse the header of `path`, decompressing only the start.
pub fn read_header(path: &Path) -> Result<BlendHeader, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut start = [0u8; 17];
    let read = read_up_to(&mut file, &mut start).map_err(|e| format!("{}: {e}", path.display()))?;
    let start = &start[..read];
    if start.starts_with(&[0x1f, 0x8b]) {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut bytes = [0u8; 17];
        let read = read_up_to(&mut decoder, &mut bytes).map_err(|e| format!("gzip: {e}"))?;
        return parse_header(&bytes[..read], BlendCompression::Gzip);
    }
    if start.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut decoder = zstd::stream::read::Decoder::new(file).map_err(|e| format!("zstd: {e}"))?;
        let mut bytes = [0u8; 17];
        let read = read_up_to(&mut decoder, &mut bytes).map_err(|e| format!("zstd: {e}"))?;
        return parse_header(&bytes[..read], BlendCompression::Zstd);
    }
    parse_header(start, BlendCompression::None)
}

fn read_up_to(reader: &mut impl Read, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

type CacheKey = (PathBuf, u64, i128);

fn cache() -> &'static Mutex<HashMap<CacheKey, Result<BlendHeader, String>>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Result<BlendHeader, String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Header of `path`, cached by (path, size, modification time).
pub fn header_cached(path: &Path) -> Result<BlendHeader, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i128)
        .unwrap_or(0);
    let key = (path.to_path_buf(), metadata.len(), modified);
    if let Some(hit) = cache().lock().ok().and_then(|cache| cache.get(&key).cloned()) {
        return hit;
    }
    let result = read_header(path);
    if let Ok(mut cache) = cache().lock() {
        if cache.len() > 20_000 {
            cache.clear();
        }
        cache.insert(key, result.clone());
    }
    result
}

/// Saved-version distribution of `.blend` files in a workspace.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDistribution {
    /// `version` → file count, most common first.
    pub versions: Vec<(String, usize)>,
    pub files: usize,
    pub unreadable: usize,
    /// More files existed than the scan limit allowed.
    pub truncated: bool,
}

impl VersionDistribution {
    pub fn most_common(&self) -> Option<&str> {
        self.versions.first().map(|(version, _)| version.as_str())
    }
}

/// Bounded scan (same skip rules as detection): up to `max_files` headers.
pub fn workspace_distribution(root: &Path, max_files: usize) -> VersionDistribution {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut distribution = VersionDistribution::default();
    for path in super::detect::blend_files(root, max_files + 1) {
        if distribution.files >= max_files {
            distribution.truncated = true;
            break;
        }
        distribution.files += 1;
        match header_cached(&path) {
            Ok(header) => *counts.entry(header.version).or_default() += 1,
            Err(_) => distribution.unreadable += 1,
        }
    }
    let mut versions = counts.into_iter().collect::<Vec<_>>();
    versions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| compare_versions(&b.0, &a.0)));
    distribution.versions = versions;
    distribution
}

/// Compare `major.minor` strings numerically.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    parse_version(a).cmp(&parse_version(b))
}

pub fn parse_version(value: &str) -> (u32, u32, u32) {
    let mut parts = value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Can a Blender of `blender_version` open a file saved by `file_version`?
/// 5.x files need 4.5 or newer; any newer file in an older Blender needs an
/// explicit downgrade opt-in; older files always open.
pub fn open_compatibility(file_version: &str, blender_version: &str) -> OpenCompatibility {
    let file = parse_version(file_version);
    let blender = parse_version(blender_version);
    if (file.0, file.1) <= (blender.0, blender.1) {
        return OpenCompatibility::Ok;
    }
    if file.0 >= 5 && (blender.0, blender.1) < (4, 5) {
        return OpenCompatibility::Refused;
    }
    OpenCompatibility::NeedsDowngrade
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompatibility {
    Ok,
    NeedsDowngrade,
    Refused,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn parses_classic_and_v5_headers() {
        let classic = parse_header(b"BLENDER-v402REND", BlendCompression::None).unwrap();
        assert_eq!((classic.major, classic.minor, classic.pointer_size), (4, 2, 8));
        assert!(classic.little_endian);
        assert_eq!(classic.header_size, 12);
        let old = parse_header(b"BLENDER_V279", BlendCompression::None).unwrap();
        assert_eq!((old.version.as_str(), old.pointer_size, old.little_endian), ("2.79", 4, false));
        let v5 = parse_header(b"BLENDER17-01v0500xxx", BlendCompression::None).unwrap();
        assert_eq!((v5.major, v5.minor, v5.header_size), (5, 0, 17));
        assert_eq!(v5.version, "5.0");
        assert!(parse_header(b"NOTBLEND1234", BlendCompression::None).is_err());
        assert!(parse_header(b"BLENDER18-01v0500", BlendCompression::None).is_err());
    }

    #[test]
    fn reads_gzip_and_zstd_compressed_files_and_caches() {
        let temp = tempfile::tempdir().unwrap();
        let plain = write(temp.path(), "plain.blend", b"BLENDER-v410REND....");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"BLENDER-v279REND....").unwrap();
        let gzip = write(temp.path(), "old.blend", &gz.finish().unwrap());
        let zst = zstd::stream::encode_all(&b"BLENDER17-01v0501REND...."[..], 3).unwrap();
        let zstd_path = write(temp.path(), "new.blend", &zst);
        assert_eq!(read_header(&plain).unwrap().version, "4.10");
        let gz_header = read_header(&gzip).unwrap();
        assert_eq!((gz_header.version.as_str(), gz_header.compression), ("2.79", BlendCompression::Gzip));
        let zstd_header = header_cached(&zstd_path).unwrap();
        assert_eq!((zstd_header.version.as_str(), zstd_header.compression), ("5.1", BlendCompression::Zstd));
        assert_eq!(header_cached(&zstd_path).unwrap(), zstd_header);

        std::fs::create_dir_all(temp.path().join("Library")).unwrap();
        write(temp.path(), "Library/ignored.blend", b"BLENDER-v300REND");
        let distribution = workspace_distribution(temp.path(), 100);
        assert_eq!(distribution.files, 3);
        assert_eq!(distribution.versions.len(), 3);
    }

    #[test]
    fn open_rules_follow_blender_compatibility() {
        assert_eq!(open_compatibility("4.2", "5.0"), OpenCompatibility::Ok);
        assert_eq!(open_compatibility("4.2", "4.2"), OpenCompatibility::Ok);
        assert_eq!(open_compatibility("5.0", "4.2"), OpenCompatibility::Refused);
        assert_eq!(open_compatibility("5.0", "4.5"), OpenCompatibility::NeedsDowngrade);
        assert_eq!(open_compatibility("4.4", "4.2"), OpenCompatibility::NeedsDowngrade);
        assert_eq!(compare_versions("4.10", "4.9"), std::cmp::Ordering::Greater);
    }
}
