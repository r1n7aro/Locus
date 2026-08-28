//! Locus-as-MCP-server: lifecycle and tauri glue.
//!
//! `reconcile` is the single entry point: it reads mcp_server.json, tears
//! down any running listener and starts a new one when enabled. Called at
//! boot and after every settings write (same lazy-ensure pattern as the MCP
//! client manager). The HTTP layer itself is tauri-free; this module wires
//! it to the app: tool execution, tool listing and the initialize
//! instructions are handed over as closures capturing the AppHandle.

pub mod config;
pub mod http;
pub mod install;
pub mod protocol;
#[cfg(test)]
mod tests;
pub mod tools;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::tool::ToolRuntimeState;

pub const MCP_SERVER_STATUS_EVENT: &str = "mcp-server-status";

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub running: bool,
    pub bound_port: Option<u16>,
    pub last_error: Option<String>,
    pub active_sessions: usize,
}

/// Tauri-managed handle around the (at most one) running listener.
#[derive(Default)]
pub struct McpServerHandle {
    inner: Mutex<RunningState>,
}

#[derive(Default)]
struct RunningState {
    task: Option<tokio::task::JoinHandle<()>>,
    ctx: Option<Arc<http::ServerContext>>,
    bound_port: Option<u16>,
    last_error: Option<String>,
}

pub fn status(handle: &McpServerHandle) -> McpServerStatus {
    let inner = handle.inner.lock().unwrap_or_else(|p| p.into_inner());
    McpServerStatus {
        running: inner.task.is_some(),
        bound_port: inner.bound_port,
        last_error: inner.last_error.clone(),
        active_sessions: inner
            .ctx
            .as_ref()
            .map(|ctx| ctx.active_sessions())
            .unwrap_or(0),
    }
}

fn emit_status(app: &AppHandle) {
    let handle = app.state::<Arc<McpServerHandle>>();
    let _ = app.emit(MCP_SERVER_STATUS_EVENT, status(&handle));
}

/// (Re)applies current settings: stops any running listener, then starts a
/// new one when enabled.
pub async fn reconcile(app: AppHandle) {
    let settings = config::load_settings();
    stop(&app);
    if !settings.enabled {
        emit_status(&app);
        return;
    }

    let runtime_state = Arc::new(ToolRuntimeState::default());
    let resolve_checkout: http::CheckoutResolver = {
        let registry = app
            .state::<Arc<crate::workspace_service::ProjectRegistry>>()
            .inner()
            .clone();
        Arc::new(move |request| {
            let checkout_id = crate::workspace_service::CheckoutId::new(request.checkout_id)
                .map_err(|error| error.to_string())?;
            let workspace_ref = crate::workspace_service::WorkspaceRef::new(
                checkout_id,
                request.expected_generation,
            );
            let scope = registry
                .resolve_workspace_ref(&workspace_ref)
                .map_err(|error| error.to_string())?;
            Ok(http::CheckoutBinding {
                checkout_id: scope.runtime().checkout_id().to_string(),
                workspace_generation: scope.runtime().generation(),
            })
        })
    };
    let dispatcher: http::ToolDispatcher = {
        let app = app.clone();
        let timeout_ms = settings.call_timeout_ms;
        let runtime_state = runtime_state.clone();
        Arc::new(move |binding, name, args| {
            let app = app.clone();
            let runtime_state = runtime_state.clone();
            Box::pin(async move {
                let checkout_id = crate::workspace_service::CheckoutId::new(binding.checkout_id)
                    .expect("resolved MCP checkout id remains valid");
                let workspace_ref = crate::workspace_service::WorkspaceRef::new(
                    checkout_id,
                    Some(binding.workspace_generation),
                );
                tools::execute_tool(app, name, args, timeout_ms, runtime_state, workspace_ref).await
            })
        })
    };
    let list_tools: http::ToolListProvider = {
        let app = app.clone();
        Arc::new(move |binding| {
            let checkout_id =
                crate::workspace_service::CheckoutId::new(binding.checkout_id.clone())
                    .expect("resolved MCP checkout id remains valid");
            let workspace_ref = crate::workspace_service::WorkspaceRef::new(
                checkout_id,
                Some(binding.workspace_generation),
            );
            tools::listed_tools(&app, &config::load_settings(), &workspace_ref)
        })
    };
    let instructions: http::InstructionsProvider = {
        let app = app.clone();
        Arc::new(move |binding| {
            let app = app.clone();
            Box::pin(async move {
                let checkout_id = crate::workspace_service::CheckoutId::new(binding.checkout_id)
                    .expect("resolved MCP checkout id remains valid");
                let workspace_ref = crate::workspace_service::WorkspaceRef::new(
                    checkout_id,
                    Some(binding.workspace_generation),
                );
                tools::build_instructions(&app, &workspace_ref).await
            })
        })
    };
    let ctx = Arc::new(http::ServerContext::new(
        settings.token.clone(),
        resolve_checkout,
        dispatcher,
        list_tools,
        instructions,
    ));

    match http::start(settings.port, ctx.clone()).await {
        Ok((addr, task)) => {
            let handle = app.state::<Arc<McpServerHandle>>();
            let mut inner = handle.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.task = Some(task);
            inner.ctx = Some(ctx);
            inner.bound_port = Some(addr.port());
            inner.last_error = None;
            eprintln!("[McpServer] listening on http://{addr}/mcp?checkoutId=<checkout-id>");
        }
        Err(e) => {
            let handle = app.state::<Arc<McpServerHandle>>();
            let mut inner = handle.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.last_error = Some(e.clone());
            eprintln!("[McpServer] start failed: {e}");
        }
    }
    emit_status(&app);
}

/// Aborts the listener (accept loop + its per-connection tasks). In-flight
/// unity calls are dropped; the unity_bridge demux tolerates abandoned
/// waiters, so this is safe for the v1 restart-on-settings-change semantics.
fn stop(app: &AppHandle) {
    let handle = app.state::<Arc<McpServerHandle>>();
    let mut inner = handle.inner.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(task) = inner.task.take() {
        task.abort();
    }
    inner.ctx = None;
    inner.bound_port = None;
}
