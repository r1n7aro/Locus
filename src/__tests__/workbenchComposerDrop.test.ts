import { describe, expect, it } from "vitest";
import {
  workbenchComposerFileAttachment,
  workbenchComposerEditorAttachment,
  workbenchComposerTreeFileAttachment,
} from "../components/workbench/workbenchComposerDrop";
import { createWorkbenchEditorInput } from "../stores/workbench";
import type { WorkbenchResourceRef } from "../types/workbench";

describe("editor tab composer attachments", () => {
  const context = { workspaceRoot: "F:/Game", targetWorkspaceRoot: "f:\\Game\\" };
  const editor = (resource: WorkbenchResourceRef, sourcePath?: string) => (
    createWorkbenchEditorInput(resource, "Source tab", { sourcePath })
  );

  it("attaches the knowledge document path instead of its tab identity", () => {
    const source = editor({ kind: "knowledge", projectId: "p", documentId: "doc-1" });
    expect(workbenchComposerEditorAttachment(source, {
      ...context,
      knowledgeDocument: { type: "design", path: "combat\\animation.md", sourceRoot: "F:/Game" },
    })).toEqual({
      assetRef: { kind: "knowledge", path: "design/combat/animation.md", name: "Source tab", source: "manual" },
    });
    expect(workbenchComposerEditorAttachment(source, context)).toBeNull();
  });

  it.each([
    { kind: "workspaceFile", projectId: "p", path: "Assets/Scripts/Player.cs" },
    { kind: "asset", projectId: "p", path: "Assets/Scripts/Player.cs" },
  ] satisfies WorkbenchResourceRef[])("attaches $kind through the existing asset reference type", (resource) => {
    expect(workbenchComposerEditorAttachment(editor(resource), context)).toMatchObject({
      assetRef: { kind: "asset", path: "Assets/Scripts/Player.cs", name: "Source tab" },
    });
  });

  it("preserves the scene and object identity", () => {
    expect(workbenchComposerEditorAttachment(editor({
      kind: "sceneObject", projectId: "p", scenePath: "Assets/Main.unity", objectPath: "Root/Player",
    }), context)).toMatchObject({
      assetRef: { kind: "sceneObject", path: "Assets/Main.unity/Root/Player" },
    });
  });

  it("resolves workspace files against the source checkout", () => {
    expect(workbenchComposerEditorAttachment(editor({
      kind: "workspaceFile", projectId: "p", path: "docs/notes.md",
    }), context)).toMatchObject({ localFile: { path: "F:/Game/docs/notes.md", isDir: false } });
  });

  it.each(["localFile", "localDirectory"] as const)("uses the resolved nested %s path", (kind) => {
    expect(workbenchComposerEditorAttachment(editor({
      kind, projectId: "p", nodeId: "mount", relativePath: "nested/item",
    }, "E:\\Reference\\nested\\item"), context)).toMatchObject({
      localFile: { path: "E:/Reference/nested/item", isDir: kind === "localDirectory" },
    });
  });

  it.each([
    { kind: "newSession", projectId: "p" },
    { kind: "session", projectId: "p", sessionId: "s" },
    { kind: "section", projectId: "p", section: "knowledge" },
    { kind: "view", projectId: "p", viewId: "v" },
    { kind: "folder", projectId: "p", nodeId: "virtual" },
  ] satisfies WorkbenchResourceRef[])("does not fabricate attachments for $kind", (resource) => {
    expect(workbenchComposerEditorAttachment(editor(resource), context)).toBeNull();
  });

  it("rejects unavailable resources and relative paths without a source workspace", () => {
    const source = editor({ kind: "workspaceFile", projectId: "p", path: "docs/notes.md" });
    expect(workbenchComposerEditorAttachment({ ...source, availability: "unavailable" }, context)).toBeNull();
    expect(workbenchComposerEditorAttachment(source, { ...context, workspaceRoot: "" })).toBeNull();
  });

  it("keeps files and knowledge from another checkout attached to their original paths", () => {
    const crossWorkspace = { ...context, targetWorkspaceRoot: "F:/Other" };
    expect(workbenchComposerEditorAttachment(editor({
      kind: "asset", projectId: "p", path: "Assets/Player.prefab",
    }), crossWorkspace)).toMatchObject({
      localFile: { path: "F:/Game/Assets/Player.prefab", isDir: false },
    });
    expect(workbenchComposerEditorAttachment(editor({
      kind: "knowledge", projectId: "p", documentId: "doc-1",
    }), {
      ...crossWorkspace,
      knowledgeDocument: { type: "design", path: "combat.md", sourceRoot: "F:/Game" },
    })).toMatchObject({
      localFile: { path: "F:/Game/Locus/knowledge/design/combat.md", isDir: false },
    });
    expect(workbenchComposerEditorAttachment(editor({
      kind: "sceneObject", projectId: "p", scenePath: "Assets/Main.unity", objectPath: "Player",
    }), crossWorkspace)).toBeNull();
  });
});

