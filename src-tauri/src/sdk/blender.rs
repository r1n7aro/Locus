//! `locus.blender` RPC methods (`blender.*`) and `attachments.add`.
//!
//! Workspace-scoped methods bind the checkout's Blender service for the
//! duration of the request (starting it when the workspace mode is lazy),
//! like the Unity lifecycle methods do.

use super::*;
use crate::blender::exec::{self, ExecFailure};
use crate::blender::target::{self, Scope, TargetSpec};
use crate::workspace_service::{ServiceKind, WorkspaceRef};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    workspace_ref: Option<WorkspaceRef>,
    session_id: Option<String>,
    python_call: Option<String>,
    target: Option<String>,
    instance: Option<String>,
    pid: Option<u32>,
    file: Option<String>,
    path: Option<String>,
    version: Option<String>,
    installation: Option<String>,
    background: Option<bool>,
    new: Option<bool>,
    all: Option<bool>,
    allow_downgrade: Option<bool>,
    scope: Option<String>,
    payload: Option<Value>,
    timeout_ms: Option<u64>,
    caller_instance: Option<String>,
    names: Option<Vec<String>>,
}

struct Bound {
    scope: Scope,
    _execution: Arc<crate::workspace_service::AgentExecutionContext>,
    _binding: crate::workspace_service::ResolvedServiceBinding,
}

fn require_enabled(operation: &str) -> Result<(), String> {
    if crate::blender::blender_integration_enabled() {
        Ok(())
    } else {
        Err(format!(
            "service_disabled: {operation} requires Blender, which is turned off in Settings > Services"
        ))
    }
}

async fn bind(app: &AppHandle, params: &Params, operation: &str) -> Result<Bound, String> {
    require_enabled(operation)?;
    let reference = params
        .workspace_ref
        .as_ref()
        .ok_or_else(|| format!("{operation} requires workspaceRef"))?;
    let workspace = resolve_sdk_workspace_scope(app, reference, operation)?;
    let runtime = workspace.runtime();
    if runtime.services().effective_mode(ServiceKind::Blender)
        == crate::workspace_service::EffectiveServiceMode::Off
    {
        return Err(format!(
            "service_disabled: {operation} requires the Blender service, which is turned off for this workspace"
        ));
    }
    let registry = app.state::<Arc<crate::workspace_service::ProjectRegistry>>();
    let execution = registry
        .execution_context(runtime.checkout_id(), &[ServiceKind::Blender])
        .await
        .map_err(|error| format!("{operation} could not start the Blender service: {error}"))?;
    let binding = execution
        .resolve_service(ServiceKind::Blender)
        .map_err(|error| format!("{operation}: Blender service unavailable: {error}"))?;
    let scope = Scope::new(runtime.checkout_id().as_str(), runtime.root(), params.session_id.as_deref());
    Ok(Bound { scope, _execution: execution, _binding: binding })
}

fn hub_for(operation: &str) -> Result<Arc<crate::blender::hub::Hub>, String> {
    exec::running_hub().map_err(|failure| format!("{operation}: {}", failure.render()))
}

fn to_value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn failure_value(failure: &ExecFailure) -> Value {
    json!({ "ok": false, "failure": failure, "code": failure.code(), "message": failure.render() })
}

