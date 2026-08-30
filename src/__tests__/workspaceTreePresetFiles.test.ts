import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("workspace tree presets and mounted files", () => {
  it("stores each preset in an independent workspace text file", () => {
    const tree = read("src-tauri/src/workspace_tree.rs");
    expect(tree).toContain('const WORKSPACE_TREE_DIR: &str = "workspace-trees";');
    expect(tree).toContain('const WORKSPACE_TREE_INDEX: &str = "index.json";');
    expect(tree).toContain('tree_dir(root).join(format!("{preset_id}.json"))');
    expect(tree).toContain("atomic_write_config(path, &bytes)");
    expect(tree).toContain("pub fn switch_preset(");
    expect(tree).toContain("pub fn create_preset(");
  });

  it("keeps preset switching inside the single-line overflow menu", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    expect(workbench).not.toContain("development-preset-select");
    expect(workbench).not.toContain("<BaseDropdown");
    expect(workbench).toContain('v-for="preset in explorerStore.snapshots[presetProjectId]?.presets || []"');
    expect(workbench).toContain("switchWorkspaceTreePreset(preset.presetId)");
    expect(workbench).toMatch(/\.development-explorer-actions\s*\{[^}]*margin-left:\s*auto/s);
  });

  it("mounts knowledge folders and local files with scoped previews", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const command = read("src-tauri/src/commands/workspace_explorer.rs");
    const preview = read("src/components/workbench/WorkspaceFilePreview.vue");
    expect(workbench).toContain('kind: "mountPath" as const');
    expect(workbench).toContain("mountKnowledgeFolder");
    expect(workbench).toContain("subscribeLocusFileDrop");
    expect(command).toContain("ensure_preview_path_authorized");
    expect(command).toContain('kind: "unity".to_string()');
    expect(preview).toContain("previewWorkspaceAsset(");
    expect(preview).toContain("AssetTextViewer");
  });

  it("drags file-explorer entries into the workspace and previews folders in tabs", () => {
    const assetView = read("src/components/AssetView.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const directoryPreview = read("src/components/workbench/WorkspaceDirectoryPreview.vue");

    expect(assetView).toContain("assetWorkspaceReferenceDragData");
    expect(assetView).toContain("workbenchReferenceInternalDragSource");
    expect(assetView).toContain('@drag-pointer-down="startAssetWorkspaceDrag"');
    expect(read("src/components/asset/AssetExplorer.vue"))
      .toContain('@drag-pointer-down="beginDrag"');
    expect(read("src/components/asset/AssetDirectoryList.vue"))
      .toContain('@pointerdown="beginDrag(entry.node, $event)"');
    expect(workbench).toContain('kind: "localDirectory"');
    expect(workbench).toContain("<WorkspaceDirectoryPreview");
    expect(directoryPreview).toContain("entry.relativePath");
    expect(directoryPreview).toContain("explorerStore.loadMount");
  });

  it("edits mounted source files while keeping Unity assets on Locus Inspector", () => {
    const command = read("src-tauri/src/commands/workspace_explorer.rs");
    const service = read("src/services/workspaceExplorer.ts");
    const fileEditor = read("src/components/workbench/WorkspaceFilePreview.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const sharedAssetPreview = read("src/components/asset/WorkspaceAssetPreview.vue");

    expect(command).toContain("pub async fn project_explorer_write_file(");
    expect(command).toContain("pub async fn workspace_file_write(");
    expect(command).toContain("expected_content_hash");
    expect(command).toContain("workspace.explorer_file_changed");
    expect(command).toContain("workspace.explorer_file_uses_unity_inspector");
    expect(service).toContain('"project_explorer_write_file"');
    expect(fileEditor).toContain("<BaseMarkdownEditor");
    expect(fileEditor).toContain("projectExplorerWriteFile(");
    expect(fileEditor).toContain("workspaceFileWrite(");
    expect(fileEditor).toContain('@shortcut-save="saveFile"');
    expect(fileEditor).toContain('emit("dirtyChange", value)');
    expect(workbench).toContain("dirtyEditorCloseDialog");
    expect(workbench).toContain("saveAndCloseDirtyEditor");
    expect(fileEditor).toContain("preview?.kind === 'unity'");
    expect(fileEditor).toContain("<WorkspaceAssetPreview");
    expect(sharedAssetPreview).toContain("UnityObjectPreview");
    expect(sharedAssetPreview).toContain("edit: props.writable");
  });

  it("copies knowledge documents and folders from the full knowledge panel", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const dragProtocol = read("src/components/knowledge/knowledgeWorkspaceDrag.ts");
    expect(explorer).toContain("startKnowledgeInternalDrag(");
    expect(dragProtocol).toContain("KNOWLEDGE_INTERNAL_DRAG_TYPE");
    expect(dragProtocol).toContain("knowledgeInternalDragSource");
    expect(workbench).toContain("resolveWorkbenchInternalDrop");
    expect(workbench).toContain("workbenchInternalDropTarget");
    expect(workbench).toContain('@dragenter.capture="onExplorerDragOver"');
    expect(workbench).toContain('@dragover.capture="onExplorerDragOver"');
    expect(workbench).toContain('@drop="onExplorerDrop"');
    expect(read("src/components/explorer/WorkspaceTree.vue"))
      .toContain("emit('dragPointerDown', item, $event)");
    expect(workbench).toContain('sourceKind: "knowledge"');
    expect(workbench).toContain('resourceKind: "knowledge" as const');
    expect(workbench).not.toContain("v-else-if=\"showKnowledge\"\n        embedded");
  });

  it("sorts system entries and resolves blank workspace space as the root tail", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const store = read("src/stores/workspaceExplorer.ts");
    const tree = read("src-tauri/src/workspace_tree.rs");
    expect(store).toContain('resourceKind: SYSTEM_RESOURCE_KIND');
    expect(store).toContain('resourceId: NEW_SESSION_SYSTEM_RESOURCE_ID');
    expect(store).toContain('resourceId: KNOWLEDGE_SYSTEM_RESOURCE_ID');
    expect(store).toContain('resourceId: COLLABORATION_SYSTEM_RESOURCE_ID');
    expect(workbench).toContain('node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID');
    expect(workbench).toContain('node.resourceId === COLLABORATION_SYSTEM_RESOURCE_ID');
    expect(workbench).toContain("dragEnabled: true");
    expect(workbench).toContain("position: snapshot.nodes.filter((node) => !node.parentNodeId).length");
    expect(workbench).toContain("await moveExplorerNodeToIntent((sourceData as WorkspaceLayoutInternalDragData).item, intent.layout)");
    expect(tree).toContain('"session" | "knowledge" | "system"');
  });

  it("uses distinct collaboration and knowledge icons", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    expect(workbench).toContain('case "collaboration": return GitMerge;');
    expect(workbench).toContain('case "knowledgeRoot": return BookOpen;');
    expect(workbench).toContain('kind: "knowledgeRoot"');
    expect(workbench).toContain("editor.resource.kind === 'knowledgeRoot'");
    expect(read("src/stores/workspaceExplorer.ts")).not.toContain("KNOWLEDGE_FOLDERS");
  });

  it("keeps knowledge and collaboration views mounted in the workspace", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    expect(workbench).toContain('v-show="group.activeEditorId === editor.editorId"');
    expect(workbench).toMatch(/<KnowledgeView\s+v-else-if="editor\.resource\.kind === 'knowledge'/);
    expect(workbench).toMatch(/<CollabView\s+v-else-if="editor\.resource\.kind === 'collaboration'/);
    expect(workbench).toContain(':workspace-ref="editorWorkspaceRef(editor)"');
    expect(workbench).toContain(':is-active="focused && group.activeEditorId === editor.editorId"');
  });

  it("routes native drops by pointer position", () => {
    const command = read("src-tauri/src/commands/unity_embed.rs");
    const richInput = read("src/components/chat/RichChatInput.vue");
    expect(command).toContain("LocusFileDropPayload { files, x, y }");
    expect(richInput).toContain("insideComposer");
    expect(richInput).toContain("getBoundingClientRect()");
  });

  it("turns the new-session row into a reference drop zone", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const zh = read("src/language/zh.json");
    const newSessionTarget = workbench.indexOf(
      'rowHit?.item.meta.kind === "newSession"',
    );
    const layoutMove = workbench.indexOf(
      'if (sourceType === WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE)',
      newSessionTarget,
    );

    expect(workbench).toContain("function workspaceLayoutAttachmentDraft(");
    expect(workbench).toContain("function newSessionDropDraft(");
    expect(workbench).toContain("workspaceLayoutNewSessionDraft(");
    expect(workbench).toContain("await createNewSessionWithAttachments(intent.target, draft)");
    expect(newSessionTarget).toBeGreaterThan(0);
    expect(newSessionTarget).toBeLessThan(layoutMove);
    expect(workbench).toContain('\"is-new-session-drop-zone\": dropAvailable');
    expect(workbench).toContain('hit.closest(".workspace-tree-row-shell.is-new-session-row")');
    expect(workbench).toContain('return "inline";');
    expect(workbench).toContain(".workspace-tree-row-shell.is-new-session-drop-zone .workspace-tree-row");
    expect(workbench).toMatch(/\.workspace-tree-row-shell\.is-new-session-drop-zone \.workspace-tree-row\)[^{]*\{[\s\S]*border:\s*1px dashed var\(--border-strong\);/);
    expect(workbench).toMatch(/\.workspace-tree-row-shell\.is-new-session-drop-zone \.workspace-tree-name\)[^{]*\{[\s\S]*text-align:\s*center;/);
    expect(zh).toContain('\"development.dropToCreateSession\": \"拖到此处以创建新会话\"');
  });

  it("resizes and persists the workspace explorer width", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    expect(workbench).toContain("onExplorerResizeStart");
    expect(workbench).toContain("onExplorerResizeMove");
    expect(workbench).toContain("onExplorerResizeEnd");
    expect(workbench).toContain("locus:developmentExplorerWidth");
    expect(workbench).toContain('class="development-explorer-resize"');
    expect(workbench).toMatch(/\.development-explorer-resize:hover::after,[\s\S]*\.development-explorer-resize\.active::after/);
  });

  it("restores and persists the knowledge explorer width", () => {
    const knowledgeView = read("src/components/KnowledgeView.vue");
    expect(knowledgeView).toContain('locus:knowledgeSidebarWidth');
    expect(knowledgeView).toContain("readKnowledgeSidebarWidth");
    expect(knowledgeView).toContain("persistKnowledgeSidebarWidth");
    expect(knowledgeView).toContain("sidebarWidth.value = readKnowledgeSidebarWidth();");
    expect(knowledgeView).toContain("if (wasResizing) persistKnowledgeSidebarWidth();");
  });
});
