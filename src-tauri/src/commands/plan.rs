use crate::error::AppError;
use crate::workspace::Workspace;
use std::sync::Arc;
use tauri::{AppHandle, State};

fn sanitize_slug(path: &str) -> String {
    let last_segment = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    last_segment
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[tauri::command]
pub async fn save_plan_artifact(
    session_id: String,
    agent_id: String,
    request_text: String,
    response_text: String,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    let project_slug = sanitize_slug(&cwd);
    let data_dir = crate::commands::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let plan_dir = data_dir.join("plan").join(&project_slug);
    std::fs::create_dir_all(&plan_dir).map_err(|e| format!("Failed to create plan dir: {}", e))?;

    let now = chrono::Local::now();
    let sid_short = session_id.chars().take(8).collect::<String>();
    let filename = format!(
        "{}__{}__{}.md",
        project_slug,
        now.format("%Y%m%d-%H%M%S"),
        sid_short,
    );

    let content = format!(
        "---\nproject: {}\nworkdir: {}\nsession: {}\nagent: {}\ncreated: {}\n---\n\n## Request\n\n{}\n\n## Plan\n\n{}",
        project_slug, cwd, session_id, agent_id,
        now.to_rfc3339(), request_text, response_text
    );

    let path = plan_dir.join(&filename);
    std::fs::write(&path, &content).map_err(|e| format!("Failed to write plan file: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}