pub(super) async fn dispatch(app: &AppHandle, action: &str, value: Value) -> Result<Value, String> {
    let params: Params = if value.is_null() { Params::default() } else { parse_params(value)? };
    let operation = format!("blender.{action}");
    match action {
        "installations" => {
            require_enabled(&operation)?;
            to_value(crate::blender::installations::list_probed().await)
        }
        "install_addon" => {
            require_enabled(&operation)?;
            to_value(
                crate::blender::manage::install_selected(params.installation.as_deref(), params.all.unwrap_or(false))
                    .await?,
            )
        }
        "reload_hosts" => {
            require_enabled(&operation)?;
            let (reloaded, restart_needed) = crate::blender::manage::reload_hosts().await?;
            Ok(json!({ "reloaded": reloaded, "restartNeeded": restart_needed }))
        }
        "workspace_version" => {
            let bound = bind(app, &params, &operation).await?;
            to_value(target::workspace_version(&bound.scope))
        }
        "set_workspace_version" => {
            let bound = bind(app, &params, &operation).await?;
            target::set_pinned_version(&bound.scope.checkout_id, params.version.as_deref())?;
            to_value(target::workspace_version(&bound.scope))
        }
        "file_info" => {
            let bound = bind(app, &params, &operation).await?;
            let raw = params.path.as_deref().ok_or("file_info requires path")?;
            let path = resolve_path(&bound.scope, raw);
            let header = crate::blender::blend_header::header_cached(&path)?;
            let installations = crate::blender::installations::list_probed().await;
            let compatible = installations
                .iter()
                .filter(|item| !item.hidden)
                .filter_map(|item| {
                    let probe = item.probe.as_ref()?;
                    Some(json!({
                        "installation": item.id,
                        "version": probe.version_string,
                        "compatibility": crate::blender::blend_header::open_compatibility(&header.version, &probe.version_string),
                    }))
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "path": path.display().to_string(),
                "version": header.version,
                "compression": header.compression,
                "installations": compatible,
            }))
        }
        "instances" => {
            let bound = bind(app, &params, &operation).await?;
            let hub = hub_for(&operation)?;
            let all = params.scope.as_deref() == Some("all");
            let instances = target::scoped_instances(&hub, &bound.scope)
                .into_iter()
                .filter(|item| all || item.in_workspace)
                .collect::<Vec<_>>();
            let mut result = json!({ "instances": instances });
            if all {
                result["withoutHost"] = to_value(
                    crate::blender::hub::unregistered_processes(&hub)
                        .into_iter()
                        .map(|process| json!({ "pid": process.pid, "path": process.image_path }))
                        .collect::<Vec<_>>(),
                )?;
            }
            Ok(result)
        }
        "target.get" => {
            let bound = bind(app, &params, &operation).await?;
            Ok(json!({ "target": target::session_target(&bound.scope).map(|spec| spec.describe()) }))
        }
        "target.set" => {
            let bound = bind(app, &params, &operation).await?;
            let hub = hub_for(&operation)?;
            let spec = if params.background.unwrap_or(false) {
                TargetSpec::Background { version: params.version.clone() }
            } else if let Some(pid) = params.pid {
                let instance = hub.instance_by_pid(pid).ok_or_else(|| format!("no Blender instance with PID {pid}"))?;
                TargetSpec::Instance(instance.key.clone())
            } else if let Some(file) = params.file.as_deref() {
                let path = resolve_path(&bound.scope, file);
                TargetSpec::File(path.display().to_string())
            } else if let Some(instance) = params.instance.as_deref() {
                match TargetSpec::parse(instance)? {
                    TargetSpec::Pid(pid) => TargetSpec::Instance(
                        hub.instance_by_pid(pid).ok_or_else(|| format!("no Blender instance with PID {pid}"))?.key.clone(),
                    ),
                    other => other,
                }
            } else {
                return Err("set_target needs an instance, pid=, file= or background=True".into());
            };
            target::set_session_target(&bound.scope, spec.clone())?;
            Ok(json!({ "target": spec.describe() }))
        }
        "target.clear" => {
            let bound = bind(app, &params, &operation).await?;
            target::clear_session_target(&bound.scope);
            Ok(json!({ "target": null }))
        }
        "launch" => {
            let bound = bind(app, &params, &operation).await?;
            let hub = hub_for(&operation)?;
            let file = params.file.as_deref().map(|file| resolve_path(&bound.scope, file));
            let instance = target::launch_interactive(
                &hub,
                &bound.scope,
                file.as_deref(),
                params.installation.as_deref(),
                params.version.as_deref(),
                params.allow_downgrade.unwrap_or(false),
            )
            .await?;
            to_value(instance.snapshot())
        }
        "background" => {
            let bound = bind(app, &params, &operation).await?;
            let hub = hub_for(&operation)?;
            let file = params.file.as_deref().map(|file| resolve_path(&bound.scope, file));
            let instance = target::background_instance_with(
                &hub,
                &bound.scope,
                params.installation.as_deref(),
                params.version.as_deref(),
                file.as_deref(),
                params.new.unwrap_or(false),
            )
            .await?;
            to_value(instance.snapshot())
        }
        "close" => {
            let _bound = bind(app, &params, &operation).await?;
            let hub = hub_for(&operation)?;
            let instance = resolve_instance_arg(&hub, params.instance.as_deref())?;
            let pid = instance.registration().pid;
            if !hub.background.owns(pid) {
                return Err("only background instances that Locus launched can be closed".into());
            }
            hub.background.close(&hub, pid).await;
            Ok(json!({ "closed": pid }))
        }
        "reset_namespace" => {
            let bound = bind(app, &params, &operation).await?;
            let hub = hub_for(&operation)?;
            let instance = exec::resolve_instance(&hub, &bound.scope, params.target.as_deref())
                .await
                .map_err(|failure| failure.render())?;
            let session = bound.scope.session_id.clone().unwrap_or_default();
            instance
                .request("reset_namespace", &session, Duration::from_secs(10))
                .await?;
            Ok(json!({ "instance": instance.key }))
        }
        "release_shm" => {
            require_enabled(&operation)?;
            let hub = hub_for(&operation)?;
            let instance = resolve_instance_arg(&hub, params.instance.as_deref())?;
            let names = serde_json::to_string(&params.names.unwrap_or_default()).unwrap_or_default();
            instance.request("release_shm", &names, Duration::from_secs(10)).await?;
            Ok(json!({ "released": true }))
        }
        "execute" => execute(app, params, &operation).await,
        _ => Err(format!("Unknown Blender SDK method: blender.{action}")),
    }
}

