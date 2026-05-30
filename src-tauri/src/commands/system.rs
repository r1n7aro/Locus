use std::path::PathBuf;

use tauri::AppHandle;
use tauri::State;

#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;

#[cfg(windows)]
const WINDOWS_NOTIFICATION_DISPLAY_NAME: &str = "Locus";

#[tauri::command]
pub fn get_system_locale() -> Option<String> {
    sys_locale::get_locale()
}

#[tauri::command]
pub fn get_proxy_status() -> crate::network::ProxyStatus {
    crate::network::get_proxy_status()
}

#[tauri::command]
pub fn save_proxy_config(
    config: crate::network::ProxyConfig,
) -> Result<crate::network::ProxyStatus, crate::error::AppError> {
    crate::network::save_proxy_config(config).map_err(crate::error::AppError::from)
}

#[tauri::command]
pub async fn get_python_runtime_state(
    app_handle: AppHandle,
    refresh: Option<bool>,
    discover: Option<bool>,
) -> Result<crate::python_runtime::PythonRuntimeState, crate::error::AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::python_runtime::python_runtime_state_with_options(
            Some(&app_handle),
            refresh.unwrap_or(false),
            discover.unwrap_or(true),
        )
    })
    .await
    .map_err(|e| {
        crate::error::AppError::new(
            "python_runtime.join_failed",
            format!("Failed to load Python runtime state: {}", e),
        )
    })?
    .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub async fn save_python_runtime_selection(
    selected_id: String,
    app_handle: AppHandle,
) -> Result<crate::python_runtime::PythonRuntimeState, crate::error::AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::python_runtime::save_python_runtime_selection(&selected_id, Some(&app_handle))
    })
    .await
    .map_err(|e| {
        crate::error::AppError::new(
            "python_runtime.join_failed",
            format!("Failed to save Python runtime selection: {}", e),
        )
    })?
    .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn send_system_notification(
    app_handle: AppHandle,
    title: String,
    body: Option<String>,
) -> Result<(), String> {
    send_system_notification_impl(&app_handle, &title, body.as_deref())
}

#[tauri::command]
pub fn play_custom_notification_sound(path: String, volume: Option<f32>) -> Result<(), String> {
    let path = PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err("Audio file path is empty".into());
    }
    if !path.is_file() {
        return Err(format!("Audio file does not exist: {}", path.display()));
    }

    let file = std::fs::File::open(&path)
        .map_err(|error| format!("Failed to open audio file: {error}"))?;
    let reader = std::io::BufReader::new(file);
    let sink_handle = rodio::DeviceSinkBuilder::open_default_sink()
        .map_err(|error| format!("Failed to open default audio output: {error}"))?;
    let player = rodio::play(sink_handle.mixer(), reader)
        .map_err(|error| format!("Failed to play audio file: {error}"))?;
    let volume = volume
        .filter(|value| value.is_finite())
        .unwrap_or(1.0)
        .clamp(0.0, 2.0);
    player.set_volume(volume);

    std::thread::Builder::new()
        .name("locus-custom-notification-sound".into())
        .spawn(move || {
            player.sleep_until_end();
            drop(sink_handle);
        })
        .map(|_| ())
        .map_err(|error| format!("Failed to start audio playback thread: {error}"))
}

#[tauri::command]
pub fn request_app_exit(app_handle: AppHandle) {
    exit_app(&app_handle);
}

pub(crate) fn exit_app(app_handle: &AppHandle) {
    if let Err(error) = crate::unity_bridge::restore_background_hook_runtime() {
        eprintln!("[Locus] failed to restore Unity background hook before exit: {error}");
    }
    crate::commands::destroy_unity_embed_control_window_on_main(app_handle);
    app_handle.exit(0);
}

