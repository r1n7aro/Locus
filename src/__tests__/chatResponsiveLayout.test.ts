import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat responsive layout", () => {
  it("keeps the shared chat view stable unless the user explicitly collapses sessions", () => {
    const chatView = read("src/components/ChatView.vue");
    const picker = read("src/components/chat/SessionCompactPicker.vue");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");

    expect(chatView).toContain('layoutMode?: ChatLayoutMode;');
    expect(chatView).toContain("layoutModeChange: [mode: ResolvedChatLayoutMode]");
    expect(chatView).toContain('if (props.layoutMode === "vertical") return "vertical";');
    expect(chatView).toContain('return "horizontal";');
    expect(chatView).not.toContain("AUTO_VERTICAL_MIN_CHAT_WIDTH");
    expect(chatView).toContain("<SessionCompactPicker");
    expect(chatView).toContain("showSessionPanel");
    expect(chatView).toContain("showSessionCompactPicker");
    expect(chatView).toContain(':show-expand-panel-button="sessionPanelCollapsed && !isVerticalLayout"');
    expect(chatView).toContain("'is-vertical-layout': isVerticalLayout");
    expect(chatView).toContain(".chat-view.is-vertical-layout :deep(.chat-transcript-message.is-session)");
    expect(picker).toContain("MAX_RECENT_SESSIONS = 12");
    expect(picker).toContain("recentSessions");
    expect(picker).toContain("showNewButton");
    expect(picker).toContain("newChatShortcutLabel");
    expect(picker).toContain("formatShortcut(shortcutState.newChat)");
    expect(picker).toContain('v-if="showNewButton"');
    expect(picker).toContain('class="session-compact-expand"');
    expect(picker).toContain('class="session-compact-option-plus"');
    expect(picker).toContain('class="session-compact-option-shortcut"');
    expect(picker).toContain("font-size: 14px;");
    expect(picker).toContain(".session-compact-expand {\n  order: 3;\n  margin-left: auto;");
    expect(picker).toMatch(/\.session-compact-new,\s*\.session-compact-expand \{[\s\S]*width: 28px;[\s\S]*border: 1px solid transparent;/);
    expect(picker).toContain(".session-compact-expand:focus-visible");
    expect(picker).toContain("border-color: var(--border-strong);");
    expect(picker).toContain('class="session-compact-dropdown"');
    expect(picker).toContain('class="session-compact-option"');
    expect(sessionPanel).toContain('class="sp-session-item sp-new-session-item"');
    expect(sessionPanel).toContain(":class=\"{ active: activeSessionId === null }\"");
    expect(sessionPanel).toContain("chat.session.createNew");
    expect(sessionPanel).toContain("v-for=\"row in visibleRows\"");
    expect(sessionPanel).toContain("const visibleRows = computed<VisibleTreeRow[]>");
    expect(sessionPanel).toContain("buildSessionTree");
    expect(sessionPanel).not.toContain("sessionBeforeNewChat");
    expect(sessionPanel).not.toContain("displayRows");
    expect(sessionPanel).not.toContain("sp-footer");
  });

  it("keeps Unity and the native app on the same chat workspace contract", () => {
    const app = read("src/App.vue");
    const unityView = read("src/components/UnityEmbeddedSessionView.vue");
    const workspace = read("src/components/ChatWorkspaceView.vue");
    const sidebar = read("src/components/ChatSidebarPanel.vue");

    expect(app).toContain("loadChatWorkspaceView");
    expect(app).toContain("await registerListeners();");
    expect(unityView).toContain("<ChatWorkspaceView");
    expect(workspace).toContain("<ChatView");
    expect(workspace).toContain(":layout=\"isVerticalLayout ? 'bottom' : 'side'\"");
    expect(workspace).toContain("const workspaceRef = ref<HTMLElement | null>(null);");
    expect(workspace).toContain("ASSISTANT_PANEL_MIN_CHAT_WIDTH");
    expect(workspace).toContain('const isVerticalLayout = computed(() => props.layoutMode === "vertical");');
    expect(workspace).not.toContain("canKeepAuxiliaryPanelOnSide");
    expect(workspace).not.toContain("canRestoreAuxiliaryPanelOnSide");
    expect(workspace).toContain("assistantSidebarMaxSideWidth");
    expect(workspace).toContain(":max-side-width=\"assistantSidebarMaxSideWidth\"");
    expect(workspace).toContain("function handleWorkspaceResize(entries: ResizeObserverEntry[])");
    expect(workspace).toContain("workspaceResizeObserver = new ResizeObserver(handleWorkspaceResize)");
    expect(workspace).not.toContain("scheduleWorkspaceWidthUpdate");
    expect(workspace).toContain("saveRawContext");
    expect(sidebar).toContain("layout?: \"side\" | \"bottom\"");
    expect(sidebar).toContain("maxSideWidth?: number;");
    expect(sidebar).toContain("effectiveSidebarWidth");
    expect(sidebar).toContain("document.body.style.cursor = props.layout === \"bottom\" ? \"row-resize\" : \"col-resize\"");
  });

  it("pins chat resize to parent width instead of intrinsic child width", () => {
    const app = read("src/App.vue");
    const chatView = read("src/components/ChatView.vue");
    const shell = read("src/components/chat/ChatInputShell.vue");
    const composer = read("src/components/chat/ChatComposer.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const modelEffort = read("src/components/ModelEffortSelector.vue");

    expect(app).toMatch(/\.app-layout\s*\{[^}]*width:\s*100%;[^}]*height:\s*100%;[^}]*overflow:\s*hidden;/);
    expect(app).toContain("const appLayoutStyle = computed");
    expect(app).toContain("uiStore.nativeWindowWidth");
    expect(app).toContain(':style="appLayoutStyle"');
    expect(app).not.toMatch(/\.app-layout\s*\{[^}]*width:\s*100vw;/);
    expect(chatView).toMatch(/\.chat-view-layout\s*\{[\s\S]*flex:\s*1 1 0;[\s\S]*width:\s*100%;/);
    expect(chatView).toMatch(/\.chat-view\s*\{[\s\S]*flex:\s*1 1 0;[\s\S]*width:\s*0;/);
    expect(chatView).toMatch(/\.input-area\s*\{[\s\S]*flex:\s*0 0 auto;[\s\S]*width:\s*100%;[\s\S]*min-width:\s*0;/);
    expect(shell).toMatch(/\.chat-input-shell\s*\{[\s\S]*width:\s*100%;[\s\S]*min-width:\s*0;/);
    expect(composer).toMatch(/\.chat-composer\s*\{[\s\S]*width:\s*100%;[\s\S]*min-width:\s*0;/);
    expect(transcript).toMatch(/\.chat-transcript-scroll\s*\{[\s\S]*width:\s*100%;[\s\S]*min-width:\s*0;/);
    expect(modelEffort).toMatch(/\.model-effort-trigger\s*\{[\s\S]*min-width:\s*0;[\s\S]*max-width:\s*min\(280px, 100%\);/);
  });

  it("keeps resize work out of the hot path while the window or session splitter is moving", () => {
    const app = read("src/App.vue");
    const chatView = read("src/components/ChatView.vue");
    const theme = read("src/composables/useTheme.ts");
    const uiStore = read("src/stores/ui.ts");
    const tauriConfig = read("src-tauri/tauri.conf.json");
    const capabilities = read("src-tauri/capabilities/default.json");

    expect(tauriConfig).toContain('"shadow": false');
    expect(tauriConfig).toContain('"backgroundColor": "#1d1d21"');
    expect(capabilities).toContain("core:window:allow-set-background-color");
    expect(capabilities).toContain("core:webview:allow-set-webview-background-color");
    expect(theme).toContain("getCurrentWebviewWindow().setBackgroundColor(color)");
    expect(app).toMatch(/html,\s*body,\s*#app\s*\{[\s\S]*background:\s*var\(--bg-color\);/);
    expect(app).not.toContain("--locus-resize-anchor-width");
    expect(app).not.toContain("is-resize-anchor-right");
    expect(app).not.toContain(".app-layout.is-window-resizing .main-area");
    expect(app).toContain("content-visibility: visible;");
    expect(app).not.toContain("transform: translateZ(0);");
    expect(app).not.toContain(".app-layout.is-window-resizing .tab-content *");
    expect(app).toContain('class="tab-drag-region"');
    expect(app).toMatch(/\.tab-bar\s*\{[\s\S]*--window-resize-hit-area:\s*6px;[\s\S]*-webkit-app-region:\s*no-drag;/);
    expect(app).toMatch(/\.tab-drag-region\s*\{[\s\S]*inset:\s*var\(--window-resize-hit-area\) var\(--window-resize-hit-area\) 0 var\(--window-resize-hit-area\);[\s\S]*-webkit-app-region:\s*drag;/);
    expect(app).toMatch(/\.tab-spacer\s*\{[\s\S]*-webkit-app-region:\s*drag;[\s\S]*align-self:\s*stretch;/);

    expect(uiStore).toContain("function scheduleWindowResizeSettle");
    expect(uiStore).toContain("normalizeNativeDimension");
    expect(uiStore).toContain("MIN_TRACKABLE_WINDOW_WIDTH_PX");
    expect(uiStore).toContain("MIN_TRACKABLE_WINDOW_HEIGHT_PX");
    expect(uiStore).toContain('NATIVE_WINDOW_CLIENT_SIZE_EVENT = "locus-native-window-client-size"');
    expect(uiStore).toContain("function applyNativeWindowClientSize");
    expect(uiStore).toContain("window.onResized");
    expect(uiStore).toContain("scheduleWindowResizeSettle(event.payload.width, event.payload.height)");
    expect(uiStore).toContain("listen<NativeWindowClientSizeEvent>");
    expect(uiStore).toContain("nativeWindowWidth");
    expect(uiStore).toContain("nativeWindowHeight");
    expect(uiStore).not.toContain("windowResizeAnchor");
    expect(uiStore).not.toContain("resizeAnchorWidth");
    expect(uiStore).not.toContain("window.onMoved");
    expect(uiStore).not.toContain("screenX");
    expect(uiStore).not.toContain("recordLayoutDiagnostic");
    expect(chatView).toContain("function isLiveResizeInProgress()");
    expect(chatView).not.toContain('uiStore.noteViewportResize("chat-transcript", width)');
    expect(chatView).toContain('recordLayoutDiagnostic("chat.transcript.viewportResize"');
    expect(chatView).toContain("transcriptResizeObserver = new ResizeObserver(handleTranscriptResize)");
    expect(chatView).toContain('flushPendingTranscriptResizeReconcile("window-resize-settled")');
    expect(chatView).toContain('flushPendingTranscriptResizeReconcile("session-drag-settled")');
    expect(chatView).toContain("let pendingSessionPanelWidth: number | null = null;");
    expect(chatView).toContain("sessionSplitterFrame = requestViewportFrame(flushSessionSplitterWidth)");
    expect(chatView).toContain("sessionSplitterLayoutLeft = layoutRef.value?.getBoundingClientRect().left ?? 0;");
    expect(chatView).not.toContain("const rect = layoutRef.value.getBoundingClientRect();");
  });
});