fn resolve_path(scope: &Scope, raw: &str) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(raw.trim());
    if path.is_absolute() { path } else { scope.root.join(path) }
}

fn resolve_instance_arg(
    hub: &crate::blender::hub::Hub,
    instance: Option<&str>,
) -> Result<Arc<crate::blender::hub::Instance>, String> {
    let text = instance.map(str::trim).filter(|text| !text.is_empty()).ok_or("instance is required")?;
    match TargetSpec::parse(text)? {
        TargetSpec::Pid(pid) => hub.instance_by_pid(pid),
        TargetSpec::Instance(key) => hub.instance(&key),
        _ => None,
    }
    .ok_or_else(|| format!("no Blender instance {text}"))
}

async fn execute(app: &AppHandle, params: Params, operation: &str) -> Result<Value, String> {
    let bound = bind(app, &params, operation).await?;
    let hub = match exec::running_hub() {
        Ok(hub) => hub,
        Err(failure) => return Ok(failure_value(&failure)),
    };
    let instance = match exec::resolve_instance(&hub, &bound.scope, params.target.as_deref()).await {
        Ok(instance) => instance,
        Err(failure) => return Ok(failure_value(&failure)),
    };
    if params.caller_instance.as_deref().is_some_and(|caller| caller == instance.key) {
        return Ok(failure_value(&ExecFailure::Failed {
            message: "This code already runs inside that Blender: call bpy or blender.<helper> directly instead of locus.blender."
                .to_string(),
        }));
    }
    let mut payload = params.payload.unwrap_or_else(|| json!({}));
    if !payload.is_object() {
        return Err("payload must be an object".into());
    }
    let call = params.python_call.as_deref().and_then(crate::tool::python_call::get);
    if let Some(call) = call.as_ref() {
        // A readonly `python` call makes every nested execution readonly.
        if call.readonly {
            payload["readonly"] = Value::Bool(true);
        }
        payload["batch"] = Value::from(call.token.clone());
    }
    let timeout = exec::clamp_timeout(params.timeout_ms);
    match exec::execute_on(&hub, &bound.scope, &instance, payload, timeout, |_| {}, None).await {
        Ok(success) => {
            let mut value = to_value(success)?;
            value["ok"] = Value::Bool(true);
            Ok(value)
        }
        Err(failure) => Ok(failure_value(&failure)),
    }
}

/// `locus.attach_image`: the image file the SDK wrote for this `python` call.
pub(super) async fn attach(params: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AttachParams {
        python_call: Option<String>,
        path: String,
        #[serde(default)]
        delete: bool,
    }
    let params: AttachParams = parse_params(params)?;
    let call = params
        .python_call
        .as_deref()
        .and_then(crate::tool::python_call::get)
        .ok_or("locus.attach_image works inside a `python` tool call (no active call found)")?;
    let bytes = tokio::fs::read(&params.path)
        .await
        .map_err(|error| format!("cannot read image {}: {error}", params.path))?;
    // Only the SDK's own temporary copies are removed.
    let path = std::path::Path::new(&params.path);
    let owned_temp = path.parent().is_some_and(|parent| {
        crate::blender::installations::normalize_path(parent)
            == crate::blender::installations::normalize_path(&std::env::temp_dir())
    }) && path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("locus-attach-"));
    if params.delete && owned_temp {
        let _ = tokio::fs::remove_file(path).await;
    }
    let count = call.attach(bytes)?;
    Ok(json!({ "attached": count }))
}
