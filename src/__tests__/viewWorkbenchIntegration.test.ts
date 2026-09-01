import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relativePath: string) {
  return readFileSync(resolve(cwd, relativePath), "utf8").replace(/\r\n/g, "\n");
}

describe("View Workbench integration", () => {
  it("opens View packages as standard Workbench editor tabs", () => {
    const runtime = read("src-tauri/src/view.rs");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const editor = read("src/components/workbench/WorkbenchViewEditor.vue");
    const tabs = read("src/components/workbench/WorkbenchEditorTabs.vue");

    const openStart = runtime.indexOf("pub async fn open_view_in_workbench(");
    const unityStart = runtime.indexOf("pub async fn open_view_unity_embed_window(", openStart);
    const openView = runtime.slice(openStart, unityStart);
    expect(openView).toContain("VIEW_WORKBENCH_OPEN_EVENT");
    expect(openView).toContain("set_view_tab_host_scoped_sync(");
    expect(openView).toContain("keep_existing_for_host: true");
    expect(openView).not.toContain("build_view_window(");
    expect(openView).not.toContain("merge_view_tab_into_host_window(");

    expect(workbench).toContain("VIEW_WORKBENCH_OPEN_EVENT");
    expect(workbench).toContain("openViewInWorkbench(event.payload)");
    expect(workbench).toContain('kind: "view"');
    expect(workbench).toContain("<WorkbenchViewEditor");
    expect(workbench).toContain(":view-id=\"editor.resource.viewId\"");
    expect(workbench).toContain("ensureWorkbenchViewEditorReady");
    expect(editor).toContain("viewContentMount(workspaceRef, request)");
    expect(editor).toContain("viewContentHide(props.workspaceRef, props.viewId)");
    expect(tabs).toContain('case "view"');
  });

  it("hands the native View child window over before removing a transferred tab", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const ready = workbench.indexOf("await ensureWorkbenchViewEditorReady(result.editorId)");
    const acknowledge = workbench.indexOf("const acknowledgement: WorkbenchWindowTransferAckPayload", ready);
    const relinquish = workbench.indexOf("workbenchViewEditorRefs.get(editorId)?.relinquish()");
    const remove = workbench.indexOf("await finalizeTransferredSourceEditor(paneId, editorId)", relinquish);
    expect(ready).toBeGreaterThan(0);
    expect(acknowledge).toBeGreaterThan(ready);
    expect(relinquish).toBeGreaterThan(0);
    expect(remove).toBeGreaterThan(relinquish);
  });
});
