//! Actual Python SDK → shared assets API → YAML/live parity in an owned corpus.
use super::*;

fn report(text: &str) -> Result<Value, String> {
    let start = text
        .find('{')
        .ok_or_else(|| format!("Asset fixture returned no JSON: {}", clip(text, 1000)))?;
    let end = text
        .rfind('}')
        .ok_or("Asset fixture returned incomplete JSON")?;
    serde_json::from_str(&text[start..=end]).map_err(|e| format!("{e}: {}", clip(text, 1000)))
}

async fn ensure_frontend_host(
    app: &AppHandle,
    project: &str,
    reference: &str,
    cancel: &watch::Receiver<bool>,
) -> Result<Value, String> {
    // CliDriverConfig::requires_frontend creates the standard main window in
    // setup, including onboarding suppression and native Workbench handlers.
    // This probe waits for its real TypeScript dispatcher and assets IPC rather
    // than bypassing the View runtime with a backend-only substitute.
    if app.get_webview_window("main").is_none() {
        return Err("Asset API View acceptance requires the standard main WebView".into());
    }
    let expected: Value = serde_json::from_str(reference).map_err(|error| error.to_string())?;
    let code = r#"
const tabs = locus.workbench.tabs();
const capabilities = await locus.assets.capabilities();
return {
  ready: capabilities.starts_editor === false && capabilities.supported_operations.includes("set"),
  windowLabel: locus.windowLabel,
  workspace: locus.workspace,
  tabCount: tabs.length,
};
"#;
    let started = Instant::now();
    loop {
        if run_cancelled(cancel) {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.into());
        }
        let problem = match crate::view::request_frontend_execution(
            app, project, code, Some("main"), 5_000,
        ).await {
            Ok(reply) => {
                let result = &reply["result"];
                if result["ready"] == true
                    && result["windowLabel"] == "main"
                    && result["workspace"]["checkoutId"] == expected["checkoutId"]
                {
                    return Ok(result.clone());
                }
                format!("Unexpected frontend readiness response: {reply}")
            }
            Err(error) => error,
        };
        if started.elapsed() >= Duration::from_secs(60) {
            return Err(format!("Native frontend readiness timed out: {problem}"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub(super) async fn run(
    app: &AppHandle,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let project = resolve_project_path(config.project_path.as_deref(), app).await?;
    let root = Path::new(&project);
    crate::unity_assets::require_closed_editor(root).await?;
    let marker = root.join("Library/Locus/NativeBridge.enabled");
    let marker_before = std::fs::read(&marker).ok();
    unity_bridge::sync_native_bridge_marker(&project, true)?;
    let owned_marker = std::fs::read(&marker).map_err(|e| e.to_string())?;
    set_workspace_for_driver(app, &project).await?;
    prepare_suite_environment(&project, config, sink)?;
    let plugin = check_or_install_plugin(&project, config.install_plugin, sink).await?;
    let folder = format!(
        "Assets/LocusAssetApiTests/run-{}",
        uuid::Uuid::new_v4().simple()
    );
    let evidence = root
        .join("Library/Locus/AssetApiAcceptance")
        .join(folder.rsplit('/').next().unwrap());
    std::fs::create_dir_all(&evidence).map_err(|e| e.to_string())?;
    sink.emit(
        "suite_start",
        json!({"suite":"asset-api","project":project,"fixture":folder,"evidence":evidence}),
    );
    let result=async {
        ensure_connected(&project,config,plugin,sink,cancel).await?;
        wait_for_unity_editor_idle(&project,config,sink,cancel).await?;
        let output=execute_capture(&project,&format!("print(Locus.LocusAssetApiFixtureApi.Create({}, 32));",json!(folder))).await?;
        let manifest=report(&output)?;
        std::fs::write(evidence.join("manifest.json"),serde_json::to_vec_pretty(&manifest).unwrap()).map_err(|e|e.to_string())?;
        let closed=unity_bridge::close_current_project_unity_processes(&project,Duration::from_secs(60)).await?;
        sink.emit("suite_event",json!({"suite":"asset-api","line":"Editor closed before offline Python SDK writes","processIds":closed.process_ids}));
        crate::unity_assets::require_closed_editor(root).await?;
        let registry=app.state::<Arc<crate::workspace_service::ProjectRegistry>>();
        let runtime=registry.register(&project)?;
        let reference=serde_json::to_string(&crate::workspace_service::WorkspaceRef::for_runtime(&runtime)).map_err(|e|e.to_string())?;
        let script=include_str!("asset_api_acceptance.py.txt");
        let args=|backend:&str| vec![reference.clone(),manifest.to_string(),backend.to_string()];
        let yaml_output=run_python_sdk_script(app,&project,script,&args("yaml"),config.suite_timeout,"Offline asset API acceptance").await?;
        std::fs::write(evidence.join("yaml.log"),&yaml_output).map_err(|e|e.to_string())?;
        crate::unity_assets::require_closed_editor(root).await?;
        let yaml=report(yaml_output.lines().find(|l|l.starts_with("LOCUS_ASSET_API_ACCEPTANCE:")).ok_or_else(||format!("YAML acceptance marker missing: {yaml_output}"))?)?;
        sink.emit("suite_event",json!({"suite":"asset-api","line":"Offline Python SDK batch passed without an Editor process","details":yaml}));
        ensure_connected(&project,config,PluginPrepareOutcome::UpToDate,sink,cancel).await?;
        wait_for_unity_editor_idle(&project,config,sink,cancel).await?;
        let mut online_manifest=manifest.clone();
        for pair in online_manifest["pairs"].as_array_mut().unwrap() {pair["yaml"]=pair["yaml_live"].clone();}
        let online_output=run_python_sdk_script(app,&project,script,&[reference.clone(),online_manifest.to_string(),"yaml".into()],config.suite_timeout,"Editor-coordinated YAML acceptance").await?;
        std::fs::write(evidence.join("yaml-online.log"),&online_output).map_err(|e|e.to_string())?;
        let online=report(online_output.lines().find(|l|l.starts_with("LOCUS_ASSET_API_ACCEPTANCE:")).ok_or_else(||format!("Online YAML acceptance marker missing: {online_output}"))?)?;
        let live_output=run_python_sdk_script(app,&project,script,&args("live"),config.suite_timeout,"Live asset API acceptance").await?;
        std::fs::write(evidence.join("live.log"),&live_output).map_err(|e|e.to_string())?;
        let live=report(live_output.lines().find(|l|l.starts_with("LOCUS_ASSET_API_ACCEPTANCE:")).ok_or_else(||format!("Live acceptance marker missing: {live_output}"))?)?;
        let mut comparisons=vec![];
        for pair in manifest["pairs"].as_array().ok_or("Fixture pairs missing")? {
            let left=crate::unity_asset_core::inspect(&std::fs::read(root.join(pair["yaml"].as_str().unwrap())).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let right=crate::unity_asset_core::inspect(&std::fs::read(root.join(pair["live"].as_str().unwrap())).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            // Ignore source asset names, scene-settings metadata and backend
            // revisions, but compare EVERY fixture field, including untouched
            // fields, complete nested arrays and the managed reference registry.
            let fields=|snapshot:&crate::unity_asset_core::AssetSnapshot|->BTreeMap<String,Value> {
                snapshot.objects.iter().filter(|o|o.root_type=="Material"||o.fields.iter().any(|f|f.property_path=="/MonoBehaviour/amount"))
                    .flat_map(|o|o.fields.iter().filter(|f|(f.property_path.starts_with("/Material/")&&f.property_path!="/Material/m_Name")||(f.property_path.starts_with("/MonoBehaviour/")&&!f.property_path.starts_with("/MonoBehaviour/m_")&&!f.property_path.ends_with("/serializedVersion")))
                        .map(|f|(format!("{}:{}",o.object_id,f.property_path),f.value.clone()))).collect()
            };
            let a=fields(&left); let b=fields(&right);
            let online_snapshot=crate::unity_asset_core::inspect(&std::fs::read(root.join(pair["yaml_live"].as_str().unwrap())).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            if fields(&online_snapshot)!=a {return Err(format!("Online/offline YAML mismatch for {}",pair["yaml_live"]));}
            if a.is_empty(){return Err(format!("No fixture fields in {}",pair["yaml"]));}
            if a!=b {
                let differences=a.keys().chain(b.keys()).collect::<BTreeSet<_>>().into_iter().filter(|k|a.get(*k)!=b.get(*k))
                    .map(|k|json!({"field":k,"yaml":a.get(k),"live":b.get(k)})).collect::<Vec<_>>();
                std::fs::write(evidence.join("differences.json"),serde_json::to_vec_pretty(&differences).unwrap()).map_err(|e|e.to_string())?;
                return Err(format!("Persisted backend parity mismatch for {}: {}",pair["yaml"],serde_json::to_string(&differences).unwrap()));
            }
            comparisons.push(json!({"yaml":pair["yaml"],"live":pair["live"],"fields":a.len(),"equal":true}));
        }
        // Reload saved assets and verify managed-reference identity in Unity,
        // rather than inferring validity from parseable YAML alone.
        let verify=execute_capture(&project,&format!(r#"
var paths=UnityEditor.AssetDatabase.FindAssets("t:LocusAssetApiFixture",new[]{{{folder}}});
int checkedAssets=0;
foreach(var guid in paths){{ var path=UnityEditor.AssetDatabase.GUIDToAssetPath(guid); if(path.EndsWith("/Shared.asset"))continue;
UnityEditor.AssetDatabase.ImportAsset(path,UnityEditor.ImportAssetOptions.ForceUpdate);
var asset=UnityEditor.AssetDatabase.LoadAssetAtPath<Locus.AssetTesting.LocusAssetApiFixture>(path);
if(asset==null||asset.amount!=731||asset.root==null||!object.ReferenceEquals(asset.root,asset.alias)||!object.ReferenceEquals(asset.root,asset.root.next))throw new System.Exception("Unity round-trip failed: "+path);
checkedAssets++;}}
print("LOCUS_ASSET_ROUNDTRIP:"+checkedAssets);
"#,folder=json!(folder))).await?;
        if !verify.contains("LOCUS_ASSET_ROUNDTRIP:96"){return Err(format!("Round-trip count mismatch: {verify}"));}
        let safety_output=run_python_sdk_script(app,&project,include_str!("asset_api_safety_acceptance.py.txt"),&[reference.clone(),manifest.to_string()],Duration::from_secs(300),"Asset safety acceptance").await?;
        std::fs::write(evidence.join("safety.log"),&safety_output).map_err(|e|e.to_string())?;
        let safety=report(safety_output.lines().find(|l|l.starts_with("LOCUS_ASSET_API_SAFETY_ACCEPTANCE:")).ok_or_else(||format!("Safety acceptance marker missing: {safety_output}"))?)?;
        let frontend=ensure_frontend_host(app,&project,&reference,cancel).await?;
        std::fs::write(evidence.join("frontend-ready.json"),serde_json::to_vec_pretty(&frontend).unwrap()).map_err(|e|e.to_string())?;
        sink.emit("suite_event",json!({"suite":"asset-api","line":"Native frontend SDK and Workbench are ready","details":frontend}));
        let view_output=run_python_sdk_script(app,&project,include_str!("asset_api_view_acceptance.py.txt"),&[reference.clone(),manifest.to_string()],Duration::from_secs(300),"Native View asset API acceptance").await?;
        std::fs::write(evidence.join("view.log"),&view_output).map_err(|e|e.to_string())?;
        let view=report(view_output.lines().find(|l|l.starts_with("LOCUS_ASSET_API_VIEW_ACCEPTANCE:")).ok_or_else(||format!("View acceptance marker missing: {view_output}"))?)?;
        let result=json!({"fixture":folder,"evidence":evidence,"yaml":yaml,"yaml_online":online,"live":live,"frontend":frontend,"view":view,"safety":safety,"comparisons":comparisons,"roundtrip":verify,"passed":comparisons.len(),"failed":0});
        std::fs::write(evidence.join("result.json"),serde_json::to_vec_pretty(&result).unwrap()).map_err(|e|e.to_string())?;
        sink.emit("suite_result",json!({"suite":"asset-api","passed":comparisons.len(),"failed":0,"details":result}));
        Ok(())
    }.await;
    if let Err(error) = &result {
        let _ = std::fs::write(evidence.join("error.log"), error);
        sink.emit(
            "suite_result",
            json!({"suite":"asset-api","passed":0,"failed":1,"error":error,"evidence":evidence}),
        );
    }
    // This suite verified the project had no Editor on entry, so it owns the
    // process it launched. Keep unrelated projects and their editors untouched.
    let cleanup =
        unity_bridge::close_current_project_unity_processes(&project, Duration::from_secs(60))
            .await;
    if cleanup.is_ok() && std::fs::read(&marker).ok().as_ref() == Some(&owned_marker) {
        if let Some(before) = marker_before {
            std::fs::write(&marker, before).map_err(|e| e.to_string())?;
        } else {
            std::fs::remove_file(&marker).map_err(|e| e.to_string())?;
        }
    }
    result?;
    cleanup?;
    Ok(())
}
