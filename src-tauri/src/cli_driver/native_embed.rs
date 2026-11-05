//! Real desktop Vue / native WebView2 acceptance in a marked isolated project.
use super::*;
use crate::unity_embed_webview::{self as host, Surface};
use crate::workspace_service::WorkspaceRef;
#[path = "native_embed_metrics.rs"]
mod metrics;
#[path = "native_embed_p3.rs"]
mod p3;

async fn evaluate(app: &AppHandle, label: &str, code: &str) -> Result<Value, String> {
    let result = host::devtools(
        app,
        label,
        "Runtime.evaluate",
        json!({"expression":code,"awaitPromise":true,"returnByValue":true}),
    )
    .await?;
    if result.get("exceptionDetails").is_some() {
        return Err(format!("Native workbench JavaScript failed: {result}"));
    }
    Ok(result["result"]["value"].clone())
}
async fn wait_surface(id: &str) -> Result<Surface, String> {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(surface) = host::snapshot()
            .into_iter()
            .find(|s| s.window_id == id && s.ready && s.visible)
        {
            return Ok(surface);
        }
        if Instant::now() > deadline {
            return Err(format!(
                "Workbench {id} did not become ready: {:?}",
                host::snapshot()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn open(
    app: &AppHandle,
    reference: &WorkspaceRef,
    id: &str,
    session: &str,
) -> Result<Surface, String> {
    crate::commands::unity_embed_open_frontend_window(
        app.clone(),
        reference.clone(),
        crate::commands::UnityEmbedOpenFrontendWindowRequest {
            window_id: Some(id.into()),
            target_kind: "session".into(),
            target_id: Some(session.into()),
            title: Some(format!("Locus {id}")),
        },
        app.state(),
        app.state(),
    )
    .await
    .map_err(|e| e.to_string())?;
    wait_surface(id).await
}
async fn close_all(app: &AppHandle, project: &str) -> Result<(), String> {
    crate::unity_bridge::unity_execute_code(project,"foreach (var w in UnityEngine.Resources.FindObjectsOfTypeAll<Locus.LocusEditorWindow>()) w.Close(); return \"closed\";").await?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let diagnostic = host::diagnostics(app).await?;
        if host::snapshot().is_empty()
            && diagnostic["controllers"]
                .as_array()
                .is_some_and(Vec::is_empty)
            && diagnostic["retiring"] == 0
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("Embedded WebViews not reclaimed: {diagnostic}"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn check(cases: &mut Vec<Value>, name: &str, passed: bool, detail: Value) -> Result<(), String> {
    cases.push(json!({"name":name,"passed":passed,"detail":detail}));
    if passed {
        Ok(())
    } else {
        Err(format!("Native workbench check failed: {name}"))
    }
}
pub(super) async fn run(
    app: &AppHandle,
    project: &str,
    _config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    use base64::Engine as _;
    let root = Path::new(project);
    let marker: Value = serde_json::from_slice(
        &std::fs::read(root.join(".locus-native-embed-acceptance.json"))
            .map_err(|_| "native-embed requires an owned isolated test project")?,
    )
    .map_err(|e| e.to_string())?;
    let loops = if marker["quick"] == true { 3 } else { marker["cycles"].as_u64().unwrap_or(100).clamp(100, 500) as usize };
    sink.emit(
        "suite_start",
        json!({"suite":"native-embed","project":project,"cycles":loops}),
    );
    let runtime = app
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .register(root)?;
    let reference = WorkspaceRef::for_runtime(&runtime);
    crate::commands::set_unity_embed_enabled(app.clone(), reference.clone(), true, app.state())
    .await
    .map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    let mut samples = serde_json::Map::new();
    let outcome:Result<(),String>=async{
        close_all(app, project).await?;
        let a=crate::commands::create_session("Native embed A".into(),None,None,None,reference.clone(),app.state(),app.state()).await.map_err(|e|e.to_string())?;
        let b=crate::commands::create_session("Native embed B".into(),None,None,None,reference.clone(),app.state(),app.state()).await.map_err(|e|e.to_string())?;
        let first=open(app,&reference,"acceptance-a",&a).await?;
        let ready=evaluate(app,&first.label,r#"({href:location.href,ownerHref:opener.location.href,title:document.title,workbench:!!document.querySelector('.development-workbench'),ownerApp:!!opener.document.getElementById('app').__vue_app__,childScripts:document.scripts.length,text:document.body.innerText.slice(0,1800)})"#).await?;
        check(&mut cases,"related-child-without-app-bootstrap",ready["ownerApp"]==true&&ready["childScripts"]==0&&ready["workbench"]==true,ready)?;
        check(&mut cases,"no-tauri-overlay",!app.webview_windows().keys().any(|s|s.starts_with("unity-embed")),json!(app.webview_windows().keys().collect::<Vec<_>>()))?;
        let native=host::diagnostics(app).await?;
        let own_controller=native["controllers"].as_array().and_then(|items|items.iter().find(|c|c["label"]==first.label)).cloned().unwrap_or(Value::Null);
        check(&mut cases,"controller-parent-visible",own_controller["parentHwnd"]==first.parent_hwnd&&own_controller["visible"]==true,native)?;
        let deadline=Instant::now()+Duration::from_secs(20);
        while evaluate(app,&first.label,"!!document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input:not(:disabled)')").await?!=true{
            if Instant::now()>deadline{return Err("Session composer did not mount".into());}tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let (focus_tx,focus_rx)=tokio::sync::oneshot::channel();let focus_label=first.label.clone();
        app.run_on_main_thread(move||{let _=focus_tx.send(host::focus(&focus_label));}).map_err(|e|e.to_string())?;
        focus_rx.await.map_err(|e|e.to_string())??;
        evaluate(app,&first.label,"window.__nativeInput=[];document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input').addEventListener('input',e=>window.__nativeInput.push(e.isTrusted));document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input').focus();true").await?;
        let field=evaluate(app,&first.label,"(()=>{const r=document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input').getBoundingClientRect();return {x:(r.left+20)*devicePixelRatio,y:(r.top+14)*devicePixelRatio};})()").await?;
        crate::unity_bridge::unity_execute_code(project,&format!("LocusNativeViewP2.NativeInput.Click({}L,{},{});return \"clicked\";",first.parent_hwnd,first.x+field["x"].as_f64().unwrap_or(0.) as i32,first.y+field["y"].as_f64().unwrap_or(0.) as i32)).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        crate::unity_bridge::unity_execute_code(project,&format!("LocusNativeViewP2.NativeInput.Character({}L,'界');return \"typed\";",first.parent_hwnd)).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let native_input=evaluate(app,&first.label,"({value:document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input').value,trusted:window.__nativeInput.includes(true),focused:document.hasFocus()})").await?;
        check(&mut cases,"win32-unicode-input",native_input["trusted"]==true&&native_input["value"].as_str().is_some_and(|s|s.contains('界')),native_input)?;
        host::devtools(app,&first.label,"Input.insertText",json!({"text":"Native WebView 输入验证"})).await?;
        let typed=evaluate(app,&first.label,"({value:document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input').value??document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input').textContent,trusted:window.__nativeInput.includes(true),focused:document.hasFocus(),active:document.activeElement?.className,disabled:document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input').disabled})").await?;
        check(&mut cases,"trusted-browser-input",typed["trusted"]==true&&typed["value"].as_str().is_some_and(|s|s.contains("输入验证")),typed)?;
        let capture=crate::view::capture_frontend_window(app,&first.label,None).await?;
        std::fs::write(root.join("native-embed.png"),base64::engine::general_purpose::STANDARD.decode(capture["data"].as_str().ok_or("Missing capture")?).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        let repeated=open(app,&reference,"acceptance-a",&a).await?;
        check(&mut cases,"reopen-reuses-document",repeated.generation==first.generation,json!(repeated.generation))?;
        let second=open(app,&reference,"acceptance-b",&b).await?;
        let value=evaluate(app,&second.label,"document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input')?.value??document.querySelector('.workbench-editor-instance:not([inert]) .chat-composer-input')?.textContent??''").await?;
        check(&mut cases,"session-drafts-isolated",!value.as_str().unwrap_or("").contains("输入验证"),value)?;
        let api=crate::view::request_frontend_execution(app,project,"await locus.fs.write('Assets/NativeEmbed.txt','shared native workbench');return {text:await locus.fs.read('Assets/NativeEmbed.txt'),tabs:locus.workbench.tabs(),label:locus.windowLabel};",Some(&first.label),20000).await?;
        check(&mut cases,"real-scoped-frontend-file-api",api["result"]["text"]=="shared native workbench"&&api["result"]["label"]==first.label,api)?;
        let png=include_bytes!("../../icons/32x32.png").to_vec();
        let blob=app.state::<Arc<crate::binary_cache::BinaryCache>>().insert(png,"image/png".into());
        let image=evaluate(app,&first.label,&format!("new Promise(resolve=>{{const i=new Image();i.onload=()=>resolve(i.naturalWidth);i.onerror=()=>resolve(-1);i.src='http://locus-binary.localhost/blob/{blob}';}})")).await?;
        check(&mut cases,"binary-resource-protocol",image==32,image)?;
        let view=crate::view::request_frontend_execution(app,project,r#"const v=await locus.views.create({id:'native-embed-fixture',fileName:'native-embed-fixture.vue',name:'Native embed fixture',component:`<script setup>import {ref,onMounted} from 'vue';import {locus} from '@locus/view-runtime';const text=ref('loading');onMounted(async()=>{text.value=await locus.fs.read('Assets/NativeEmbed.txt');});</script><template><div class="native-embed-fixture">{{text}}</div></template>`});await locus.views.open(v.summary.id);return {views:locus.views.instances(),tabs:locus.workbench.tabs()};"#,Some(&first.label),30000).await?;
        check(&mut cases,"view-opens-in-own-workbench",view["result"]["views"].as_array().is_some_and(|v|!v.is_empty()),view)?;
        let deadline=Instant::now()+Duration::from_secs(15);
        loop {
            if evaluate(app,&first.label,"document.querySelector('.native-embed-fixture')?.textContent").await?=="shared native workbench" {break;}
            if Instant::now()>deadline {return Err("Embedded View did not execute its scoped file API".into());}
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        check(&mut cases,"embedded-view-scoped-file-api",true,json!("shared native workbench"))?;
        crate::unity_bridge::unity_execute_code(project,"UnityEditor.EditorApplication.delayCall += () => UnityEditor.EditorUtility.RequestScriptReload(); return \"reload scheduled\";").await?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        let restored=wait_surface("acceptance-a").await?;
        let preserved=evaluate(app,&first.label,"!!document.querySelector('.native-embed-fixture')").await?;
        check(&mut cases,"domain-reload-preserves-vue",restored.generation==first.generation&&preserved==true,preserved)?;
        let main=app.get_webview_window("main").ok_or("Missing main window")?;
        let was_visible=main.is_visible().map_err(|e|e.to_string())?;
        main.hide().map_err(|e|e.to_string())?;
        let hidden_probe=async {
            evaluate(app,&first.label,"window.__childFrame=false;opener.__ownerFrame=false;requestAnimationFrame(()=>window.__childFrame=true);opener.requestAnimationFrame(()=>opener.__ownerFrame=true);document.querySelector('.workbench-editor-tabs [data-workbench-tab-id]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:200,clientY:80}));true").await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            evaluate(app,&first.label,"({ownerVisibility:opener.document.visibilityState,childVisibility:document.visibilityState,ownerFrame:opener.__ownerFrame,childFrame:window.__childFrame,menu:!!document.querySelector('.base-context-menu'),opacity:document.querySelector('.base-context-menu')?getComputedStyle(document.querySelector('.base-context-menu')).opacity:null,classes:document.querySelector('.base-context-menu')?.className})").await
        }.await;
        if was_visible {main.show().map_err(|e|e.to_string())?;}
        let hidden=hidden_probe?;
        let _=check(&mut cases,"menu-with-main-hidden",hidden["menu"]==true&&hidden["opacity"]=="1",hidden);
        evaluate(app,&first.label,"document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));true").await?;
        close_all(app,project).await?;
        if marker["p3"] == true { p3::run(app, project, &reference, &a, &mut cases).await?; }
        let minutes = marker["soakMinutes"].as_u64().unwrap_or(0).min(240);
        if minutes > 0 { p3::soak(app, project, &reference, &a, minutes, sink, &mut cases).await?; }
        for _ in 0..10{open(app,&reference,"cycle",&a).await?;close_all(app,project).await?;}
        tokio::time::sleep(Duration::from_secs(10)).await;
        samples.insert("baseline".into(),metrics::capture(app).await?);
        let parked_document=samples["baseline"]["diagnostics"]["idle"].as_array().and_then(|v|v.last()).and_then(Value::as_str).ok_or("No parked document after warmup")?.to_string();
        let empty=evaluate(app,&parked_document,"window.__nativeSharedStyle=document.head.querySelector('[data-locus-shared-style]');({workbenches:document.querySelectorAll('.development-workbench').length,children:document.body.childElementCount})").await?;
        check(&mut cases,"parked-document-unmounts-vue",empty["workbenches"]==0&&empty["children"]==0,empty)?;
        let mut open_ms=Vec::new();
        for index in 0..(loops*2){
            let started=Instant::now();let surface=open(app,&reference,"cycle",&a).await?;open_ms.push(started.elapsed().as_millis());
            check(&mut cases,&format!("cycle-{index}-ready"),surface.error.is_none(),json!(surface.generation))?;
            close_all(app,project).await?;
            if (index+1)%25==0 {
                samples.insert(format!("cycle-{}",index+1),metrics::capture(app).await?);
                std::fs::write(root.join("native-embed-samples.json"),serde_json::to_vec_pretty(&samples).unwrap()).map_err(|e|e.to_string())?;
                sink.emit("suite_progress",json!({"suite":"native-embed","cycles":index+1,"total":loops*2}));
            }
            if index+1==loops {tokio::time::sleep(Duration::from_secs(5)).await;samples.insert("firstRound".into(),metrics::capture(app).await?);}
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        samples.insert("settled".into(),metrics::capture(app).await?);samples.insert("openMs".into(),json!(open_ms));
        if loops >= 100 {
            let p95 = |values: &[u128]| { let mut values = values.to_vec(); values.sort_unstable(); values[((values.len() * 95).div_ceil(100) - 1).min(values.len()-1)] };
            let early = p95(&open_ms[..loops]); let late = p95(&open_ms[loops..]);
            let _ = check(&mut cases,"open-latency-does-not-accumulate", late <= early * 2 + 30, json!({"firstRoundP95Ms":early,"secondRoundP95Ms":late}));
        }
        let before=&samples["baseline"];let after=&samples["settled"];
        check(&mut cases,"closed-scopes-and-bounded-pool",after["diagnostics"]["controllers"].as_array().is_some_and(Vec::is_empty)&&after["diagnostics"]["retiring"]==0&&after["diagnostics"]["idle"].as_array().is_some_and(|v|v.len()<=2),after["diagnostics"].clone())?;
        let _=check(&mut cases,"controller-reused-through-all-cycles",after["diagnostics"]["created"]==before["diagnostics"]["created"]&&after["diagnostics"]["reused"].as_u64().unwrap_or(0)>=before["diagnostics"]["reused"].as_u64().unwrap_or(0)+(loops*2) as u64,after["diagnostics"].clone());
        let styles=evaluate(app,&parked_document,"window.__nativeSharedStyle===document.head.querySelector('[data-locus-shared-style]')").await.unwrap_or_else(|e|json!({"error":e}));
        let _=check(&mut cases,"rebind-reuses-style-nodes",styles==true,styles);
        tokio::time::sleep(Duration::from_secs(30)).await;
        let expired=metrics::capture(app).await?;
        check(&mut cases,"all-controllers-expire",expired["diagnostics"]["idle"].as_array().is_some_and(Vec::is_empty)&&expired["diagnostics"]["retiring"]==0&&expired["diagnostics"]["controllers"].as_array().is_some_and(Vec::is_empty),expired["diagnostics"].clone())?;
        // The documented native cache expires after 30 seconds. Judge retained
        // resources after that natural lifecycle finishes, keeping the 5-second
        // warm-pool sample above as separate evidence. No forced GC is used here.
        let growth=|key:&str|expired[key].as_u64().unwrap_or(0) as i64-before[key].as_u64().unwrap_or(0) as i64;
        let _=check(&mut cases,"browser-resource-budget",growth("browserBytes")<128*1024*1024&&growth("browserHandles")<100,json!({"bytes":growth("browserBytes"),"handles":growth("browserHandles"),"settleSeconds":35}));
        samples.insert("naturalIdle35s".into(),expired);
        // Diagnostic only: forced GC must not turn a failed natural resource budget into a pass.
        let gc=open(app,&reference,"gc-diagnostic",&a).await?;
        samples.insert("domBeforeGC".into(),host::devtools(app,&gc.label,"Memory.getDOMCounters",json!({})).await?);
        host::devtools(app,&gc.label,"HeapProfiler.collectGarbage",json!({})).await?;
        samples.insert("domAfterGC".into(),host::devtools(app,&gc.label,"Memory.getDOMCounters",json!({})).await?);
        close_all(app,project).await?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        samples.insert("afterDiagnosticGC".into(),metrics::capture(app).await?);
        if cases.iter().any(|c|c["passed"]==false) { return Err("Native workbench acceptance checks failed; see cases".into()); }
        Ok(())
    }.await;
    let report = json!({"cases":cases,"samples":samples,"error":outcome.as_ref().err(),"scope":"Real IPC and browser input; no external LLM call or physical IME test."});
    std::fs::write(
        root.join("native-embed-results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let cleanup = close_all(app, project).await;
    sink.emit("suite_result",json!({"suite":"native-embed","passed":cases.iter().filter(|c|c["passed"]==true).count(),"failed":cases.iter().filter(|c|c["passed"]==false).count()+usize::from(outcome.is_err()&&!cases.iter().any(|c|c["passed"]==false)),"evidence":root.join("native-embed-results.json")}));
    outcome?;
    cleanup
}
