pub mod data;
#[cfg(windows)]
mod native;
use crate::session::{
    models::{CaptureReference, ImageData},
    store::SessionStore,
};
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};
use base64::Engine;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Manager;

#[derive(Default)]
pub struct CaptureState {
    busy: AtomicBool,
}
pub fn init(app: &tauri::AppHandle) {
    app.manage(CaptureState::default());
}
struct CaptureGuard<'a>(&'a AtomicBool);
impl Drop for CaptureGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotAttachment {
    image: ImageData,
    clipboard_error: Option<String>,
    spatial_error: Option<String>,
}

/// Capture only the originating composer's checkout. The caller keeps its draft
/// identity while awaiting this response; no global focus registration is needed.
#[tauri::command]
pub async fn capture_unity_attachment(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    workspace_ref: WorkspaceRef,
    target: String,
) -> Result<ScreenshotAttachment, String> {
    if !matches!(target.as_str(), "scene" | "game") {
        return Err("Invalid screenshot target".into());
    }
    let runtime = app
        .state::<Arc<ProjectRegistry>>()
        .resolve_workspace_ref(&workspace_ref)
        .map_err(|e| e.to_string())?;
    let root = runtime.runtime().root().to_string_lossy().to_string();
    let state = app.state::<CaptureState>();
    if state.busy.swap(true, Ordering::Acquire) {
        return Err("Another screenshot is in progress".into());
    }
    let _guard = CaptureGuard(&state.busy);
    let started = std::time::Instant::now();
    let id = uuid::Uuid::new_v4().to_string();
    let request = serde_json::json!({"id":id,"target":target}).to_string();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::unity_bridge::send_message(&root, "capture_attachment", &request),
    )
    .await
    .map_err(|_| "Unity screenshot timed out".to_string())??;
    if !response.ok {
        return Err(response
            .error
            .or(response.message)
            .unwrap_or_else(|| "Unity screenshot failed".into()));
    }
    let message = response.message.ok_or("Unity returned no screenshot")?;
    let capture_app = app.clone();
    let label = window.label().to_owned();
    let checkout = workspace_ref.checkout_id.to_string();
    let result = tokio::task::spawn_blocking(move || {
        deliver(&capture_app, &label, &checkout, &id, &message)
    })
    .await
    .map_err(|e| e.to_string())?;
    tracing::info!(
        log_module = "LocusCapture",
        elapsed_ms = started.elapsed().as_millis(),
        "screenshot command finished"
    );
    result
}
/// Matches `MAX_CHAT_MESSAGE_IMAGE_PIXELS` in `src/composables/chatMessageImage.ts`.
const CLIPBOARD_PNG_MAX_PIXELS: usize = 33_554_432;

/// Writes a PNG rendered by the frontend (the chat "copy as image" action) to
/// the system clipboard. The webview tries its async clipboard first; this is
/// the fallback when that is refused, e.g. while Unity owns keyboard focus.
/// The request body carries the raw PNG bytes.
#[tauri::command]
pub async fn write_clipboard_png(
    window: tauri::WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected PNG bytes".into());
    };
    let bytes = bytes.clone();
    tokio::task::spawn_blocking(move || write_png_to_clipboard(&window, &bytes))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(windows)]
fn write_png_to_clipboard(window: &tauri::WebviewWindow, bytes: &[u8]) -> Result<(), String> {
    let (width, height, rgba) = decode_png_rgba(bytes)?;
    let owner = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    native::clipboard(width, height, &rgba, owner)
}

#[cfg(not(windows))]
fn write_png_to_clipboard(_window: &tauri::WebviewWindow, _bytes: &[u8]) -> Result<(), String> {
    Err("Clipboard image write requires Windows".into())
}

/// Decodes any 8/16-bit PNG into tightly packed RGBA8.
#[cfg_attr(not(windows), allow(dead_code))]
fn decode_png_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    let pixels = width as usize * height as usize;
    if pixels == 0 || pixels > CLIPBOARD_PNG_MAX_PIXELS {
        return Err("Image is too large for the clipboard".into());
    }
    let size = reader
        .output_buffer_size()
        .ok_or("Image is too large for the clipboard")?;
    let mut buffer = vec![0; size];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    buffer.truncate(frame.buffer_size());
    let rgba = match frame.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        other => return Err(format!("Unsupported PNG color type {other:?}")),
    };
    if rgba.len() != pixels * 4 {
        return Err("Invalid PNG pixel data".into());
    }
    Ok((frame.width, frame.height, rgba))
}

