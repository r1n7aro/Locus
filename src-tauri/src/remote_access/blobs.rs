//! Binary previews for paired devices.
//!
//! Commands hand images and media to the desktop UI through the binary cache
//! (`http://locus-binary.localhost/blob/{uuid}`), so the bytes never travel in
//! a JSON reply. Devices fetch the same blobs over HTTPS
//! (`/remote/v1/blob/{uuid}`, device token required); the app maps the URL
//! scheme onto that route.

/// Blob ids are UUIDs (`BinaryCache::insert`); anything else is not looked up.
pub fn is_blob_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Content type a blob is served with. Devices only embed media; any other
/// type is sent as an opaque download so it can never render as a page.
pub fn served_mime(mime: &str) -> &str {
    let essence = mime.split(';').next().unwrap_or("").trim();
    let media = ["image/", "audio/", "video/"].iter().any(|prefix| essence.starts_with(prefix));
    // SVG can carry script: inert in <img>, and the sandbox CSP every blob
    // response carries keeps it inert when opened directly.
    if media || essence == "application/pdf" {
        mime
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_ids_are_uuids_only() {
        assert!(is_blob_id("0f8fad5b-d9cb-469f-a165-70867728950e"));
        assert!(!is_blob_id("0f8fad5b-d9cb-469f-a165-70867728950"));
        assert!(!is_blob_id("../../../../etc/passwd-aaaaaaaaaaaaaaaa"));
        assert!(!is_blob_id("0f8fad5b_d9cb_469f_a165_70867728950e"));
        assert!(!is_blob_id(""));
    }

    #[test]
    fn only_media_keeps_its_type() {
        assert_eq!(served_mime("image/png"), "image/png");
        assert_eq!(served_mime("video/mp4"), "video/mp4");
        assert_eq!(served_mime("application/pdf"), "application/pdf");
        assert_eq!(served_mime("text/html"), "application/octet-stream");
        assert_eq!(served_mime("application/javascript"), "application/octet-stream");
    }
}