describe("workbench composer file drops", () => {
  it("keeps Unity project files as asset references", () => {
    expect(workbenchComposerFileAttachment({
      absolutePath: "F:\\Game\\Assets\\Prefabs\\Player.prefab",
      workspaceRoot: "F:\\Game",
      relativePath: "Prefabs/Player.prefab",
      name: "Player.prefab",
    })).toEqual({
      assetRef: {
        path: "Assets/Prefabs/Player.prefab",
        kind: "asset",
        name: "Player.prefab",
        typeLabel: undefined,
        source: "manual",
      },
    });
  });

  it("maps project knowledge files to knowledge references", () => {
    expect(workbenchComposerFileAttachment({
      absolutePath: "F:\\Game\\Locus\\knowledge\\design\\combat.md",
      workspaceRoot: "F:\\Game",
      knowledgeSource: true,
      name: "combat.md",
    })).toEqual({
      assetRef: {
        path: "design/combat.md",
        kind: "knowledge",
        name: "combat.md",
        typeLabel: undefined,
        source: "manual",
      },
    });
  });

  it("preserves files outside the project as local file attachments", () => {
    expect(workbenchComposerFileAttachment({
      absolutePath: "E:\\References\\brief.pdf",
      workspaceRoot: "F:\\Game",
      name: "brief.pdf",
      typeLabel: "PDF",
    })).toEqual({
      localFile: {
        path: "E:/References/brief.pdf",
        isDir: false,
        name: "brief.pdf",
        typeLabel: "PDF",
        source: "local",
      },
    });
  });

  it.each([
    "F:/Game/Locus/knowledge/reference/Unity Manual",
    "F:/Game/Locus/knowledge/design/notes.md",
    "F:/Game/Assets/Prefabs",
  ])("preserves an explicit directory reference for %s", (path) => {
    expect(workbenchComposerFileAttachment({
      absolutePath: path,
      workspaceRoot: "F:/Game",
      isDir: true,
    })).toMatchObject({ localFile: { path, isDir: true } });
  });
});

describe("workspace tree composer drops", () => {
  const explorerNode = {
    sourcePath: "F:\\Game\\Locus\\knowledge\\reference\\Unity Manual",
    sourceKind: "knowledge",
  };

  it("attaches a mounted knowledge folder with its actual directory path", () => {
    expect(workbenchComposerTreeFileAttachment({
      kind: "folder",
      explorerNode,
      name: "Unity Manual",
    }, "F:/Game")).toEqual({
      localFile: {
        path: "F:/Game/Locus/knowledge/reference/Unity Manual",
        isDir: true,
        name: "Unity Manual",
        typeLabel: undefined,
        source: "knowledge",
      },
    });
  });

  it("attaches a nested mounted folder using the child path, not the mount root", () => {
    expect(workbenchComposerTreeFileAttachment({
      kind: "mountedFolder",
      explorerNode,
      mountEntry: {
        absolutePath: `${explorerNode.sourcePath}\\渲染\\URP\\`,
        relativePath: "渲染/URP",
        name: "URP",
        isDir: true,
      },
    }, "F:/Game")).toMatchObject({
      localFile: {
        path: "F:/Game/Locus/knowledge/reference/Unity Manual/渲染/URP",
        isDir: true,
        name: "URP",
        source: "knowledge",
      },
    });
  });

  it("keeps a mounted knowledge document as a knowledge reference", () => {
    expect(workbenchComposerTreeFileAttachment({
      kind: "mountedFile",
      explorerNode,
      mountEntry: {
        absolutePath: `${explorerNode.sourcePath}\\rendering.md`,
        relativePath: "rendering.md",
        name: "rendering.md",
        isDir: false,
      },
    }, "F:/Game")).toMatchObject({
      assetRef: {
        path: "reference/Unity Manual/rendering.md",
        kind: "knowledge",
        name: "rendering.md",
      },
    });
  });

  it("preserves a mounted folder's source path when dropping into another checkout", () => {
    expect(workbenchComposerTreeFileAttachment({
      kind: "folder",
      explorerNode,
    }, "F:/OtherCheckout")).toMatchObject({
      localFile: {
        path: "F:/Game/Locus/knowledge/reference/Unity Manual",
        isDir: true,
      },
    });
  });

  it("does not turn virtual workspace folders or system entries into file references", () => {
    expect(workbenchComposerTreeFileAttachment({
      kind: "folder",
      explorerNode: { sourcePath: null },
    }, "F:/Game")).toBeNull();
    expect(workbenchComposerTreeFileAttachment({
      kind: "knowledgeRoot",
      explorerNode,
    }, "F:/Game")).toBeNull();
  });
});
