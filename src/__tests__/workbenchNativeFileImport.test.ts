// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import ts from "typescript";
import { ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";

// Execute the production native file drag handlers of the workbench with
// stubbed services: OS files released on a Files folder are copied, anything
// else keeps the existing reference behavior.
const source = readFileSync("src/components/workbench/DevelopmentWorkbench.vue", "utf8")
  .split('<script setup lang="ts">')[1]!.split("</script>")[0]!;
const parsed = ts.createSourceFile("workbench.ts", source, ts.ScriptTarget.Latest, true);
const names = ["handleLocusFileDrop", "handleLocusFileDragState", "nativeFileReferenceData", "nativeWorkbenchHit",
  "nativeWorkbenchDropDecisionAt", "updateNativeWorkbenchDropTarget", "currentNativeWorkbenchDropIntent",
  "clearNativeWorkbenchDropTarget"];
const code = ts.transpileModule(parsed.statements.filter(statement => ts.isFunctionDeclaration(statement)
  && !!statement.name && names.includes(statement.name.text)).map(statement => statement.getText(parsed)).join("\n"),
{ compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

const files = [{ path: "F:/Downloads/hero.png", name: "hero.png", isDir: false, source: "local" }];

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

function fixture(options: { filesFolder: boolean }) {
  const host = document.createElement("main");
  host.innerHTML = '<div class="workbench-editor-group" data-workbench-pane-id="chat"><div class="chat-composer"><textarea></textarea></div></div>';
  document.body.append(host);
  const hit = host.querySelector("textarea")!;
  const hitTest = vi.fn((): Element | null => hit);
  const commit = vi.fn(async () => {});
  const composerDropTarget = ref<unknown>(null);
  const unityActive = ref(false);
  const nativeWorkspaceFileDrop = {
    update: vi.fn(() => options.filesFolder),
    clear: vi.fn(),
    drop: vi.fn(async () => options.filesFolder),
  };
  const deps = {
    ownerDocument: { elementFromPoint: hitTest }, workbenchRootRef: ref(host), explorerRootRef: ref(null),
    locusFileWorkspaceDragActive: ref(false), locusFileWorkspaceDragCount: ref(0), locusFileWorkspaceTabEligible: ref(false),
    unityAssetWorkspaceDragActive: unityActive, unityAssetWorkspaceDragRefs: ref([]),
    pinDropProjectId: ref(null), externalDropTarget: ref(null), layoutDropIntent: ref(null),
    dropTargetKey: ref(null), editorDropIntent: ref(null), composerDropTarget,
    editorForPane: () => ({ editorId: "editor", resource: { kind: "session" } }),
    handleWorkbenchInternalTargetChange: (decision: { intent: unknown } | null) => {
      composerDropTarget.value = decision?.intent ?? null;
    },
    nativeDropScope: () => ({ projectId: "project", workspaceRef: { checkoutId: "checkout" }, workspaceRoot: "F:/Game" }),
    commitWorkbenchInternalDrop: commit, WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE: "reference",
    updateWorkspaceDragPointer: vi.fn(), clearWorkspaceDragPointer: vi.fn(), UNITY_WORKSPACE_DRAG_STATE_TTL_MS: 1200,
    notificationStore: { addNotice: vi.fn() }, normalizeAppError: (error: unknown) => error,
    nativeWorkspaceFileDrop,
  };
  const handlers = new Function(...Object.keys(deps), `let unityWorkspaceDragStateClearTimer = 0; ${code}
    return { state: handleLocusFileDragState, drop: handleLocusFileDrop };`)(...Object.values(deps));
  return { ...handlers, hit, hitTest, commit, composerDropTarget, unityActive, nativeWorkspaceFileDrop };
}

const over = { phase: "over", active: true, fileCount: 1, tabEligible: false, x: 40, y: 60 } as const;

describe("OS file drops in the workbench", () => {
  it("copies into the Files folder under the release point instead of adding a reference", async () => {
    const f = fixture({ filesFolder: true });
    f.state({ ...over, phase: "enter" });
    expect(f.nativeWorkspaceFileDrop.update).toHaveBeenCalledWith(f.hit);
    expect(f.composerDropTarget.value).toBeNull();
    f.state({ ...over, phase: "drop", active: false });
    await f.drop({ files, x: 40, y: 60 });
    expect(f.nativeWorkspaceFileDrop.drop).toHaveBeenCalledExactlyOnceWith(f.hit, ["F:/Downloads/hero.png"]);
    expect(f.commit).not.toHaveBeenCalled();
  });

  it("keeps attaching references where no Files folder accepts the drop", async () => {
    const f = fixture({ filesFolder: false });
    f.state(over);
    expect(f.composerDropTarget.value).toEqual({ kind: "composer", paneId: "chat", editorId: "editor" });
    await f.drop({ files, x: 40, y: 60 });
    expect(f.nativeWorkspaceFileDrop.drop).toHaveBeenCalledOnce();
    expect(f.commit).toHaveBeenCalledExactlyOnceWith("reference", expect.objectContaining({
      entries: [expect.objectContaining({ kind: "file", path: "F:/Downloads/hero.png" })],
    }), { kind: "composer", paneId: "chat", editorId: "editor" });
  });

  it("does not offer a copy while a Unity asset drag is active", () => {
    const f = fixture({ filesFolder: true });
    f.unityActive.value = true;
    f.state(over);
    expect(f.nativeWorkspaceFileDrop.update).not.toHaveBeenCalled();
    expect(f.nativeWorkspaceFileDrop.clear).toHaveBeenCalled();
  });

  it("clears the folder highlight when the drag leaves", () => {
    const f = fixture({ filesFolder: true });
    f.state(over);
    f.nativeWorkspaceFileDrop.clear.mockClear();
    f.state({ phase: "leave", active: false, fileCount: 0, tabEligible: false, x: 0, y: 0 });
    expect(f.nativeWorkspaceFileDrop.clear).toHaveBeenCalled();
  });
});
