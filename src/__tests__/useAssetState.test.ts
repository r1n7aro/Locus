import { beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick, reactive } from "vue";
import { useAssetState } from "../composables/useAssetState";
import type { AssetPreviewPayload, SemanticTargetInspector, SemanticTreeNode } from "../types";

const assetServiceMocks = vi.hoisted(() => ({
  assetDbOverview: vi.fn(),
  assetDbScan: vi.fn(),
  searchWorkspaceAssets: vi.fn(),
  previewWorkspaceAsset: vi.fn(),
  previewWorkspaceAssetTarget: vi.fn(),
  getWatcherTuning: vi.fn(),
  setWatcherTuning: vi.fn(),
}));

const projectServiceMocks = vi.hoisted(() => ({
  listDirEntriesPage: vi.fn(),
}));

vi.mock("../services/asset", () => assetServiceMocks);
vi.mock("../services/project", () => projectServiceMocks);
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("../services/errors", () => ({
  normalizeAppError: (error: unknown) => {
    if (typeof error === "object" && error !== null && "message" in error) return error;
    return {
      code: "unknown",
      message: String(error),
      retryable: false,
      severity: "error",
    };
  },
}));
vi.mock("../composables/useHideMeta", () => ({
  isMetaFile: (name: string) => name.endsWith(".meta"),
}));
vi.mock("../composables/useSelectionLock", () => ({
  acquireSelectionLock: () => () => {},
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
  await nextTick();
}

function textPreview(snippet: string): AssetPreviewPayload {
  return {
    kind: "text",
    snippet,
    truncated: false,
    totalLines: 1,
  };
}

function treeNode(
  id: string,
  label: string,
  parentId: string | null = null,
): SemanticTreeNode {
  return {
    id,
    parentId,
    label,
    objectKind: "GameObject",
    changeKind: "unchanged",
    path: parentId ? `Root/${label}` : label,
    childIds: [],
    badgeCounts: {
      added: 0,
      removed: 0,
      modified: 0,
      componentsChanged: 0,
    },
    hasInspector: true,
  };
}

function structuredPreview(previewKey: string, tree: SemanticTreeNode[] = []): AssetPreviewPayload {
  return {
    kind: "structured",
    previewKey,
    layout: "assetInspector",
    tree,
    targets: [],
  };
}

function inspector(targetId: string): SemanticTargetInspector {
  return {
    targetId,
    title: targetId,
    path: `Assets/${targetId}.asset`,
    panels: [],
  };
}

function dirEntriesPage(
  entries: Array<{ name: string; relPath: string; isDir: boolean }>,
  totalCount = entries.length,
  nextOffset = entries.length,
  hasMore = false,
) {
  return {
    entries,
    totalCount,
    nextOffset,
    hasMore,
  };
}

describe("useAssetState preview flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    projectServiceMocks.listDirEntriesPage.mockResolvedValue(
      dirEntriesPage([], 0, 0, false),
    );
  });

  it("keeps the current preview visible until the latest asset request resolves", async () => {
    const firstRequest = deferred<AssetPreviewPayload>();
    const secondRequest = deferred<AssetPreviewPayload>();

    assetServiceMocks.previewWorkspaceAsset
      .mockImplementationOnce(() => firstRequest.promise)
      .mockImplementationOnce(() => secondRequest.promise);

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.previewPayload.value = textPreview("old preview");
    state.previewNode.value = {
      kind: "file",
      name: "Old.asset",
      path: "Assets/Old.asset",
      depth: 1,
    };

    const firstLoad = state.loadPreview({
      kind: "file",
      name: "First.asset",
      path: "Assets/First.asset",
      depth: 1,
    });
    await flushPromises();

    expect(state.previewPayload.value).toEqual(textPreview("old preview"));
    expect(state.previewNode.value?.path).toBe("Assets/Old.asset");
    expect(state.previewLoading.value).toBe(true);

    const secondLoad = state.loadPreview({
      kind: "file",
      name: "Second.asset",
      path: "Assets/Second.asset",
      depth: 1,
    });
    await flushPromises();

    expect(state.previewPayload.value).toEqual(textPreview("old preview"));
    expect(state.previewNode.value?.path).toBe("Assets/Old.asset");

    firstRequest.resolve(textPreview("first preview"));
    await firstLoad;
    await flushPromises();

    expect(state.previewPayload.value).toEqual(textPreview("old preview"));
    expect(state.previewNode.value?.path).toBe("Assets/Old.asset");
    expect(state.previewLoading.value).toBe(true);

    secondRequest.resolve(textPreview("second preview"));
    await secondLoad;
    await flushPromises();

    expect(state.previewPayload.value).toEqual(textPreview("second preview"));
    expect(state.previewNode.value?.path).toBe("Assets/Second.asset");
    expect(state.previewLoading.value).toBe(false);
  });

  it("drops stale target responses after a new preview session starts", async () => {
    const targetRequest = deferred<SemanticTargetInspector>();

    assetServiceMocks.previewWorkspaceAsset.mockResolvedValue(structuredPreview("new-key"));
    assetServiceMocks.previewWorkspaceAssetTarget.mockImplementationOnce(() => targetRequest.promise);

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.previewPayload.value = structuredPreview("old-key");
    state.previewNode.value = {
      kind: "file",
      name: "Old.prefab",
      path: "Assets/Old.prefab",
      depth: 1,
    };

    const pendingTarget = state.loadTarget("old-key", "child-a");
    await flushPromises();
    expect(state.targetLoading.value).toBe(true);

    await state.loadPreview({
      kind: "file",
      name: "New.prefab",
      path: "Assets/New.prefab",
      depth: 1,
    });
    await flushPromises();

    targetRequest.resolve(inspector("child-a"));
    const targetResult = await pendingTarget;
    await flushPromises();

    expect(targetResult).toBeNull();
    expect(state.previewNode.value?.path).toBe("Assets/New.prefab");
    expect(state.activeTargetId.value).toBeNull();
    expect(state.targetCache.value.size).toBe(0);
    expect(state.targetLoading.value).toBe(false);
  });

  it("selects and loads the prefab root target after a structured prefab preview resolves", async () => {
    const root = treeNode("go:root", "Player");
    const child = treeNode("go:child", "Camera", root.id);

    assetServiceMocks.previewWorkspaceAsset.mockResolvedValue(
      structuredPreview("prefab-key", [root, child]),
    );
    assetServiceMocks.previewWorkspaceAssetTarget.mockResolvedValue(inspector(root.id));

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));

    await state.loadPreview({
      kind: "file",
      name: "Player.prefab",
      path: "Assets/Player.prefab",
      depth: 1,
    });
    await flushPromises();

    expect(state.activeTargetId.value).toBe(root.id);
    expect(state.targetCache.value.get(root.id)).toEqual(inspector(root.id));
    expect(assetServiceMocks.previewWorkspaceAssetTarget).toHaveBeenCalledWith(
      "prefab-key",
      root.id,
    );
  });

  it("keeps the latest selected target active when the prefab root load resolves later", async () => {
    const rootRequest = deferred<SemanticTargetInspector>();
    const childRequest = deferred<SemanticTargetInspector>();
    const root = treeNode("go:root", "Player");
    const child = treeNode("go:child", "Camera", root.id);

    assetServiceMocks.previewWorkspaceAsset.mockResolvedValue(
      structuredPreview("prefab-key", [root, child]),
    );
    assetServiceMocks.previewWorkspaceAssetTarget
      .mockImplementationOnce(() => rootRequest.promise)
      .mockImplementationOnce(() => childRequest.promise);

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));

    const previewLoad = state.loadPreview({
      kind: "file",
      name: "Player.prefab",
      path: "Assets/Player.prefab",
      depth: 1,
    });
    await flushPromises();

    expect(state.activeTargetId.value).toBe(root.id);

    const childLoad = state.loadTarget("prefab-key", child.id);
    await flushPromises();

    expect(state.activeTargetId.value).toBe(child.id);

    rootRequest.resolve(inspector(root.id));
    await previewLoad;
    await flushPromises();

    expect(state.activeTargetId.value).toBe(child.id);

    childRequest.resolve(inspector(child.id));
    await childLoad;
    await flushPromises();

    expect(state.activeTargetId.value).toBe(child.id);
    expect(state.targetCache.value.has(root.id)).toBe(true);
    expect(state.targetCache.value.has(child.id)).toBe(true);
    expect(state.targetLoading.value).toBe(false);
  });

  it("loads folder pages incrementally for large directories", async () => {
    projectServiceMocks.listDirEntriesPage
      .mockResolvedValueOnce(
        dirEntriesPage(
          [
            { name: "A.prefab", relPath: "Assets/A.prefab", isDir: false },
            { name: "B.prefab", relPath: "Assets/B.prefab", isDir: false },
          ],
          4,
          2,
          true,
        ),
      )
      .mockResolvedValueOnce(
        dirEntriesPage(
          [
            { name: "C.prefab", relPath: "Assets/C.prefab", isDir: false },
            { name: "D.prefab", relPath: "Assets/D.prefab", isDir: false },
          ],
          4,
          4,
          false,
        ),
      );

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.initRoots();

    await state.togglePath("Assets");

    const assetsRoot = state.explorerTree.value[0];
    expect(assetsRoot?.kind).toBe("folder");
    expect(projectServiceMocks.listDirEntriesPage).toHaveBeenCalledWith(
      "Assets",
      0,
      200,
      true,
    );
    expect(assetsRoot?.kind === "folder" && assetsRoot.children).toHaveLength(2);
    expect(assetsRoot?.kind === "folder" && assetsRoot.hasMore).toBe(true);

    await state.loadMoreFolder("Assets");

    expect(projectServiceMocks.listDirEntriesPage).toHaveBeenCalledWith(
      "Assets",
      2,
      200,
      true,
    );
    expect(assetsRoot?.kind === "folder" && assetsRoot.children).toHaveLength(4);
    expect(assetsRoot?.kind === "folder" && assetsRoot.hasMore).toBe(false);
  });

  it("prefetches one level of child folders when expanding a directory", async () => {
    projectServiceMocks.listDirEntriesPage.mockImplementation(
      async (subPath: string, offset = 0, limit = 200) => {
        if (subPath === "Assets" && offset === 0 && limit === 200) {
          return dirEntriesPage([
            { name: "Art", relPath: "Assets/Art", isDir: true },
            { name: "Docs", relPath: "Assets/Docs", isDir: true },
            { name: "Readme.md", relPath: "Assets/Readme.md", isDir: false },
          ]);
        }
        if (subPath === "Assets/Art" && offset === 0 && limit === 1) {
          return dirEntriesPage([
            { name: "Textures", relPath: "Assets/Art/Textures", isDir: true },
          ]);
        }
        if (subPath === "Assets/Docs" && offset === 0 && limit === 1) {
          return dirEntriesPage([
            { name: "Guide.txt", relPath: "Assets/Docs/Guide.txt", isDir: false },
          ]);
        }
        throw new Error(`Unexpected listDirEntriesPage call: ${subPath} ${offset} ${limit}`);
      },
    );

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.initRoots();

    await state.togglePath("Assets");
    await flushPromises();

    const assetsRoot = state.explorerTree.value[0];
    expect(assetsRoot?.kind).toBe("folder");

    const artFolder = assetsRoot?.kind === "folder"
      ? assetsRoot.children.find((child) => child.path === "Assets/Art")
      : null;
    const docsFolder = assetsRoot?.kind === "folder"
      ? assetsRoot.children.find((child) => child.path === "Assets/Docs")
      : null;

    expect(artFolder?.kind).toBe("folder");
    expect(docsFolder?.kind).toBe("folder");
    expect(artFolder?.kind === "folder" && artFolder.loaded).toBe(false);
    expect(docsFolder?.kind === "folder" && docsFolder.loaded).toBe(false);
    expect(artFolder?.kind === "folder" && artFolder.hasChildFoldersKnown).toBe(true);
    expect(artFolder?.kind === "folder" && artFolder.hasChildFolders).toBe(true);
    expect(docsFolder?.kind === "folder" && docsFolder.hasChildFoldersKnown).toBe(true);
    expect(docsFolder?.kind === "folder" && docsFolder.hasChildFolders).toBe(false);

    expect(projectServiceMocks.listDirEntriesPage).toHaveBeenCalledWith(
      "Assets/Art",
      0,
      1,
      true,
    );
    expect(projectServiceMocks.listDirEntriesPage).toHaveBeenCalledWith(
      "Assets/Docs",
      0,
      1,
      true,
    );
  });

  it("exposes the selected folder entries for the current-directory pane", async () => {
    projectServiceMocks.listDirEntriesPage.mockResolvedValueOnce(
      dirEntriesPage([
        { name: "Scripts", relPath: "Assets/Scripts", isDir: true },
        { name: "Player.prefab", relPath: "Assets/Player.prefab", isDir: false },
      ]),
    );

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.initRoots();

    await state.selectFolder("Assets", { preservePreview: true, revealInTree: "none" });

    expect(state.selectedFolderPath.value).toBe("Assets");
    expect(state.currentFolderLabel.value).toBe("Assets");
    expect(state.visibleDirectoryEntries.value.map((entry) => entry.path)).toEqual([
      "Assets/Scripts",
      "Assets/Player.prefab",
    ]);
  });

  it("filters the current folder locally without calling global asset search", async () => {
    projectServiceMocks.listDirEntriesPage.mockResolvedValueOnce(
      dirEntriesPage([
        { name: "HUD.prefab", relPath: "Assets/HUD.prefab", isDir: false },
        { name: "MainScene.unity", relPath: "Assets/MainScene.unity", isDir: false },
      ]),
    );

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.initRoots();
    await state.selectFolder("Assets", { preservePreview: true, revealInTree: "none" });

    state.runFilenameSearch("hud");

    expect(assetServiceMocks.searchWorkspaceAssets).not.toHaveBeenCalled();
    expect(state.visibleDirectoryEntries.value.map((entry) => entry.name)).toEqual([
      "HUD.prefab",
    ]);
  });

  it("selects folders without expanding them by default", async () => {
    projectServiceMocks.listDirEntriesPage.mockResolvedValueOnce(
      dirEntriesPage([
        { name: "Art", relPath: "Assets/Art", isDir: true },
      ]),
    );

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.initRoots();

    await state.selectFolder("Assets", { preservePreview: true, revealInTree: "none" });

    expect(state.selectedFolderPath.value).toBe("Assets");
    expect(state.isPathExpanded("Assets")).toBe(false);
    expect(state.visibleDirectoryEntries.value.map((entry) => entry.path)).toEqual([
      "Assets/Art",
    ]);
  });

  it("clears expanded state after loading a leaf folder", async () => {
    projectServiceMocks.listDirEntriesPage.mockResolvedValueOnce(
      dirEntriesPage([
        {
          name: "AudioManager.asset",
          relPath: "ProjectSettings/AudioManager.asset",
          isDir: false,
        },
      ]),
    );

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.initRoots();

    await state.togglePath("ProjectSettings");

    const projectSettingsRoot = state.explorerTree.value[2];
    expect(state.isPathExpanded("ProjectSettings")).toBe(false);
    expect(projectServiceMocks.listDirEntriesPage).toHaveBeenCalledWith(
      "ProjectSettings",
      0,
      200,
      true,
    );
    expect(projectSettingsRoot?.kind === "folder" && projectSettingsRoot.loaded).toBe(true);
    expect(
      projectSettingsRoot?.kind === "folder" && projectSettingsRoot.hasChildFolders,
    ).toBe(false);
  });

  it("probes folder branches before showing toggle affordances in the folder-only tree", async () => {
    projectServiceMocks.listDirEntriesPage.mockImplementation(
      async (subPath: string, offset = 0, limit = 200) => {
        if (subPath === "ProjectSettings" && offset === 0 && limit === 1) {
          return dirEntriesPage([
            {
              name: "ProjectVersion.txt",
              relPath: "ProjectSettings/ProjectVersion.txt",
              isDir: false,
            },
          ]);
        }
        throw new Error(`Unexpected listDirEntriesPage call: ${subPath} ${offset} ${limit}`);
      },
    );

    const state = useAssetState(reactive({ workingDir: "F:/repo" }));
    state.initRoots();

    await state.probeFolderPath("ProjectSettings");

    const projectSettingsRoot = state.explorerTree.value[2];
    expect(projectServiceMocks.listDirEntriesPage).toHaveBeenCalledWith(
      "ProjectSettings",
      0,
      1,
      true,
    );
    expect(projectSettingsRoot?.kind === "folder" && projectSettingsRoot.loaded).toBe(false);
    expect(
      projectSettingsRoot?.kind === "folder" && projectSettingsRoot.hasChildFoldersKnown,
    ).toBe(true);
    expect(
      projectSettingsRoot?.kind === "folder" && projectSettingsRoot.hasChildFolders,
    ).toBe(false);
  });
});
