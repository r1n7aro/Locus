//! State of one running `python` tool call that SDK requests from its
//! process can reach through `LOCUS_PYTHON_CALL`: the call's readonly flag,
//! its live output sink and the images `locus.attach_image` adds.
//!
//! Images reach the tool result through a task-local sink the caller
//! installs around the tool future (the agent loop does); callers without
//! one (background tasks) get the images saved as files and listed in the
//! output.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::session::models::ImageData;

pub const MAX_IMAGES: usize = 4;
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const ENV_CALL: &str = "LOCUS_PYTHON_CALL";
pub const ENV_READONLY: &str = "LOCUS_PYTHON_READONLY";

pub type ImageSink = Arc<Mutex<Vec<ImageData>>>;

tokio::task_local! {
    static IMAGE_SINK: ImageSink;
}

/// Run `future` with an image sink; returns its output and the images.
pub async fn with_image_sink<F: std::future::Future>(future: F) -> (F::Output, Vec<ImageData>) {
    let sink: ImageSink = Arc::new(Mutex::new(Vec::new()));
    let output = IMAGE_SINK.scope(Arc::clone(&sink), future).await;
    let images = sink.lock().map(|mut images| std::mem::take(&mut *images)).unwrap_or_default();
    (output, images)
}

pub fn current_image_sink() -> Option<ImageSink> {
    IMAGE_SINK.try_with(Arc::clone).ok()
}

pub struct PythonCall {
    pub token: String,
    pub readonly: bool,
    pub session_id: Option<String>,
    pub output: Option<crate::async_tasks::TaskOutputReporter>,
    images: Mutex<Vec<ImageData>>,
}

fn calls() -> &'static Mutex<HashMap<String, Arc<PythonCall>>> {
    static CALLS: OnceLock<Mutex<HashMap<String, Arc<PythonCall>>>> = OnceLock::new();
    CALLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registered for the lifetime of the guard.
pub struct PythonCallGuard {
    call: Arc<PythonCall>,
}

impl PythonCallGuard {
    pub fn token(&self) -> &str {
        &self.call.token
    }

    pub fn take_images(&self) -> Vec<ImageData> {
        self.call.images.lock().map(|mut images| std::mem::take(&mut *images)).unwrap_or_default()
    }
}

impl Drop for PythonCallGuard {
    fn drop(&mut self) {
        if let Ok(mut calls) = calls().lock() {
            calls.remove(&self.call.token);
        }
    }
}

pub fn register(
    readonly: bool,
    session_id: Option<String>,
    output: Option<crate::async_tasks::TaskOutputReporter>,
) -> PythonCallGuard {
    let call = Arc::new(PythonCall {
        token: uuid::Uuid::new_v4().simple().to_string(),
        readonly,
        session_id,
        output,
        images: Mutex::new(Vec::new()),
    });
    if let Ok(mut calls) = calls().lock() {
        calls.insert(call.token.clone(), Arc::clone(&call));
    }
    PythonCallGuard { call }
}

pub fn get(token: &str) -> Option<Arc<PythonCall>> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    calls().lock().ok()?.get(token).cloned()
}

pub fn sniff_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

impl PythonCall {
    /// `locus.attach_image`: PNG, JPEG or WebP, at most 20 MiB, 4 per call.
    pub fn attach(&self, data: Vec<u8>) -> Result<usize, String> {
        use base64::Engine as _;
        let mime = sniff_mime(&data).ok_or("only PNG, JPEG and WebP images can be attached")?;
        if data.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "image is {} MiB; the limit is 20 MiB (lower max_long_edge)",
                data.len() / (1024 * 1024)
            ));
        }
        let mut images = self.images.lock().map_err(|e| e.to_string())?;
        if images.len() >= MAX_IMAGES {
            return Err(format!("at most {MAX_IMAGES} images can be attached per call"));
        }
        images.push(ImageData {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
            capture: None,
        });
        Ok(images.len())
    }
}

/// No sink (background task): save the images and name the files.
pub fn save_images(images: &[ImageData], dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    use base64::Engine as _;
    let _ = std::fs::create_dir_all(dir);
    images
        .iter()
        .enumerate()
        .filter_map(|(index, image)| {
            let extension = image.mime_type.rsplit('/').next().unwrap_or("png").replace("jpeg", "jpg");
            let path = dir.join(format!("image-{}.{extension}", index + 1));
            let bytes = base64::engine::general_purpose::STANDARD.decode(&image.data).ok()?;
            std::fs::write(&path, bytes).ok()?;
            Some(path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    #[tokio::test]
    async fn sink_collects_images_attached_inside_the_scope() {
        let (count, images) = with_image_sink(async {
            let guard = register(false, None, None);
            let call = get(guard.token()).expect("registered");
            call.attach(PNG.to_vec()).unwrap();
            let images = guard.take_images();
            current_image_sink().unwrap().lock().unwrap().extend(images);
            1
        })
        .await;
        assert_eq!(count, 1);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].mime_type, "image/png");
        assert!(current_image_sink().is_none());
    }

    #[test]
    fn attach_rejects_unknown_formats_and_caps_the_count() {
        let guard = register(false, None, None);
        let call = get(guard.token()).unwrap();
        assert!(call.attach(b"GIF89a".to_vec()).is_err());
        for _ in 0..MAX_IMAGES {
            call.attach(PNG.to_vec()).unwrap();
        }
        assert!(call.attach(PNG.to_vec()).is_err());
        let token = guard.token().to_string();
        drop(guard);
        assert!(get(&token).is_none());
    }
}
