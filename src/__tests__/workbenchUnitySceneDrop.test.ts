// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import ts from "typescript";
import { ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";

// Execute the production drop handlers and hit testing, including the native
// child-window entry point, without mounting unrelated workspace services.
const source = readFileSync("src/components/workbench/DevelopmentWorkbench.vue", "utf8")
  .split('<script setup lang="ts">')[1]!.split("</script>")[0]!;
const parsed = ts.createSourceFile("workbench.ts", source, ts.ScriptTarget.Latest, true);
const names = ["handleWorkspaceUnityAssetDrop", "handleNativeUnityAssetDrop",
  "nativeAssetReferenceData", "nativeWorkbenchDropDecisionAt", "updateNativeWorkbenchDropTarget",
  "currentNativeWorkbenchDropIntent", "clearNativeWorkbenchDropTarget", "handleUnityAssetWorkspaceDragState",
  "nativeWorkbenchHit"];
const code = ts.transpileModule(parsed.statements.filter(statement => ts.isFunctionDeclaration(statement)
  && !!statement.name && names.includes(statement.name.text)).map(statement => statement.getText(parsed)).join("\n"),
{ compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const refs = [{ kind: "sceneObject", path: "Assets/Scenes/World.unity/Actors/Player", name: "Player", source: "unity" }];

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

function fixture(scale = 1) {
  const host = document.createElement("main");
  host.innerHTML = '<div class="workbench-editor-group" data-workbench-pane-id="chat"><div class="chat-composer"><textarea></textarea></div></div>';
  document.body.append(host);
  const hitTest = vi.fn((): Element | null => host.querySelector("textarea"));
  const commit = vi.fn(async () => {});
  const composerDropTarget = ref<unknown>(null);
  const unityActive = ref(false);
  const unityRefs = ref([]);
  const deps = {
    ownerDocument: { elementFromPoint: hitTest }, ownerWindow: { devicePixelRatio: scale },
    workbenchRootRef: ref(host), explorerRootRef: ref(null),
    locusFileWorkspaceDragActive: ref(false), locusFileWorkspaceTabEligible: ref(false),
    unityAssetWorkspaceDragActive: unityActive, unityAssetWorkspaceDragRefs: unityRefs,
    pinDropProjectId: ref(null), externalDropTarget: ref(null), layoutDropIntent: ref(null),
    dropTargetKey: ref(null), editorDropIntent: ref(null), composerDropTarget,
    editorForPane: () => ({ editorId: "editor", resource: { kind: "session" } }),
    handleWorkbenchInternalTargetChange: (decision: { intent: unknown } | null) => {
      composerDropTarget.value = decision?.intent ?? null;
    },
    nativeDropScope: () => ({ projectId: "project", workspaceRef: { checkoutId: "checkout" }, workspaceRoot: "F:/Game" }),
    normalizeReferencePath: (path: string) => path.replaceAll("\\", "/"),
    commitWorkbenchInternalDrop: commit, WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE: "reference",
    clearWorkspaceDragPointer: vi.fn(), UNITY_WORKSPACE_DRAG_STATE_TTL_MS: 1200,
    notificationStore: { addNotice: vi.fn() }, normalizeAppError: (error: unknown) => error,
    nativeWorkspaceFileDrop: { update: vi.fn(() => false), clear: vi.fn(), drop: vi.fn(async () => false) },
  };
  const handlers = new Function(...Object.keys(deps), `let unityWorkspaceDragStateClearTimer = 0; ${code}
    return { drop: handleWorkspaceUnityAssetDrop, nativeDrop: handleNativeUnityAssetDrop, clear: handleUnityAssetWorkspaceDragState };`)(...Object.values(deps));
  return { ...handlers, commit, hitTest, composerDropTarget, unityActive, unityRefs };
}

describe("Unity scene object drops into workbench windows", () => {
  it("attaches at the release position without a preceding file drag event", async () => {
    const f = fixture();
    await f.drop({ refs, x: 120, y: 340 });
    expect(f.hitTest).toHaveBeenCalledExactlyOnceWith(120, 340);
    expect(f.commit).toHaveBeenCalledExactlyOnceWith("reference", expect.objectContaining({
      entries: [{ kind: "sceneObject", scenePath: "Assets/Scenes/World.unity", objectPath: "Actors/Player", name: "Player", typeLabel: undefined }],
    }), { kind: "composer", paneId: "chat", editorId: "editor" });
    expect(f.unityActive.value).toBe(false);
    expect(f.unityRefs.value).toEqual([]);
  });

  it("resolves the drop after Unity clears the drag preview before release delivery", async () => {
    const f = fixture();
    f.clear({ hasRefs: true, refs });
    f.clear({ hasRefs: false, refs: [] });
    await f.drop({ refs, x: 70, y: 190 });
    expect(f.commit).toHaveBeenCalledOnce();
  });

  it.each([1, 1.5, 2])("uses the embedded document coordinates at %sx display scale", async scale => {
    const f = fixture(scale);
    f.nativeDrop({ refs, x: 120 * scale, y: 340 * scale });
    await Promise.resolve();
    expect(f.hitTest).toHaveBeenCalledExactlyOnceWith(120, 340);
    expect(f.commit).toHaveBeenCalledOnce();
  });

  it("rejects a release outside the composer even if an earlier target was cached", async () => {
    const f = fixture();
    f.composerDropTarget.value = { kind: "composer", paneId: "old", editorId: "old" };
    f.hitTest.mockReturnValue(null);
    await f.drop({ refs, x: 800, y: 20 });
    expect(f.commit).not.toHaveBeenCalled();
    expect(f.composerDropTarget.value).toBeNull();
  });
});
