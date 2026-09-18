//! Frame Debugger pause/return-path diagnostic through the real unity_execute bridge.
use super::*;

const API: &str =
    include_str!("../../../skills/graphics-debugger/unity/Editor/FrameDebuggerApi.cs");

fn diagnostic_script(name: &str, bundled: &str) -> Result<String, String> {
    match std::env::var_os("LOCUS_FRAME_DEBUGGER_PROBE_ROOT") {
        Some(root) => std::fs::read_to_string(Path::new(&root).join(name))
            .map_err(|error| format!("Read Frame Debugger diagnostic {name}: {error}")),
        None => Ok(bundled.to_string()),
    }
}

async fn probe(
    project: &str,
    sink: &DriverEventSink,
    name: &str,
    code: &str,
) -> Result<String, String> {
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        unity_bridge::unity_execute_code_with_progress(project, code, |progress| {
            sink.emit(
                "probe_progress",
                json!({"suite":"frame-debugger","probe":name,"progress":progress}),
            );
        }),
    )
    .await
    .unwrap_or_else(|_| Err(format!("{name}: driver timed out after 90 seconds")));
    sink.emit(
        "suite_event",
        json!({
            "suite":"frame-debugger", "probe":name, "elapsedMs":started.elapsed().as_millis(),
            "ok":result.is_ok(), "output":result.as_ref().ok(), "error":result.as_ref().err(),
        }),
    );
    result
}