fn inflate(payload: &str, expected: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| e.to_string())?;
    let mut raw = Vec::with_capacity(expected);
    flate2::read::DeflateDecoder::new(compressed.as_slice())
        .take((expected + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() != expected {
        return Err("Invalid screenshot payload length".into());
    }
    Ok(raw)
}
fn deliver(
    app: &tauri::AppHandle,
    window_label: &str,
    checkout: &str,
    id: &str,
    message: &str,
) -> Result<ScreenshotAttachment, String> {
    let started = std::time::Instant::now();
    let wire: serde_json::Value = serde_json::from_str(message).map_err(|e| e.to_string())?;
    let width = wire["width"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 8192)
        .ok_or("Invalid screenshot width")? as u32;
    let height = wire["height"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 8192)
        .ok_or("Invalid screenshot height")? as u32;
    let count = width as usize * height as usize;
    if count > data::MAX_PIXELS {
        return Err("Screenshot too large".into());
    }
    let rgba = inflate(
        wire["pixels"].as_str().ok_or("Missing screenshot pixels")?,
        count * 4,
    )?;
    let spatial = parse_spatial(&wire)?.into_iter().next();
    let context = wire
        .get("context")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value::<data::CaptureViewContext>(value.clone()))
        .transpose()
        .map_err(|e| e.to_string())?;
    if let Some(context) = &context {
        context.validate()?;
    }
    let spatial_error = if spatial.is_none() {
        Some(
            context
                .as_ref()
                .and_then(|c| c.spatial_error.clone())
                .unwrap_or_else(|| "Object/depth data unavailable".into()),
        )
    } else {
        None
    };
    let decode_ms = started.elapsed().as_millis();
    let window = app
        .get_webview_window(window_label)
        .ok_or("Screenshot input was closed")?;
    #[cfg(windows)]
    let clipboard_error = native::clipboard(
        width,
        height,
        &rgba,
        window.hwnd().map_err(|e| e.to_string())?.0 as isize,
    )
    .err();
    #[cfg(not(windows))]
    let clipboard_error = Some("Clipboard capture requires Windows".to_string());
    let clipboard_ms = started.elapsed().as_millis() - decode_ms;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fastest);
        encoder
            .write_header()
            .map_err(|e| e.to_string())?
            .write_image_data(&rgba)
            .map_err(|e| e.to_string())?;
    }
    let png_ms = started.elapsed().as_millis() - decode_ms - clipboard_ms;
    app.state::<Arc<SessionStore>>().save_capture(
        id,
        checkout,
        &bytes,
        spatial.as_ref(),
        context.as_ref(),
    )?;
    let image = ImageData {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        mime_type: "image/png".into(),
        capture: Some(CaptureReference { id: id.into() }),
    };
    tracing::info!(
        log_module = "LocusCapture",
        decode_ms,
        clipboard_ms,
        png_ms,
        total_ms = started.elapsed().as_millis(),
        unity_ms = wire["captureMs"].as_f64().unwrap_or(0.),
        "attachment delivered"
    );
    Ok(ScreenshotAttachment {
        image,
        clipboard_error,
        spatial_error,
    })
}
fn parse_spatial(batch: &serde_json::Value) -> Result<Vec<data::SpatialSnapshot>, String> {
    let mut result = Vec::new();
    for wire in batch["snapshots"]
        .as_array()
        .ok_or("No spatial snapshots")?
        .iter()
        .take(1)
    {
        let mut snapshot: data::SpatialSnapshot =
            serde_json::from_value(wire.clone()).map_err(|e| e.to_string())?;
        let count = snapshot.width as usize * snapshot.height as usize;
        if count == 0 || count > data::MAX_PIXELS {
            return Err("Spatial snapshot is too large".into());
        }
        let raw = inflate(
            wire["payload"].as_str().ok_or("No spatial payload")?,
            count * 8,
        )?;
        snapshot.load_planes(&raw)?;
        snapshot.validate()?;
        result.push(snapshot);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::decode_png_rgba;

    fn encode(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
        bytes
    }

    #[test]
    fn decodes_rgba_rgb_and_gray_png_to_rgba() {
        let rgba = encode(2, 1, png::ColorType::Rgba, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            decode_png_rgba(&rgba).unwrap(),
            (2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8])
        );
        let rgb = encode(2, 1, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6]);
        assert_eq!(
            decode_png_rgba(&rgb).unwrap(),
            (2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255])
        );
        let gray = encode(1, 2, png::ColorType::Grayscale, &[9, 10]);
        assert_eq!(
            decode_png_rgba(&gray).unwrap(),
            (1, 2, vec![9, 9, 9, 255, 10, 10, 10, 255])
        );
    }

    #[test]
    fn rejects_non_png_bytes() {
        assert!(decode_png_rgba(b"not a png").is_err());
    }
}
