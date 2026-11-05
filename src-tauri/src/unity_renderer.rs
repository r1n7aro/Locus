//! Window/workspace-scoped GPU outputs. Scene and render-pipeline ownership
//! stays in the calling View's C# code.
use crate::{
    error::AppError,
    workspace_service::{ProjectRegistry, WorkspaceRef},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{Manager, State, Window};

#[cfg(windows)]
use locus_gpu_transport::surface::{Layout, Stats, Surface};
#[cfg(not(windows))]
type Layout = serde_json::Value;
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputRef {
    pub id: String,
    pub owner: String,
    pub domain_generation: String,
}
struct Entry {
    output: OutputRef,
    scope: String,
    // Shared Workbench documents run in their opener's JS/IPC context.
    // Keep the caller for authorization and the display window for the HWND.
    caller_window: String,
    window: String,
    reference: WorkspaceRef,
    touched: Instant,
    #[cfg(windows)]
    surface: Option<Surface>,
}
impl Entry {
    fn uses_window(&self, label: &str) -> bool {
        self.window == label || self.caller_window == label
    }
}
fn entries() -> &'static Mutex<HashMap<String, Entry>> {
    static ENTRIES: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(Default::default)
}
fn failure(message: impl ToString) -> AppError {
    AppError::new("unity.renderer", message.to_string())
}
fn scope_key(reference: &WorkspaceRef) -> String {
    crate::view::view_workspace_scope_key(reference)
}
fn project(registry: &ProjectRegistry, reference: &WorkspaceRef) -> Result<String, AppError> {
    let scope = registry.resolve_workspace_ref(reference).map_err(failure)?;
    Ok(scope.runtime().root().to_string_lossy().into_owned())
}
fn resolve_surface_window(window: &Window, label: Option<&str>) -> Result<String, AppError> {
    let label = label.unwrap_or(window.label());
    window
        .app_handle()
        .get_webview_window(label)
        .ok_or_else(|| failure("GPU surface window is unavailable"))?;
    if label != window.label() {
        // Only an opener can host a surface in its shared Workbench child.
        // Never accept a raw HWND or silently fall back to the caller window.
        // Floating windows have no native owner; the opener table decides.
        let opener = crate::shared_workbench_window::owner_of(label);
        if !label.starts_with("workbench-") || opener.as_deref() != Some(window.label()) {
            return Err(failure("GPU surface window does not belong to this caller"));
        }
        #[cfg(not(windows))]
        return Err(failure("Native GPU surface requires Windows"));
    }
    Ok(label.to_string())
}
fn authorize(
    entry: &Entry,
    output: &OutputRef,
    reference: &WorkspaceRef,
    label: &str,
) -> Result<(), AppError> {
    if entry.output.owner != output.owner
        || entry.output.domain_generation != output.domain_generation
        || entry.scope != scope_key(reference)
        || entry.caller_window != label
    {
        return Err(failure(
            "GPU output does not belong to this window, workspace or execution owner",
        ));
    }
    Ok(())
}
fn retire(entry: Entry) {
    #[cfg(windows)]
    locus_gpu_transport::surface::close_output(&entry.output.id);
    // Destroy the native window on its own thread without blocking Tauri's UI.
    tauri::async_runtime::spawn_blocking(move || drop(entry));
}
fn start_gc(app: tauri::AppHandle) {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let registry = app.state::<Arc<ProjectRegistry>>();
            let expired = {
                let Ok(mut map) = entries().lock() else {
                    continue;
                };
                let ids = map
                    .iter()
                    .filter(|(_, entry)| {
                        entry.touched.elapsed() > Duration::from_secs(15)
                            || app.get_webview_window(&entry.window).is_none()
                            || app.get_webview_window(&entry.caller_window).is_none()
                            || registry.resolve_workspace_ref(&entry.reference).is_err()
                    })
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                ids.into_iter()
                    .filter_map(|id| map.remove(&id))
                    .collect::<Vec<_>>()
            };
            for entry in expired {
                retire(entry);
            }
        }
    });
}
pub fn release_window(label: &str) {
    let removed = {
        let Ok(mut map) = entries().lock() else {
            return;
        };
        let ids = map
            .iter()
            .filter(|(_, e)| e.uses_window(label))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| map.remove(&id))
            .collect::<Vec<_>>()
    };
    for entry in removed {
        retire(entry);
    }
}