pub(super) async fn run(
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({"suite":"frame-debugger","project":project}),
    );
    crate::csharp_compile::set_enabled(true).await;
    let (_, original_status, _) = unity_bridge::query_unity_status(project).await;
    let original_status = original_status.to_string();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let fixture = format!("LocusFrameDebuggerProbe_{token}");
    let mut checks = Vec::new();
    let result = async {
        probe(project, sink, "baseline", r#"print("FD_BASELINE:" + Application.unityVersion + ":" + EditorApplication.isPlaying + ":" + EditorApplication.isPaused);"#).await?;
        unity_bridge::set_editor_status(project, "playing").await?;
        let mut cancel = watch::channel(false);
        wait_for_unity_editor_idle(project, config, sink, &mut cancel.1).await?;
        let manual_pause = probe(project, sink, "manual-pause", r#"
EditorApplication.isPaused = true;
await ctx.WaitFrames(3);
print("FD_MANUAL_PAUSE:" + EditorApplication.isPlaying + ":" + EditorApplication.isPaused);
"#).await?;
        checks.push(json!({"name":"manual-pause-return","ok":manual_pause.contains("FD_MANUAL_PAUSE:True:True")}));
        let editor_wait = probe(project, sink, "manual-paused-editor-wait", &diagnostic_script(
            "frame_debugger_editor_wait.cs.txt", include_str!("frame_debugger_editor_wait.cs.txt"))?).await;
        checks.push(json!({"name":"manual-paused-editor-wait","ok":editor_wait.as_ref().is_ok_and(|s|s.contains("FD_EDITOR_WAIT:True"))}));
        let manual_async = probe(project, sink, "manual-paused-task-delay", &diagnostic_script(
            "frame_debugger_task_delay.cs.txt", include_str!("frame_debugger_task_delay.cs.txt"))?).await;
        checks.push(json!({"name":"manual-paused-task-delay","ok":manual_async.as_ref().is_ok_and(|s|s.contains("FD_MANUAL_ASYNC:True"))}));
        let polled = probe(project, sink, "manual-paused-polled-task", &diagnostic_script(
            "frame_debugger_polled_task.cs.txt", include_str!("frame_debugger_polled_task.cs.txt"))?).await;
        checks.push(json!({"name":"manual-paused-polled-task","ok":polled.as_ref().is_ok_and(|s|s.contains("FD_POLLED_TASK:True:RanToCompletion"))}));
        probe(project, sink, "schedule-unity-task-baseline", r#"
UnityEditor.SessionState.SetBool("Locus.FrameDebuggerDriver.NativeTaskDone", false);
UnityEditor.SessionState.SetInt("Locus.FrameDebuggerDriver.NativeTaskActiveCount", -1);
System.Func<System.Threading.Tasks.Task> nativeTask = async () => {
    await System.Threading.Tasks.Task.Delay(50);
    int active = (int)typeof(Locus.LocusBridge).GetField("_activeAsyncExecuteCount", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic).GetValue(null);
    UnityEditor.SessionState.SetInt("Locus.FrameDebuggerDriver.NativeTaskActiveCount", active);
    UnityEditor.SessionState.SetString("Locus.FrameDebuggerDriver.NativeTaskStack", System.Environment.StackTrace);
    UnityEditor.SessionState.SetBool("Locus.FrameDebuggerDriver.NativeTaskDone", true);
};
UnityEditor.EditorApplication.CallbackFunction start = null;
start = () => { UnityEditor.EditorApplication.update -= start; _ = nativeTask(); };
UnityEditor.EditorApplication.update += start;
print("FD_NATIVE_BASELINE_SCHEDULED");
"#).await?;
        // No Unity execute request is active during this control interval.
        tokio::time::sleep(Duration::from_millis(800)).await;
        let isolated = probe(project, sink, "locus-task-isolation", &diagnostic_script(
            "frame_debugger_async_regression.cs.txt", include_str!("frame_debugger_async_regression.cs.txt"))?).await?;
        checks.push(json!({"name":"locus-task-isolation","ok":isolated.contains("FD_TASK_ISOLATION:True:") && isolated.contains("gameDelta=0")}));
        let timed_out = probe(project, sink, "idle-timeout", &format!(r#"
print("FD_TIMEOUT_OUTPUT");
try {{ await System.Threading.Tasks.Task.Delay(60000, cancellationToken); }}
finally {{ UnityEditor.SessionState.SetBool({key}, true); print("FD_TIMEOUT_FINALLY"); }}
"#, key=json!(format!("{fixture}_timeout")))).await;
        checks.push(json!({"name":"timeout-preserves-output","ok":matches!(&timed_out,Err(e) if e.contains("execution timed out") && e.contains("FD_TIMEOUT_OUTPUT"))}));
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let cancel_code = format!(r#"
try {{ await System.Threading.Tasks.Task.Delay(60000, cancellationToken); }}
finally {{ UnityEditor.SessionState.SetBool({key}, true); }}
"#, key=json!(format!("{fixture}_cancel")));
        let (canceled, _) = tokio::join!(
            unity_bridge::unity_execute_code_with_progress_cancellable(project, &cancel_code, cancel_rx, |_| {}),
            async move {{ tokio::time::sleep(Duration::from_millis(1500)).await; let _ = cancel_tx.send(true); }}
        );
        checks.push(json!({"name":"external-task-cancellation","ok":matches!(&canceled,Err(e) if e == unity_bridge::UNITY_EXECUTE_CANCELLED)}));
        let unwound = probe(project, sink, "cancellation-cleanup", &format!(r#"
await ctx.WaitFrames(3);
bool timeoutFinalized = UnityEditor.SessionState.GetBool({timeout}, false);
bool cancelFinalized = UnityEditor.SessionState.GetBool({cancel}, false);
UnityEditor.SessionState.EraseBool({timeout});
UnityEditor.SessionState.EraseBool({cancel});
int active = (int)typeof(Locus.LocusBridge).GetField("_activeAsyncExecuteCount", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic).GetValue(null);
print("FD_UNWOUND:" + timeoutFinalized + ":" + cancelFinalized + ":" + active);
"#, timeout=json!(format!("{fixture}_timeout")), cancel=json!(format!("{fixture}_cancel")))).await?;
        checks.push(json!({"name":"cancellation-finally-and-no-leak","ok":unwound.contains("FD_UNWOUND:True:True:1")}));
        unity_bridge::set_editor_status(project, "playing").await?;
        wait_for_unity_editor_idle(project, config, sink, &mut cancel.1).await?;
        let api = match std::env::var_os("LOCUS_FRAME_DEBUGGER_API_PATH") {
            Some(path) => std::fs::read_to_string(path).map_err(|error|error.to_string())?,
            None => API.to_string(),
        };
        let compile = unity_bridge::compile_skill_package(project, &json!({
            "packageId":"frame-debugger-driver", "sourceHash":token,
            "scripts":[{"path":"FrameDebuggerApi.cs","source":api}],
        })).await?;
        sink.emit("suite_event", json!({"suite":"frame-debugger","probe":"load-original-api","output":compile}));
        probe(project, sink, "fixture", &format!(r#"
var root = new GameObject({fixture});
root.hideFlags = HideFlags.DontSave;
var cameraObject = new GameObject("Camera");
cameraObject.transform.SetParent(root.transform);
cameraObject.transform.position = new Vector3(50000, 50000, -10);
var camera = cameraObject.AddComponent<Camera>();
camera.clearFlags = CameraClearFlags.SolidColor;
camera.backgroundColor = Color.gray;
camera.cullingMask = 1 << 31;
camera.depth = 10000;
var shader = Shader.Find("Universal Render Pipeline/Unlit") ?? Shader.Find("Unlit/Color") ?? Shader.Find("Standard");
for (int i = 0; i < 4; i++) {{
    var cube = GameObject.CreatePrimitive(PrimitiveType.Cube);
    cube.name = "Cube_" + i; cube.layer = 31;
    cube.transform.SetParent(root.transform);
    cube.transform.position = new Vector3(49997 + i * 2, 50000, 0);
    var material = new Material(shader); material.hideFlags = HideFlags.DontSave;
    material.color = new Color(i * 0.2f, 0.5f, 1f);
    cube.GetComponent<Renderer>().sharedMaterial = material;
}}
await ctx.WaitFrames(8);
print("FD_FIXTURE_READY");
"#, fixture=json!(fixture))).await?;

        let captured = probe(project, sink, "capture", &diagnostic_script(
            "frame_debugger_capture.cs.txt", include_str!("frame_debugger_capture.cs.txt"))?).await?;
        checks.push(json!({"name":"capture-returned","ok":captured.contains("FD_CAPTURE_RETURNED")}));
        checks.push(json!({"name":"capture-ready","ok":captured.contains("FD_CAPTURE:")}));

        let paused = probe(project, sink, "separate-execute-while-paused", r#"
var pausedBefore = EditorApplication.isPaused;
double before = EditorApplication.timeSinceStartup;
float gameBefore = Time.time;
await ctx.WaitFrames(5);
await System.Threading.Tasks.Task.Delay(150, cancellationToken);
print("FD_PAUSED_RETURN:" + pausedBefore + ":" + EditorApplication.isPaused + ":editorMs=" + ((EditorApplication.timeSinceStartup-before)*1000) + ":gameDelta=" + (Time.time-gameBefore));
"#).await?;
        checks.push(json!({"name":"paused-async-roundtrip","ok":paused.contains("FD_PAUSED_RETURN:True:True")}));

        if captured.contains("FD_CAPTURE_HAS_EVENTS") {
            let events = probe(project, sink, "async-event-data", &diagnostic_script(
                "frame_debugger_events.cs.txt", include_str!("frame_debugger_events.cs.txt"))?).await?;
            checks.push(json!({"name":"event-diagnostic-returned","ok":events.contains("FD_EVENT_DIAGNOSTIC_DONE")}));
            checks.push(json!({"name":"async-event-data","ok":events.contains("FD_ASYNC_EVENTS_OK:")}));
            checks.push(json!({"name":"concurrent-event-data","ok":events.contains("FD_CONCURRENT_EVENTS_OK:")}));
            checks.push(json!({"name":"event-cancel-recovery","ok":events.contains("FD_EVENT_CANCEL_RECOVERY_OK")}));
            checks.push(json!({"name":"render-target-export","ok":events.contains("FD_EXPORT:")}));
            // Leave this exception uncaught inside the snippet to verify the actual error response.
            let error = probe(project, sink, "uncaught-exception-return", r#"
print("FD_OUTPUT_BEFORE_EXCEPTION");
throw new System.InvalidOperationException("FD_EXPECTED_EXCEPTION");
"#).await;
            checks.push(json!({"name":"exception-roundtrip","ok":matches!(&error, Err(e) if e.contains("FD_EXPECTED_EXCEPTION"))}));
            checks.push(json!({"name":"error-preserves-output","ok":matches!(&error, Err(e) if e.contains("FD_OUTPUT_BEFORE_EXCEPTION"))}));
            let recovery = probe(project, sink, "execute-after-exception", r#"print("FD_RECOVERED:" + EditorApplication.isPaused + ":" + UnityEngine.FrameDebugger.enabled);"#).await?;
            checks.push(json!({"name":"post-exception-roundtrip","ok":recovery.contains("FD_RECOVERED:True:True")}));
        }
        Ok::<(), String>(())
    }.await;

    // Cleanup runs even when a probe fails. Only transient fixture objects belong to this suite.
    let cleanup = probe(project, sink, "cleanup", &format!(r#"
if (UnityEngine.FrameDebugger.enabled) {{
    var utility = System.AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetType("UnityEditorInternal.FrameDebuggerInternal.FrameDebuggerUtility", false)).FirstOrDefault(t => t != null);
    if (utility != null) {{
        var flags = System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.NonPublic;
        var target = utility.GetMethod("GetRemotePlayerGUID", flags).Invoke(null, null);
        utility.GetMethod("SetEnabled", flags).Invoke(null, new object[] {{ false, target }});
    }}
}}
var root = GameObject.Find({fixture});
if (root != null) {{
    foreach(var renderer in root.GetComponentsInChildren<Renderer>())
        if (renderer.sharedMaterial != null) UnityEngine.Object.DestroyImmediate(renderer.sharedMaterial);
    UnityEngine.Object.DestroyImmediate(root);
}}
print("FD_CLEANUP:" + UnityEngine.FrameDebugger.enabled + ":" + EditorApplication.isPaused);
"#, fixture=json!(fixture))).await;
    let restore = unity_bridge::set_editor_status(project, &original_status).await;
    checks.push(json!({"name":"cleanup","ok":cleanup.as_ref().is_ok_and(|s| s.contains("FD_CLEANUP:False:"))}));
    checks.push(json!({"name":"restore-editor-status","ok":restore.is_ok(),"status":original_status,"error":restore.err()}));
    let failed =
        checks.iter().filter(|check| check["ok"] != true).count() + usize::from(result.is_err());
    sink.emit("suite_result", json!({"suite":"frame-debugger","passed":checks.iter().filter(|check|check["ok"]==true).count(),"failed":failed,"checks":checks,"error":result.err()}));
    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "frame-debugger: {failed} failed checks; see probe outputs"
        ))
    }
}