#[tauri::command]
pub fn get_close_behavior(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<crate::config::AppCloseBehavior, crate::error::AppError> {
    Ok(config.close_behavior())
}

#[tauri::command]
pub fn set_close_behavior(
    value: crate::config::AppCloseBehavior,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_close_behavior(value)
        .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn get_dynamic_tool_loading_mode(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<crate::config::DynamicToolLoadingMode, crate::error::AppError> {
    Ok(config.dynamic_tool_loading_mode())
}

#[tauri::command]
pub fn set_dynamic_tool_loading_mode(
    value: crate::config::DynamicToolLoadingMode,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_dynamic_tool_loading_mode(value)
        .map_err(crate::error::AppError::from)
}

#[tauri::command]
pub fn get_unity_background_hook_enabled(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.unity_background_hook_enabled())
}

#[tauri::command]
pub async fn set_unity_background_hook_enabled(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
    workspace: State<'_, std::sync::Arc<crate::workspace::Workspace>>,
) -> Result<crate::unity_bridge::UnityBackgroundHookStatus, crate::error::AppError> {
    config
        .set_unity_background_hook_enabled(value)
        .map_err(crate::error::AppError::from)?;

    let status = crate::unity_bridge::set_background_hook_enabled(value).map_err(|error| {
        crate::error::AppError::new("unity.background_hook.restore_failed", error)
            .operation("setUnityBackgroundHookEnabled")
    })?;

    if !value {
        return Ok(status);
    }

    let cwd = workspace.path.read().await.clone();
    if cwd.trim().is_empty() || !crate::unity_bridge::is_unity_project(&cwd) {
        return Ok(status);
    }

    match crate::unity_bridge::ensure_background_hook_for_project(&cwd).await {
        Ok(status) => {
            if status.enabled
                && status.state == crate::unity_bridge::UnityBackgroundHookState::Failed
            {
                let message = status
                    .error
                    .clone()
                    .unwrap_or_else(|| "Unity background hook failed".to_string());
                return Err(
                    crate::error::AppError::new("unity.background_hook.failed", message)
                        .operation("setUnityBackgroundHookEnabled"),
                );
            }
            Ok(status)
        }
        Err(error) => {
            let process_info =
                crate::unity_bridge::query_current_project_editor_process(&cwd).await;
            if matches!(
                process_info.state,
                crate::unity_bridge::UnityEditorProcessState::NotRunning
            ) {
                return Ok(status);
            }
            Err(
                crate::error::AppError::new("unity.background_hook.failed", error)
                    .operation("setUnityBackgroundHookEnabled"),
            )
        }
    }
}

#[tauri::command]
pub fn get_unity_background_hook_status(
) -> Result<crate::unity_bridge::UnityBackgroundHookStatus, crate::error::AppError> {
    Ok(crate::unity_bridge::background_hook_status())
}

#[tauri::command]
pub fn get_view_windows_above_main(
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<bool, crate::error::AppError> {
    Ok(config.view_windows_above_main_enabled())
}

#[tauri::command]
pub fn set_view_windows_above_main(
    value: bool,
    config: State<'_, std::sync::Arc<crate::config::AppConfig>>,
) -> Result<(), crate::error::AppError> {
    config
        .set_view_windows_above_main_enabled(value)
        .map_err(crate::error::AppError::from)
}

#[cfg(windows)]
pub(crate) fn ensure_windows_notification_identity(app_handle: &AppHandle) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let app_id = app_handle.config().identifier.as_str();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(format!(r"SOFTWARE\Classes\AppUserModelId\{app_id}"))
        .map_err(|error| format!("Failed to create notification AppUserModelId key: {error}"))?;

    key.set_value("DisplayName", &WINDOWS_NOTIFICATION_DISPLAY_NAME)
        .map_err(|error| format!("Failed to write notification display name: {error}"))?;

    if let Ok(exe_path) = std::env::current_exe() {
        let icon_uri = exe_path.display().to_string();
        let _ = key.set_value("IconUri", &icon_uri);
        let _ = key.set_value("IconBackgroundColor", &"0");
    }

    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn ensure_windows_notification_identity(_app_handle: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn send_system_notification_impl(
    app_handle: &AppHandle,
    title: &str,
    body: Option<&str>,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    use tauri_winrt_notification::Toast;

    ensure_windows_notification_identity(app_handle)?;

    let app_id = app_handle.config().identifier.as_str();
    let (line1, line2) = split_notification_body(body);

    let mut toast = Toast::new(app_id).title(title);
    if !line1.is_empty() {
        toast = toast.text1(&line1);
    }
    if !line2.is_empty() {
        toast = toast.text2(&line2);
    }

    match toast.show() {
        Ok(()) => Ok(()),
        Err(error) => {
            let mut fallback = app_handle.notification().builder().title(title);
            if let Some(body) = body.filter(|value| !value.trim().is_empty()) {
                fallback = fallback.body(body);
            }
            fallback
                .show()
                .map_err(|fallback_error| {
                    format!(
                        "Failed to send Windows notification ({error}); fallback notification also failed: {fallback_error}"
                    )
                })
        }
    }
}

#[cfg(not(windows))]
fn send_system_notification_impl(
    app_handle: &AppHandle,
    title: &str,
    body: Option<&str>,
) -> Result<(), String> {
    let mut notification = app_handle.notification().builder().title(title);
    if let Some(body) = body.filter(|value| !value.trim().is_empty()) {
        notification = notification.body(body);
    }
    notification
        .show()
        .map_err(|error| format!("Failed to send notification: {error}"))
}

fn split_notification_body(body: Option<&str>) -> (String, String) {
    let normalized = body
        .unwrap_or_default()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());

    let first = lines.next().unwrap_or_default().to_string();
    let second = lines.collect::<Vec<_>>().join(" ");
    (first, second)
}

#[cfg(test)]
mod tests {
    use super::split_notification_body;

    #[test]
    fn split_notification_body_keeps_primary_and_secondary_lines() {
        assert_eq!(
            split_notification_body(Some("Session A\r\nCompleted response")),
            ("Session A".to_string(), "Completed response".to_string())
        );
        assert_eq!(
            split_notification_body(Some("Only one line")),
            ("Only one line".to_string(), String::new())
        );
        assert_eq!(
            split_notification_body(Some("First\n\nSecond\nThird")),
            ("First".to_string(), "Second Third".to_string())
        );
    }
}