#[tauri::command]
pub async fn unity_renderer_capabilities(
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<serde_json::Value, AppError> {
    let root = project(&registry, &workspace_ref)?;
    if !cfg!(windows) {
        return Ok(
            serde_json::json!({"supported":false,"reason":"GPU output currently requires Windows and D3D11"}),
        );
    }
    match crate::unity_bridge::render_output_control(&root, "capabilities", "").await {
        Ok(value) => Ok(
            serde_json::json!({"supported":true,"transport":"d3d11-shared-texture","protocolVersion":1,"formats":["rgba8","bgra8"],"maxDimension":4096,"maxOutputs":16,"unity":value}),
        ),
        Err(reason) => Ok(serde_json::json!({"supported":false,"reason":reason})),
    }
}
#[tauri::command]
pub async fn unity_renderer_create(
    window: Window,
    window_label: Option<String>,
    workspace_ref: WorkspaceRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<OutputRef, AppError> {
    if !cfg!(windows) {
        return Err(failure("GPU output currently requires Windows and D3D11"));
    }
    let root = project(&registry, &workspace_ref)?;
    let surface_window = resolve_surface_window(&window, window_label.as_deref())?;
    {
        let map = entries().lock().map_err(failure)?;
        if map.values().filter(|e| e.window == surface_window).count() >= 16 {
            return Err(failure("This window already has 16 GPU outputs"));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let result = crate::unity_bridge::render_output_control(&root, "create", &id).await;
    // A lost reply must not leave an unattached producer alive indefinitely.
    let result = match result {
        Ok(value) => value,
        Err(error) => {
            #[cfg(windows)]
            locus_gpu_transport::surface::close_output(&id);
            let _ = crate::unity_bridge::render_output_control(&root, "close", &id).await;
            return Err(failure(error));
        }
    };
    if project(&registry, &workspace_ref).is_err()
        || resolve_surface_window(&window, Some(&surface_window)).is_err()
        || window
            .app_handle()
            .get_webview_window(window.label())
            .is_none()
    {
        #[cfg(windows)]
        locus_gpu_transport::surface::close_output(&id);
        return Err(failure(
            "Workspace or window closed while creating the output",
        ));
    }
    let domain = match result
        .get("domainGeneration")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
    {
        Some(value) if result.get("version").and_then(|v| v.as_u64()) == Some(1) => {
            value.to_string()
        }
        _ => {
            #[cfg(windows)]
            locus_gpu_transport::surface::close_output(&id);
            return Err(failure("Incompatible GPU output handshake"));
        }
    };
    let output = OutputRef {
        id,
        owner: uuid::Uuid::new_v4().to_string(),
        domain_generation: domain,
    };
    {
        let mut map = entries().lock().map_err(failure)?;
        if map.values().filter(|e| e.window == surface_window).count() >= 16 {
            #[cfg(windows)]
            locus_gpu_transport::surface::close_output(&output.id);
            return Err(failure("This window already has 16 GPU outputs"));
        }
        map.insert(
            output.id.clone(),
            Entry {
                output: output.clone(),
                scope: scope_key(&workspace_ref),
                caller_window: window.label().into(),
                window: surface_window,
                reference: workspace_ref,
                touched: Instant::now(),
                #[cfg(windows)]
                surface: None,
            },
        );
    }
    start_gc(window.app_handle().clone());
    Ok(output)
}
#[tauri::command]
pub fn unity_renderer_mount(
    window: Window,
    workspace_ref: WorkspaceRef,
    output: OutputRef,
    layout: Layout,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), AppError> {
    project(&registry, &workspace_ref)?;
    let mut map = entries().lock().map_err(failure)?;
    let entry = map
        .get_mut(&output.id)
        .ok_or_else(|| failure("GPU output is closed"))?;
    authorize(entry, &output, &workspace_ref, window.label())?;
    #[cfg(windows)]
    {
        if entry.surface.is_some() {
            return Err(failure("GPU output is already mounted"));
        }
        let target = window
            .app_handle()
            .get_webview_window(&entry.window)
            .ok_or_else(|| failure("GPU surface window is unavailable"))?;
        let parent = target.hwnd().map_err(failure)?.0 as usize;
        entry.surface = Some(Surface::start(output.id, parent, layout).map_err(failure)?);
        entry.touched = Instant::now();
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let _ = layout;
        Err(failure("Native GPU surface requires Windows"))
    }
}
#[tauri::command]
pub fn unity_renderer_layout(
    window: Window,
    workspace_ref: WorkspaceRef,
    output: OutputRef,
    layout: Layout,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), AppError> {
    project(&registry, &workspace_ref)?;
    let mut map = entries().lock().map_err(failure)?;
    let entry = map
        .get_mut(&output.id)
        .ok_or_else(|| failure("GPU output is closed"))?;
    authorize(entry, &output, &workspace_ref, window.label())?;
    entry.touched = Instant::now();
    #[cfg(windows)]
    {
        entry
            .surface
            .as_ref()
            .ok_or_else(|| failure("GPU output is not mounted"))?
            .update(layout)
            .map_err(failure)
    }
    #[cfg(not(windows))]
    {
        let _ = layout;
        Err(failure("Native GPU surface requires Windows"))
    }
}
#[tauri::command]
pub fn unity_renderer_status(
    window: Window,
    workspace_ref: WorkspaceRef,
    output: OutputRef,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<serde_json::Value, AppError> {
    project(&registry, &workspace_ref)?;
    let mut map = entries().lock().map_err(failure)?;
    let entry = map
        .get_mut(&output.id)
        .ok_or_else(|| failure("GPU output is closed"))?;
    authorize(entry, &output, &workspace_ref, window.label())?;
    entry.touched = Instant::now();
    #[cfg(windows)]
    {
        locus_gpu_transport::surface::touch_output(&output.id);
        serde_json::to_value(
            entry
                .surface
                .as_ref()
                .map(|s| s.status())
                .unwrap_or_else(|| Stats {
                    state: "unmounted".into(),
                    ..Default::default()
                }),
        )
        .map_err(failure)
    }
    #[cfg(not(windows))]
    {
        Ok(serde_json::json!({"state":"unsupported"}))
    }
}
#[tauri::command]
pub async fn unity_renderer_detach(
    window: Window,
    workspace_ref: WorkspaceRef,
    output: OutputRef,
) -> Result<(), AppError> {
    // Cleanup must remain possible after workspace generation changed.
    #[cfg(windows)]
    let surface = {
        let mut map = entries().lock().map_err(failure)?;
        let Some(entry) = map.get_mut(&output.id) else {
            return Ok(());
        };
        authorize(entry, &output, &workspace_ref, window.label())?;
        entry.surface.take()
    };
    #[cfg(windows)]
    if let Some(surface) = surface {
        tauri::async_runtime::spawn_blocking(move || surface.stop_and_join())
            .await
            .map_err(failure)?;
    }
    Ok(())
}
#[tauri::command]
pub fn unity_renderer_close(
    window: Window,
    workspace_ref: WorkspaceRef,
    output: OutputRef,
) -> Result<(), AppError> {
    let removed = {
        let mut map = entries().lock().map_err(failure)?;
        if let Some(entry) = map.get(&output.id) {
            authorize(entry, &output, &workspace_ref, window.label())?;
        }
        map.remove(&output.id)
    };
    if let Some(entry) = removed {
        retire(entry);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(display_window: &str) -> Entry {
        let reference: WorkspaceRef = serde_json::from_value(serde_json::json!({
            "checkoutId": "checkout-a", "expectedGeneration": 2,
            "expectedMaterializationEpoch": 3
        }))
        .unwrap();
        Entry {
            output: OutputRef {
                id: "output".into(),
                owner: "owner".into(),
                domain_generation: "domain".into(),
            },
            scope: scope_key(&reference),
            caller_window: "main".into(),
            window: display_window.into(),
            reference,
            touched: Instant::now(),
            #[cfg(windows)]
            surface: None,
        }
    }

    #[test]
    fn shared_surface_keeps_its_opener_as_ipc_owner() {
        let entry = entry("workbench-shared-a");
        assert!(authorize(&entry, &entry.output, &entry.reference, "main").is_ok());
        for caller in [
            "workbench-shared-a",
            "workbench-shared-b",
            "workspace-other",
        ] {
            assert!(authorize(&entry, &entry.output, &entry.reference, caller).is_err());
        }
    }

    #[test]
    fn shared_outputs_still_reject_other_executions_and_workspace_generations() {
        let entry = entry("workbench-shared-a");
        let mut output = entry.output.clone();
        output.owner = "other".into();
        assert!(authorize(&entry, &output, &entry.reference, "main").is_err());
        output = entry.output.clone();
        output.domain_generation = "old-domain".into();
        assert!(authorize(&entry, &output, &entry.reference, "main").is_err());
        let mut reference = entry.reference.clone();
        reference.expected_generation = Some(1);
        assert!(authorize(&entry, &entry.output, &reference, "main").is_err());
        reference = entry.reference.clone();
        reference.expected_materialization_epoch = Some(2);
        assert!(authorize(&entry, &entry.output, &reference, "main").is_err());
    }

    #[test]
    fn closing_a_display_window_releases_only_its_outputs() {
        let entries = [
            entry("main"),
            entry("workbench-shared-a"),
            entry("workbench-shared-b"),
        ];
        let released = entries
            .iter()
            .filter(|entry| entry.uses_window("workbench-shared-a"))
            .map(|entry| entry.window.as_str())
            .collect::<Vec<_>>();
        assert_eq!(released, ["workbench-shared-a"]);
        assert!(entries.iter().all(|entry| entry.uses_window("main")));
        assert!(!entries
            .iter()
            .any(|entry| entry.uses_window("workspace-other")));
    }
}
