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
    expect(workbench.slice(composerTarget, tabTarget)).toContain('previewMode: "inline"');
    expect(workbench).toContain("sessionEditorRefs.get(intent.editorId)?.appendComposerDraft(draft)");
    expect(sessionEditor).toContain("chatViewRef.value?.appendComposerDraft(draft)");
    expect(chatView).toContain("composerPanelRef.value.appendDraft(draft)");
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
    expect(workbench).toContain(":show-single-tab=\"props.auxiliary || workbenchWindow.layout.kind === 'split'\"");
    expect(workbench).toContain(":show-single-tabs=\"props.auxiliary || workbenchWindow.layout.kind === 'split'\"");
    expect(read("src/components/workbench/WorkbenchSplitHost.vue")).toContain(
      "props.showSingleTabs && count === 1",
    );
    expect(store).toContain("candidate.preview && !candidate.pinned && !candidate.dirty");
    expect(store).toContain("workbenchResourceKey(editor.resource) === resourceKey");
    expect(tabs).toContain('v-if="visible"');
    expect(tabs).toContain("editor.preview && !editor.pinned");
    expect(tabs).toContain("runningSessionIds");
    expect(tabs).toContain('running: editor.resource.kind === "session"');
    expect(tabs).toContain("<BaseTabStrip");
    expect(tabs).toContain("pin-on-double-click");
    expect(tabs).toContain('tab-id-attribute="data-workbench-tab-id"');
    expect(tabs).toContain("<BaseContextMenu");
    expect(tabs).toContain("@tab-contextmenu=\"openTabContextMenu\"");
    expect(tabs).toContain("closeFromContextMenu('left')");
    expect(tabs).toContain("closeFromContextMenu('right')");
    expect(tabs).toContain("closeFromContextMenu('all')");
    expect(workbench).toContain('@close-many="closeWorkbenchEditors(paneId, $event)"');
    expect(baseTabs).toContain('@dblclick="handleDoubleClick($event, tab)"');
    expect(baseTabs).toContain("handleAuxClick");
    expect(baseTabs).toContain('@contextmenu="openTabContextMenu($event, tab)"');
    expect(baseTabs).toContain("activeDropIndex() === index");
    expect(baseTabs).toContain("base-tab-drop-end");
    expect(baseTabs).toContain("running: tab.running");
    expect(baseTabs).toContain('class="base-tab-icon"');
    expect(baseTabs).toContain("base-tab-icon-breathe 1.8s ease-in-out infinite");
    expect(baseTabs).not.toContain("base-tab-title-scan");
    expect(baseTabs).toContain("font-style: italic");
  });

  it("locates workspace-tree resources across the current window and reserves duplication for drag placement", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const activateStart = workbench.indexOf("async function activateItem(");
    const activateEnd = workbench.indexOf("\nfunction toggleItem", activateStart);
    const activateFlow = workbench.slice(activateStart, activateEnd);
    const dropStart = workbench.indexOf("async function commitWorkbenchInternalDrop(");
    const dropEnd = workbench.indexOf("\nconst workbenchInternalDropTarget", dropStart);
    const dropFlow = workbench.slice(dropStart, dropEnd);

    expect(workbench).toContain("function matchingWorkbenchEditors(");
    expect(workbench).toContain("Object.values(workbenchWindow.value.groups).flatMap");
    expect(workbench).toContain("if (matches.length === 0) return openWorkbenchResource(descriptor, options);");
    expect(workbench).toContain("flashWorkspaceTreeEditorTabs(matches.map((match) => match.editor))");
    expect(workbench).toContain("focusPane: alreadyForeground");
    expect(workbench).toContain("{ activate: focusPane }");
    expect(workbench).toContain("workspace-tree-attention-a");
    expect(workbench).toContain("workspace-tree-tab-attention-a 420ms ease-out");
    expect(workbench).toContain("outline: 1px solid transparent");
    expect(workbench).toContain("outline-offset: -1px");
    expect(workbench).toContain("outline-color: var(--accent-color)");
    expect(workbench).toContain("box-shadow: inset 0 0 0 1px var(--accent-color)");
    expect(workbench).not.toContain("18%, 62% { box-shadow");
    expect(activateFlow).toContain("openWorkbenchResourceFromWorkspaceTree(");
    expect(activateFlow).not.toContain("openWorkbenchResource(");
    expect(dropFlow).toContain("await openWorkbenchResource(descriptor, {");
    expect(dropFlow).toContain("allowDuplicate: descriptor.resource.kind === \"session\"");
  });

  it("routes session clicks through explicit activate, reuse, and new-tab decisions", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const navigation = read("src/components/workbench/workbenchSessionNavigation.ts");

    expect(navigation).toContain('export type WorkbenchSessionNavigationMode = "activate" | "reuse" | "newTab"');
    expect(navigation).toContain("context.splitLayout");
    expect(navigation).toContain("context.focusedGroupTabCount > 1");
    expect(workbench).toContain("if ((event?.detail ?? 1) > 1) return;");
    expect(workbench).not.toContain("systemDoubleClickIntervalMs");
    expect(workbench).not.toContain("workspaceSessionClickTimers");
    expect(workbench).not.toContain("ImmediateWorkspaceSessionReuseClick");
    expect(workbench).toContain('if (mode === "activate")');
    expect(workbench).toContain("refreshServices: false");
    expect(workbench).toContain("if (!context) return null;");
    expect(workbench).toContain("workbenchStore.replaceEditor(");
    expect(workbench).toContain("preserveReplacedWorkspaceSessionDraft(current);");
    expect(workbench).toContain("restoreReplacedWorkspaceSessionDraft(descriptor.resource, editor.editorId)");
    expect(workbench).toContain('allowDuplicate: descriptor.resource.kind === "newSession"');
    expect(workbench).toContain("activateWorkspaceSessionItem(item, event);");
  });

  it("keeps same-checkout tab activation local to the workbench pane", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("lastRefreshedCheckoutServicesScopeKey");
    expect(workbench).toContain("pendingCheckoutServicesRefreshes");
    expect(workbench).toContain("workspaceRefScopeKey(workspaceContextStore.focusedWorkspaceRef)");
    expect(workbench).toContain("!== lastRefreshedCheckoutServicesScopeKey");
    expect(workbench).toContain("binding.expectedGeneration !== context.workspaceGeneration");
    expect(workbench).toContain("const editorWorkspaceRefs = new Map<string, WorkspaceRef>()");
    expect(workbench).toContain("cached && cached.expectedGeneration === expectedGeneration");
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
    expect(sessionEditor).toContain(':undo-conversation="rollbackConversation"');
    expect(sessionEditor).toContain(':undo-files-and-conversation="rollbackFilesAndConversation"');
    expect(sessionEditor).toContain(':check-undo-dirty="checkUndoDirty"');
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

    expect(sessionEditor).toContain('request.source !== "shortcut" || props.newChatShortcutAction !== "newTab"');
    expect(sessionEditor).toContain("resetSession();");
    expect(sessionEditor).toContain('emit("new-session-requested", {');
    expect(sessionEditor).toContain('@new-chat="handleNewSessionRequest"');
    expect(sessionEditor).not.toContain('@new-chat="resetSession"');

    expect(workbench).toContain("async function handleWorkbenchNewSessionRequested(");
    expect(workbench).toContain('if (action === "keepCurrent") return;');
    expect(workbench).toContain('if (action === "newTab")');
    expect(workbench).toContain("sessionEditorRefs.get(newEditor.editorId)?.focusComposerInput()");
    expect(workbench).toContain("allowDuplicate: true");
    expect(workbench).toContain('kind: "newSession" as const');
    expect(workbench).toContain('title: t("chat.session.newSession")');
    expect(workbench).toContain('@new-session-requested="handleWorkbenchNewSessionRequested(paneId, $event)"');
    expect(workbench).toContain(':shortcut-active="focused && group.activeEditorId === editor.editorId"');
    expect(workbench).toContain(':new-chat-shortcut-action="newSessionShortcutAction(group, editor)"');
    expect(chatView).toContain("if (props.shortcutActive === false) return;");
    expect(chatView).toContain('handleNewChatRequest("shortcut")');
    expect(chatView).toContain("focusComposerInput,");
    expect(sessionEditor).toContain("async function focusComposerInput(): Promise<void>");
  });

  it("opens Send to Locus API attachments in the main workbench session surface", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const sessionEditor = read("src/components/workbench/WorkbenchSessionEditor.vue");

    expect(workbench).toContain("if (payload.sendMode)");
    expect(workbench).toContain("handleSendToLocusDraft(");
    expect(workbench).toContain('if (WORKBENCH_WINDOW_ID !== "main") return;');
    expect(workbench).toContain('uiStore.setPage("development");');
    expect(workbench).toContain("sendToLocusCheckout(workspaceRef)");
    expect(workbench).toContain("createNewSessionWithAttachmentsForCheckout(checkout, draft)");
    expect(workbench).toContain("attachmentDraft({ localFiles: payload.files })");
    expect(workbench).toContain("attachmentDraft({ assetRefs: payload.refs })");
    expect(workbench).toContain("mergeAttachmentDrafts(");
    expect(sessionEditor).toContain("function exportComposerDraft()");
    expect(sessionEditor).toContain("defineExpose({");
    expect(sessionEditor).toContain("appendComposerDraft,");
    expect(sessionEditor).toContain("exportComposerDraft,");
  });
});
