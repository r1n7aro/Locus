use tauri::AppHandle;

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
) -> Result<crate::python_runtime::PythonRuntimeState, crate::error::AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::python_runtime::python_runtime_state_with_refresh(
            Some(&app_handle),
            refresh.unwrap_or(false),
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
pub fn request_app_exit(app_handle: AppHandle) {
    crate::commands::destroy_unity_embed_control_window_on_main(&app_handle);
    app_handle.exit(0);
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
