import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("development workbench editor groups", () => {
  it("renders an unbounded recursive split tree with accessible separators", () => {
    const splitHost = read("src/components/workbench/WorkbenchSplitHost.vue");

    expect(splitHost.match(/<WorkbenchSplitHost/g)?.length).toBeGreaterThanOrEqual(2);
    expect(splitHost).toContain("node.first");
    expect(splitHost).toContain("node.second");
    expect(splitHost).toContain('role="separator"');
    expect(splitHost).toContain(':aria-valuenow="Math.round(node.ratio * 100)"');
    expect(splitHost).toContain("onSeparatorKeydown");
    expect(splitHost).toContain("ArrowLeft");
    expect(splitHost).toContain("ArrowDown");
    expect(splitHost).toContain("min-width: 180px");
    expect(splitHost).toContain("min-height: 140px");
  });

  it("shows one contextual half-group preview and reserves group joins for the tab strip", () => {
    const splitHost = read("src/components/workbench/WorkbenchSplitHost.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(splitHost).toContain("activeSplitDropDirection");
    expect(splitHost).toContain('class="workbench-editor-split-preview"');
    expect(splitHost).toContain(".workbench-editor-split-preview.is-left { inset: 6px 50% 6px 6px; }");
    expect(splitHost).toContain(".workbench-editor-split-preview.is-bottom { inset: 50% 6px 6px 6px; }");
    expect(splitHost).not.toContain("workbench-editor-drop-zone");
    expect(splitHost).not.toContain("['top', 'left', 'center', 'right', 'bottom']");
    expect(workbench).toContain('.workbench-editor-tabs[data-workbench-pane-id]');
    expect(workbench).toContain("workbenchTabInsertionIndexAtPoint");
    expect(workbench).toContain("workbenchSplitDirectionAtPoint");
    expect(workbench).toContain('kind: "editor"');
    expect(workbench).toContain("workbenchStore.splitPane(");
    expect(workbench).toContain("workbenchStore.moveEditor(");
  });

  it("gives composer reference drops priority over editor splitting", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const sessionEditor = read("src/components/workbench/WorkbenchSessionEditor.vue");
    const chatView = read("src/components/ChatView.vue");
    const richInput = read("src/components/chat/RichChatInput.vue");
    const composerTarget = workbench.indexOf(
      'context.hit.closest<HTMLElement>(".chat-composer")',
    );
    const tabTarget = workbench.indexOf(
      'context.hit.closest<HTMLElement>(".workbench-editor-tabs[data-workbench-pane-id]")',
    );
    const groupTarget = workbench.indexOf(
      'context.hit.closest<HTMLElement>(\n    ".workbench-editor-group[data-workbench-pane-id]"',
    );

    expect(composerTarget).toBeGreaterThan(0);
    expect(composerTarget).toBeLessThan(tabTarget);
    expect(composerTarget).toBeLessThan(groupTarget);
    expect(workbench).toContain('intent: { kind: "composer", paneId, editorId: editor.editorId }');
    expect(workbench).toContain("sessionEditorRefs.get(intent.editorId)?.applyDraftPrefill(draft)");
    expect(workbench).toContain("function composerAcceptsCurrentDrag(");
    expect(workbench).toContain(":reference-drop-available=\"composerAcceptsCurrentDrag(paneId, editor)\"");
    expect(workbench).toContain(':reference-drop-active="');
    expect(workbench).toContain("composerDropTarget?.paneId === paneId");
    expect(sessionEditor).toContain(':reference-drop-available="referenceDropAvailable"');
    expect(sessionEditor).toContain(':reference-drop-active="referenceDropActive"');
    expect(chatView).toContain(':reference-drop-available="referenceDropAvailable"');
    expect(chatView).toContain(':reference-drop-active="referenceDropActive"');
    expect(chatView).toMatch(/\.input-area\.is-reference-drop-available\s*\{[\s\S]*z-index:\s*41;/);
    expect(read("src/components/workbench/WorkbenchSplitHost.vue")).toMatch(
      /\.workbench-editor-split-preview-layer\s*\{[\s\S]*z-index:\s*40;/,
    );
    expect(richInput).toContain(':drop-available="localFileDragActive || referenceDropAvailable || referenceDropActive"');
    expect(richInput).toContain(':drop-active="localFileDragActive || referenceDropActive"');
    expect(workbench).toContain("WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE");
    expect(workbench).toContain("referenceAttachmentDraft");
    expect(workbench).toContain("referenceEditorDescriptors");
    expect(workbench).toContain("placeWorkbenchReferenceDrag");
    expect(workbench).toContain("nativeWorkbenchDropDecisionAt");
  });

  it("opens conversation assets through the shared scoped preview facility", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const editor = read("src/components/workbench/WorkbenchAssetEditor.vue");
    const preview = read("src/components/asset/WorkspaceAssetPreview.vue");
    const unityPreview = read("src/components/unity-preview/UnityObjectPreview.vue");

    expect(workbench).toContain("<WorkbenchAssetEditor");
    expect(editor).toContain("WorkspaceAssetPreview");
    expect(editor).toContain(':workspace-ref="workspaceRef"');
    expect(preview).toContain("UnityObjectPreview");
    expect(preview).toContain(':workspace-ref="workspaceRef"');
    expect(unityPreview).toContain("props.workspaceRef ?? workspaceContextStore.focusedWorkspaceRef");
  });

  it("uses VS Code-style preview tabs and keeps one-tab split groups draggable", () => {
    const tabs = read("src/components/workbench/WorkbenchEditorTabs.vue");
    const baseTabs = read("src/components/ui/BaseTabStrip.vue");
    const store = read("src/stores/workbench.ts");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(store).toContain("group.tabs.length >= 2");
    expect(store).toContain("showSingleTab && group.tabs.length === 1");
    expect(workbench).toContain(":show-single-tab=\"workbenchWindow.layout.kind === 'split'\"");
    expect(workbench).toContain(":show-single-tabs=\"workbenchWindow.layout.kind === 'split'\"");
    expect(read("src/components/workbench/WorkbenchSplitHost.vue")).toContain(
      "props.showSingleTabs && count === 1",
    );
    expect(store).toContain("candidate.preview && !candidate.pinned && !candidate.dirty");
    expect(store).toContain("workbenchResourceKey(editor.resource) === resourceKey");
    expect(tabs).toContain('v-if="visible"');
    expect(tabs).toContain("editor.preview && !editor.pinned");
    expect(tabs).toContain("<BaseTabStrip");
    expect(tabs).toContain("pin-on-double-click");
    expect(tabs).toContain('tab-id-attribute="data-workbench-tab-id"');
    expect(baseTabs).toContain('@dblclick="handleDoubleClick($event, tab)"');
    expect(baseTabs).toContain("handleAuxClick");
    expect(baseTabs).toContain("activeDropIndex() === index");
    expect(baseTabs).toContain("base-tab-drop-end");
    expect(baseTabs).toContain("font-style: italic");
  });

  it("switches the complete editor-group state with the focused single workspace", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const store = read("src/stores/workbench.ts");

    expect(workbench).toContain('displaySettings.workspaceDisplayMode === "single"');
    expect(workbench).toContain("workspaceContextStore.focusedCheckout?.checkoutId ?? null");
    expect(workbench).toContain("?? workbenchStore.workspaceScope(WORKBENCH_WINDOW_ID)");
    expect(workbench).toContain("workbenchStore.switchWorkspaceScope(");
    expect(workbench).toContain("syncWorkbenchWorkspaceScope(workspaceScopeId)");
    expect(store).toContain("workspaceScopes");
    expect(store).toContain("persist(windowId);");
    expect(store).toContain("restoreStoredWindow(windowId, nextScopeId)");
    expect(store).toContain(":workspace:${encodeURIComponent(normalizedScopeId)}");
  });

  it("keeps close fallback inside the active project and hands off pane focus before disposal", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const store = read("src/stores/workbench.ts");
    const closeStart = workbench.indexOf("async function closeWorkbenchEditor(");
    const closeEnd = workbench.indexOf("\nfunction pinWorkbenchEditor", closeStart);
    const closeFlow = workbench.slice(closeStart, closeEnd);

    expect(store).toContain("function closeFallbackEditor(");
    expect(store).toContain("editor.checkoutBinding?.checkoutId === checkoutId");
    expect(store).toContain("editor.resource.projectId === removed.resource.projectId");
    expect(store).toContain("options.replacePreview !== false");
    expect(workbench).toContain("findWorkbenchScopeFallback(projectId, checkoutId, focusedPaneId)");
    expect(workbench).toContain("openWorkbenchScopeFallback(projectId, checkoutId, focusedPaneId)");
    expect(closeFlow.indexOf("await focusWorkbenchEditor(")).toBeGreaterThan(0);
    expect(closeFlow.indexOf("await workspaceContextStore.disposePane(")).toBeGreaterThan(
      closeFlow.indexOf("await focusWorkbenchEditor("),
    );
  });

  it("keeps every active group editor mounted and isolates session stream state", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const sessionEditor = read("src/components/workbench/WorkbenchSessionEditor.vue");
    const chatView = read("src/components/ChatView.vue");
    const embeddedSession = read("src/composables/useEmbeddedChatSession.ts");

    expect(workbench).toContain('v-for="editor in group.tabs"');
    expect(workbench).toContain('v-show="group.activeEditorId === editor.editorId"');
    expect(sessionEditor).toContain("<ChatView");
    expect(sessionEditor).toContain("scoped-session");
    expect(sessionEditor).not.toContain("<EmbeddedChatPane");
    expect(sessionEditor).not.toContain(":deep(.chat");
    expect(chatView).toContain("<ChatTranscript");
    expect(chatView).toContain("<ChatStatusIndicators");
    expect(chatView).toContain("<RichChatInput");
    expect(chatView).toContain(':session-key="sessionSurfaceKey"');
    expect(sessionEditor).toContain(':composer-value="inputText"');
    expect(sessionEditor).toContain('@update-composer-value="inputText = $event"');
    expect(sessionEditor).toContain("initialSessionId: requestedSessionId");
    expect(sessionEditor).toContain("sessionKey");
    expect(embeddedSession).toContain("hydrateExistingSession");
    expect(embeddedSession).toContain("sessionStates");
    expect(embeddedSession).toContain("bindSessionStreamEventConsumer");
    expect(embeddedSession).not.toContain("sessionIdToKey");
    expect(workbench).toContain("setActiveSessionInPane(");
  });

  it("turns the focused session tab into a clean new-session editor", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const sessionEditor = read("src/components/workbench/WorkbenchSessionEditor.vue");
    const chatView = read("src/components/ChatView.vue");

    expect(sessionEditor).toContain("function handleNewSessionRequest(): void");
    expect(sessionEditor).toContain("resetSession();");
    expect(sessionEditor).toContain('emit("new-session-requested", { editorId: props.editor.editorId });');
    expect(sessionEditor).toContain('@new-chat="handleNewSessionRequest"');
    expect(sessionEditor).not.toContain('@new-chat="resetSession"');

    expect(workbench).toContain("function handleWorkbenchNewSessionRequested(");
    expect(workbench).toContain('kind: "newSession" as const');
    expect(workbench).toContain('title: t("chat.session.newSession")');
    expect(workbench).toContain('@new-session-requested="handleWorkbenchNewSessionRequested(paneId, $event)"');
    expect(workbench).toContain(':shortcut-active="focused && group.activeEditorId === editor.editorId"');
    expect(chatView).toContain("if (props.shortcutActive === false) return;");
  });
});
