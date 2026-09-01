import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relativePath: string) {
  return readFileSync(resolve(cwd, relativePath), "utf8");
}

describe("Workbench floating windows", () => {
  it("mounts auxiliary Workbench windows from the main Vue runtime", () => {
    const app = read("src/App.vue");
    const shell = read("src/components/WorkbenchWindow.vue");
    const service = read("src/services/sharedWorkbenchWindow.ts");
    const rust = read("src-tauri/src/shared_workbench_window.rs");

    expect(app).toContain("sharedWorkbenchWindowHosts");
    expect(app).toContain("v-for=\"host in sharedWorkbenchWindowHosts\"");
    expect(app).toContain('<WorkbenchWindow :shared-host="host"');
    expect(shell).toContain(":owner-window=\"sharedHost?.browserWindow\"");
    expect(service).toContain("window.open(sharedWindowUrl(label)");
    expect(service).toContain("synchronizeDocument(browserWindow, label)");
    expect(service).toContain('targetDocument.documentElement.style.background = sourceBackground');
    expect(service).toContain("prepareSharedWorkbenchWindowPool");
    expect(rust).toContain("NewWindowResponse::Create");
    expect(rust).toContain("GetAsyncKeyState(VK_LBUTTON.0 as i32)");
    expect(rust).toContain("left_button_pressed");
    expect(rust).toContain("target_window_label");
    expect(rust).toContain("workbench_window_label_at");
    expect(rust).toContain("GetTopWindow(None)");
  });

  it("keeps the source editor until the shared target reports ready", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const targetDispatch = workbench.indexOf("ack = await dispatchSharedWorkbenchTransfer");
    const readyCheck = workbench.indexOf("if (!ack || ack.error", targetDispatch);
    const removeSource = workbench.indexOf("await finalizeTransferredSourceEditor", readyCheck);
    expect(targetDispatch).toBeGreaterThan(0);
    expect(readyCheck).toBeGreaterThan(targetDispatch);
    expect(removeSource).toBeGreaterThan(readyCheck);
    expect(workbench).toContain("WORKBENCH_TRANSFER_TIMEOUT_MS");
    expect(workbench).toContain("cancelAcceptedWorkbenchTransfer");
  });

  it("externalizes internal tab drags and resolves cross-window pane targets", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const tabs = read("src/components/workbench/WorkbenchEditorTabs.vue");
    const drag = read("src/composables/useWorkbenchWindowTabDrag.ts");
    expect(tabs).toContain(":drag-type=\"WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE\"");
    expect(tabs).toContain(":drag-externalize=\"(tab) => emit('drag-externalize', tab)\"");
    expect(workbench).toContain("workbenchWindowTabDrag.externalize(item, anchor)");
    expect(workbench).toContain("workbenchTabInsertionIndexAtPoint(x, tabBounds)");
    expect(workbench).toContain("workbenchSplitDirectionAtPoint({ x, y }");
    expect(drag).toContain("point.targetWindowLabel");
    expect(drag).toContain("await session.item.detach(point, session.item.anchor)");
    expect(drag).toContain("leftButtonPressed");
    expect(drag).not.toContain("DRAG_FRAME_MS");
    expect(drag).toContain("startLocusDragPreview(item.title, previewAnchor, ownerWindow.document)");
  });

  it("externalizes workspace-tree entries through the same native drag session", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const controller = read("src/composables/useInternalDrag.ts");

    expect(workbench).toContain("externalize: canExternalize ? () => handleWorkspaceTreeExternalize(dragData) : undefined");
    expect(workbench).toContain("workspaceTreeWindowDragItem(data, anchor)");
    expect(workbench).toContain("transferWorkspaceTreeEditors(descriptors, { target })");
    expect(workbench).toContain("createSharedDetachedWorkbenchWindow(");
    expect(workbench).toContain("workspaceTreeTransferSnapshot(editor)");
    expect(controller).toContain("internalDragPointReachedViewportEdge(nextPoint");
  });

  it("reveals shared windows only after transferred content reports ready", () => {
    const shell = read("src/components/WorkbenchWindow.vue");
    const service = read("src/services/sharedWorkbenchWindow.ts");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(shell).toContain("let windowRevealed = false");
    expect(shell).toContain('props.sharedHost ? "shared-window-shown" : "window-shown"');
    expect(shell).toContain("void nextTick(() => revealWorkbenchWindow(token, startedAt))");
    expect(service).not.toContain("await prepared.appWindow.show()");
    expect(workbench).toContain("if (detachedTargetCreated)");
    expect(workbench).toContain("removeSharedWorkbenchWindowHost(targetLabel)");
  });

  it("preserves the internal preview hotspot when detaching", () => {
    const tabs = read("src/components/ui/BaseTabStrip.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const service = read("src/services/workbenchWindow.ts");

    expect(tabs).toContain("externalize: props.dragExternalize");
    expect(workbench).toContain("const anchor = { ...internalDrag.previewAnchor.value }");
    expect(workbench).toContain("{ point, anchor: detachAnchor }");
    expect(service).toContain("logicalPoint.x - tabOrigin.x - tabAnchor.x");
    expect(service).toContain("logicalPoint.y - tabOrigin.y - tabAnchor.y");
  });

  it("destroys an emptied shared native window before removing its DOM host", () => {
    const service = read("src/services/sharedWorkbenchWindow.ts");
    const close = service.indexOf("await host.appWindow.close()", service.indexOf(
      "export async function removeSharedWorkbenchWindowHost",
    ));
    const removeHost = service.indexOf("sharedWorkbenchWindowHosts.splice", close);
    expect(close).toBeGreaterThan(0);
    expect(removeHost).toBeGreaterThan(close);
    expect(service).toContain("scheduleSharedWorkbenchWindowPoolReplenishment");
    expect(service).toContain("requestIdleCallback(replenish");
  });

  it("transfers Chat drafts and dirty workspace-file buffers", () => {
    const sessionEditor = read("src/components/workbench/WorkbenchSessionEditor.vue");
    const fileEditor = read("src/components/workbench/WorkspaceFilePreview.vue");
    const richInput = read("src/components/chat/RichChatInput.vue");
    expect(sessionEditor).toContain("exportTransferSnapshot");
    expect(richInput).toContain("function exportDraft(): UserMessageDraft");
    expect(fileEditor).toContain("function exportTransferSnapshot");
    expect(fileEditor).toContain("function applyTransferSnapshot");
    expect(fileEditor).toContain("preview.value.contentHash !== snapshot.contentHash");
  });
});
