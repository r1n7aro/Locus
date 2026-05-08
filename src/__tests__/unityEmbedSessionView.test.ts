import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity embedded session view", () => {
  it("routes the Unity overlay to the shared chat workspace", () => {
    const app = read("src/App.vue");
    const view = read("src/components/UnityEmbeddedSessionView.vue");
    const workspace = read("src/components/ChatWorkspaceView.vue");
    const command = read("src-tauri/src/commands/unity_embed.rs");

    expect(command).toContain(String.raw`const CONTROL_PIPE_NAME_PREFIX: &str = r"\\.\pipe\locus_tauri_unity_embed_";`);
    expect(command).toContain('const EMBED_URL: &str = "/unity-embed?host=tauri-overlay";');
    expect(app).toContain("const UnityEmbeddedSessionView = defineAsyncComponent");
    expect(app).toContain("isUnityEmbedWindow");
    expect(app).toContain("<UnityEmbeddedSessionView");
    expect(app).toContain("await bootstrapCritical();");
    expect(app).toContain("await registerListeners();");
    expect(view).toContain("<ChatWorkspaceView");
    expect(view).toContain("activateUnityEmbedForInput");
    expect(view).toContain("setUnityEmbedMouseActivationSuppressed");
    expect(view).toContain("getUnityEmbedFocusDebugSnapshot");
    expect(view).toContain("@pointerdown.capture=\"handlePointerDown\"");
    expect(view).not.toContain("@pointerover.capture");
    expect(view).not.toContain("@pointermove.capture");
    expect(view).toContain("layout-mode=\"auto\"");
    expect(view).not.toContain("default-session-panel-collapsed");
    expect(view).toContain('session-panel-storage-scope="unity"');
    expect(view).toContain("box-shadow: inset 0 1px 0 color-mix(in srgb, var(--border-color) 82%, var(--text-secondary) 18%);");
    expect(workspace).toContain("<ChatView");
    expect(workspace).toContain(':default-session-panel-collapsed="defaultSessionPanelCollapsed"');
    expect(workspace).toContain(':session-panel-storage-scope="sessionPanelStorageScope"');
    expect(workspace).toContain("defaultSessionPanelCollapsed: false");
    expect(workspace).toContain("<ThinkingPanel");
    expect(workspace).toContain("<ChatSidebarPanel");
    expect(workspace).toContain(':storage-scope="sessionPanelStorageScope"');
    expect(workspace).toContain("@layout-mode-change=\"handleLayoutModeChange\"");
    expect(view).not.toContain("useEmbeddedChatSession");
    expect(view).not.toContain("<EmbeddedChatPane");
  });

  it("exits the desktop app when the main window closes", () => {
    const app = read("src-tauri/src/lib.rs");
    const command = read("src-tauri/src/commands/unity_embed.rs");

    expect(app).toContain("const MAIN_WINDOW_LABEL: &str = \"main\";");
    expect(app).toContain(".on_window_event(|window, event|");
    expect(app).toContain("WindowEvent::CloseRequested");
    expect(app).toContain("api.prevent_close();");
    expect(app).toContain("commands::destroy_unity_embed_control_window_on_main(&app_handle);");
    expect(app).toContain("app_handle.exit(0);");
    expect(command).toContain("pub(crate) fn destroy_unity_embed_control_window_on_main");
    expect(command).toContain("window.destroy().or_else(|_| window.close())");
    expect(command).toContain("record_window_destroyed();");
  });

  it("boosts Unity overlay sync while the editor window is resizing", () => {
    const unityWindow = read("locus_unity/Editor/LocusEditorWindow.cs");

    expect(unityWindow).toContain("private const double ResizeSyncIntervalSeconds = 1d / 60d;");
    expect(unityWindow).toContain("private const double ResizeBoostDurationSeconds = 0.35d;");
    expect(unityWindow).toContain("resizeBoostActive ? ResizeSyncIntervalSeconds : SyncIntervalSeconds");
    expect(unityWindow).toContain("MarkResizeSyncBoost();");
  });

  it("keeps the Unity embed WebView alive across domain reloads", () => {
    const unityWindow = read("locus_unity/Editor/LocusEditorWindow.cs");
    const command = read("src-tauri/src/commands/unity_embed.rs");

    expect(unityWindow).toContain('private const string CloseReasonDomainReload = "domainReload";');
    expect(unityWindow).toContain("AssemblyReloadEvents.beforeAssemblyReload += OnBeforeAssemblyReload;");
    expect(unityWindow).toContain("AssemblyReloadEvents.afterAssemblyReload += OnAfterAssemblyReload;");
    expect(unityWindow).toContain("EditorApplication.quitting += OnEditorQuitting;");
    expect(unityWindow).toContain("SendClose(GetCloseReason())");
    expect(unityWindow).toContain("if (_assemblyReloadInProgress)");
    expect(unityWindow).toContain("return CloseReasonDomainReload;");
    expect(unityWindow).toContain("public string reason;");
    expect(unityWindow).toContain("reason = reason ?? \"\"");
    expect(command).toContain('const CLOSE_REASON_DOMAIN_RELOAD: &str = "domainReload";');
    expect(command).toContain("const TRANSIENT_CLOSE_DESTROY_DELAY: Duration = Duration::from_secs(30);");
    expect(command).toContain("struct UnityEmbedTransientCloseState");
    expect(command).toContain("fn schedule_transient_close_destroy");
    expect(command).toContain("tokio::time::sleep(TRANSIENT_CLOSE_DESTROY_DELAY).await;");
    expect(command).toContain("fn is_transient_close_reason");
    expect(command).toContain("reason == CLOSE_REASON_DOMAIN_RELOAD");
    expect(command).toContain("if is_transient_close_reason(&msg.reason)");
    expect(command).toContain("schedule_transient_close_destroy(app_handle);");
    expect(command).toContain("cancel_transient_close_destroy();");
  });

  it("suppresses Windows mouse activation for the embedded overlay outside the composer", () => {
    const view = read("src/components/UnityEmbeddedSessionView.vue");
    const service = read("src/services/unity.ts");
    const command = read("src-tauri/src/commands/unity_embed.rs");
    const app = read("src-tauri/src/lib.rs");

    expect(view).toContain("ACTIVATION_ALLOWED_SELECTOR");
    expect(view).toContain(".chat-composer-input");
    expect(view).not.toContain("\".chat-input-shell\",");
    expect(view).not.toContain("\".chat-composer\",");
    expect(view).toContain("applyMouseActivationSuppressed(true)");
    expect(service).toContain("setUnityEmbedMouseActivationSuppressed");
    expect(service).toContain("unity_embed_set_mouse_activation_suppressed");
    expect(service).toContain("activateUnityEmbedForInput");
    expect(service).toContain("unity_embed_activate_for_input");
    expect(service).toContain("getUnityEmbedFocusDebugSnapshot");
    expect(service).toContain("unity_embed_focus_debug_snapshot");
    expect(service).toContain("mouseActivateHookInstalled");
    expect(service).toContain("mouseActivateHookedHwndCount");
    expect(service).toContain("mouseActivateBlockCount");
    expect(service).toContain("overlayChildWindow");
    expect(service).toContain("activationGuardEnabled");
    expect(command).toContain("MouseActivationState");
    expect(command).toContain("guard_enabled");
    expect(command).toContain("position_child_overlay");
    expect(command).toContain("ScreenToClient");
    expect(command).toContain("SetParent");
    expect(command).toContain("SetWindowSubclass");
    expect(command).toContain("RemoveWindowSubclass");
    expect(command).toContain("WM_MOUSEACTIVATE");
    expect(command).toContain("MA_NOACTIVATE");
    expect(command).toContain("WS_EX_NOACTIVATE");
    expect(command).toContain("set_window_visible_no_activate");
    expect(command).toContain("SW_SHOWNOACTIVATE");
    expect(command).toContain("SW_HIDE");
    expect(command).toContain("let desired_visible = should_show_window_now(&window, &msg);");
    expect(command).toContain("needs_visibility_apply(desired_visible)");
    expect(command).toContain("record_applied_visibility(desired_visible)");
    expect(command).toContain("collect_descendant_windows");
    expect(command).toContain("GW_CHILD");
    expect(command).toContain("mouse_hook_sync_loop");
    expect(command).toContain("activate_for_input");
    expect(command).toContain("focus_embed_window_for_input");
    expect(command).toContain("AttachThreadInput");
    expect(command).toContain("SetKeyboardFocus");
    expect(command).toContain("COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC");
    expect(command).toContain("controller.MoveFocus");
    expect(command).not.toContain("if !is_activation_guard_enabled()");
    expect(command).toContain("mouse_activate_hook_installed");
    expect(command).toContain("mouse_activate_hooked_hwnd_count");
    expect(command).toContain("mouse_activate_block_count");
    expect(command).toContain("activation_guard_enabled");
    expect(command).toContain("overlay_child_window");
    expect(command).toContain("unity_embed_set_mouse_activation_suppressed");
    expect(command).toContain("unity_embed_activate_for_input");
    expect(command).toContain("UnityEmbedFocusDebugSnapshot");
    expect(command).toContain("foreground_title");
    expect(app).toContain("commands::unity_embed_set_mouse_activation_suppressed");
    expect(app).toContain("commands::unity_embed_activate_for_input");
    expect(app).toContain("commands::unity_embed_focus_debug_snapshot");
  });

  it("sends the current Unity host HWND for child-window mounting", () => {
    const unityWindow = read("locus_unity/Editor/LocusEditorWindow.cs");

    expect(unityWindow).toContain("GetUnityHostHwnd(_screenX, _screenY, _screenWidth, _screenHeight)");
    expect(unityWindow).toContain("FindUnityHostWindowForRect");
    expect(unityWindow).toContain("EnumWindows");
    expect(unityWindow).toContain("GetWindowThreadProcessId");
    expect(unityWindow).toContain("IntersectionArea");
  });

  it("bridges Unity asset drops into the embedded composer attachment bar", () => {
    const unityWindow = read("locus_unity/Editor/LocusEditorWindow.cs");
    const app = read("src-tauri/src/lib.rs");
    const command = read("src-tauri/src/commands/unity_embed.rs");
    const service = read("src/services/unity.ts");
    const view = read("src/components/UnityEmbeddedSessionView.vue");
    const input = read("src/components/chat/RichChatInput.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(app).toContain(".on_webview_event(|webview, event|");
    expect(app).toContain("commands::handle_unity_embed_webview_event(webview, event);");
    expect(app).toContain("commands::unity_embed_commit_asset_drop");
    expect(command).toContain("pub(crate) fn handle_unity_embed_webview_event");
    expect(command).toContain("tauri::DragDropEvent::Drop { paths, .. }");
    expect(command).toContain("unity_file_drop_asset_refs(&workspace_path, &paths)");
    expect(command).toContain("fn unity_relative_drop_path");
    expect(command).toContain("fn cache_unity_embed_asset_drag_refs");
    expect(command).toContain("pub async fn unity_embed_commit_asset_drop");
    expect(command).toContain('kind: "asset".to_string()');
    expect(command).toContain('source: Some("unity".to_string())');
    expect(command).toContain('"unity-embed-asset-drop"');
    expect(command).toContain("UnityEmbedAssetDragStatePayload");
    expect(command).toContain('"unity-embed-asset-drag-state"');
    expect(command).toContain(".disable_drag_drop_handler()");
    expect(unityWindow).toContain("public DroppedAssetRef[] assetRefs;");
    expect(unityWindow).toContain("HandleUnityObjectDrag();");
    expect(unityWindow).toContain("SendAssetDragState(false);");
    expect(unityWindow).toContain("SendAssetDragStateMessage(assetRefs);");
    expect(unityWindow).toContain('type = "assetDrag"');
    expect(unityWindow).toContain("BuildAssetRefsSignature");
    expect(unityWindow).toContain("DragAndDrop.objectReferences");
    expect(unityWindow).toContain("DragAndDrop.paths");
    expect(unityWindow).toContain('type = "assetDrop"');
    expect(unityWindow).toContain('kind = "sceneObject"');
    expect(unityWindow).toContain('source = "unity"');
    expect(unityWindow).toContain('message.type == "open" || message.type == "update"');
    expect(command).toContain("asset_refs: Option<Vec<UnityEmbedAssetRef>>");
    expect(command).toContain('"assetDrop" =>');
    expect(command).toContain('"assetDrag" =>');
    expect(service).toContain("commitUnityEmbedAssetDrop");
    expect(service).toContain("unity_embed_commit_asset_drop");
    expect(service).toContain("subscribeUnityEmbedAssetDrop");
    expect(service).toContain("subscribeUnityEmbedAssetDragState");
    expect(service).not.toContain("[Locus][UnityEmbedDrag]");
    expect(view).toContain("@dragover.capture=\"handleUnityAssetDrag\"");
    expect(view).toContain("@drop.capture=\"handleUnityAssetDrop\"");
    expect(view).toContain("commitUnityEmbedAssetDrop()");
    expect(view).toContain("subscribeUnityEmbedAssetDragState(handleUnityAssetDragState)");
    expect(view).toContain("hasUnityAssetDragState()");
    expect(view).toContain("if (!isUnityExternalFileDrag(event) && !hasUnityAssetDragState()) return false;");
    expect(view).toContain("UNITY_ASSET_DRAG_STATE_TTL_MS");
    expect(view).not.toContain("handleDragDebug");
    expect(input).toContain("assetRefAttachments = ref<AssetRefAttachment[]>([])");
    expect(input).toContain("subscribeUnityEmbedAssetDrop((payload)");
    expect(input).toContain('class="composer-attachment-list"');
    expect(input).toContain("flex-wrap: nowrap;");
    expect(input).toContain("<locus-references>");
    expect(transcript).toContain("messageAssetRefs(item.message)");
    expect(transcript).toContain('class="chat-transcript-user-asset-refs"');
  });

  it("routes embedded Ctrl-click asset refs to a locked Unity Inspector", () => {
    const chat = read("src/components/ChatView.vue");
    const service = read("src/services/unity.ts");
    const commands = read("src-tauri/src/commands/workspace.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const unityBridge = read("locus_unity/Editor/LocusBridge.cs");
    const unityTypes = read("locus_unity/Editor/LocusBridge.Types.cs");
    const embedServer = read("locus_unity/Editor/LocusEmbedHttpServer.cs");
    const app = read("src-tauri/src/lib.rs");

    expect(chat).toContain("function isUnityEmbeddedWindow()");
    expect(chat).toContain('window.location.pathname === "/unity-embed"');
    expect(chat).toContain("e.ctrlKey || e.metaKey");
    expect(chat).toContain("openUnityAssetInspector(filePath)");
    expect(service).toContain("open_unity_asset_inspector");
    expect(commands).toContain("pub async fn open_unity_asset_inspector");
    expect(bridge).toContain('send_message(project_path, "open_asset_inspector"');
    expect(app).toContain("commands::open_unity_asset_inspector");
    expect(unityBridge).toContain('case "open_asset_inspector"');
    expect(unityTypes).toContain("internal static class LocusAssetInspectorUtility");
    expect(unityTypes).toContain("TryOpenPropertyEditor");
    expect(unityTypes).toContain("LocusLockedAssetInspectorWindow");
    expect(unityTypes).not.toContain("Selection.activeObject = obj");
    expect(embedServer).toContain('request.command == "open_unity_asset_inspector"');
  });

  it("routes scene object refs to Unity selection and locked inspectors", () => {
    const markdownInject = read("src/composables/markdownInject.ts");
    const chat = read("src/components/ChatView.vue");
    const assetChip = read("src/components/AssetChip.vue");
    const service = read("src/services/unity.ts");
    const commands = read("src-tauri/src/commands/workspace.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const unityBridge = read("locus_unity/Editor/LocusBridge.cs");
    const unityTypes = read("locus_unity/Editor/LocusBridge.Types.cs");
    const embedServer = read("locus_unity/Editor/LocusEmbedHttpServer.cs");
    const app = read("src-tauri/src/lib.rs");

    expect(markdownInject).toContain("md-unity-scene-object-ref");
    expect(markdownInject).toContain("data-scene-path");
    expect(markdownInject).toContain("data-scene-object-path");
    expect(chat).toContain("selectUnitySceneObject");
    expect(chat).toContain("openUnitySceneObjectInspector");
    expect(chat).toContain("function shouldOpenUnitySceneObjectInspector");
    expect(chat).toContain("notifyUnitySceneObjectError");
    expect(chat).toContain('notificationStore.addNotice("warning"');
    expect(assetChip).toContain("notifyUnitySceneObjectError");
    expect(service).toContain("select_unity_scene_object");
    expect(service).toContain("open_unity_scene_object_inspector");
    expect(service).toContain("classifyUnitySceneObjectError");
    expect(commands).toContain("pub async fn select_unity_scene_object");
    expect(commands).toContain("pub async fn open_unity_scene_object_inspector");
    expect(bridge).toContain('send_message(project_path, "select_scene_object"');
    expect(bridge).toContain('send_message(project_path, "open_scene_object_inspector"');
    expect(app).toContain("commands::select_unity_scene_object");
    expect(app).toContain("commands::open_unity_scene_object_inspector");
    expect(unityBridge).toContain('case "select_scene_object"');
    expect(unityBridge).toContain('case "open_scene_object_inspector"');
    expect(unityTypes).toContain("internal static class LocusSceneObjectUtility");
    expect(unityTypes).toContain("Selection.activeGameObject = target");
    expect(unityTypes).toContain("OpenLockedObjectInspector(target)");
    expect(embedServer).toContain('request.command == "select_unity_scene_object"');
    expect(embedServer).toContain('request.command == "open_unity_scene_object_inspector"');
  });
});
