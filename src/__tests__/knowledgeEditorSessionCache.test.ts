import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { KnowledgeEditorSessionCache } from "../components/knowledge/knowledgeEditorSessionCache";
import {
  KnowledgeEditorWorkspaceSessionStore,
  knowledgeDirectoryEditorSessionKey,
  knowledgeDocumentEditorSessionKey,
} from "../components/knowledge/knowledgeEditorWorkspaceSession";

describe("KnowledgeEditorSessionCache", () => {
  it("evicts the least recently used session at its capacity", () => {
    const cache = new KnowledgeEditorSessionCache<number>(2);
    cache.set("a", 1);
    cache.set("b", 2);
    expect(cache.get("a")).toBe(1);

    cache.set("c", 3);

    expect(cache.get("b")).toBeUndefined();
    expect(cache.get("a")).toBe(1);
    expect(cache.get("c")).toBe(3);
    expect(cache.size).toBe(2);
  });

  it("pins dirty sessions even when the soft capacity is exceeded", () => {
    const cache = new KnowledgeEditorSessionCache<{ dirty: boolean; value: string }>(
      2,
      (session) => !session.dirty,
    );
    cache.set("dirty-a", { dirty: true, value: "draft a" });
    cache.set("clean-b", { dirty: false, value: "saved b" });
    cache.set("clean-c", { dirty: false, value: "saved c" });

    expect(cache.get("dirty-a")?.value).toBe("draft a");
    expect(cache.get("clean-b")).toBeUndefined();
    expect(cache.get("clean-c")?.value).toBe("saved c");

    cache.set("dirty-d", { dirty: true, value: "draft d" });
    cache.set("dirty-e", { dirty: true, value: "draft e" });
    expect(cache.get("dirty-a")?.value).toBe("draft a");
    expect(cache.get("dirty-d")?.value).toBe("draft d");
    expect(cache.get("dirty-e")?.value).toBe("draft e");
    expect(cache.size).toBe(3);
  });

  it("retains CodeMirror history slots for at least twenty three-field documents", () => {
    const store = new KnowledgeEditorWorkspaceSessionStore();
    for (let documentIndex = 0; documentIndex < 20; documentIndex += 1) {
      for (const section of ["summary", "maintenanceRules", "body"] as const) {
        const key = `workspace:doc-${documentIndex}:${section}`;
        store.markdownEditors.set(key, {
          state: EditorState.create({ doc: `${documentIndex}:${section}` }),
          scrollTop: documentIndex,
          scrollLeft: 0,
        });
      }
    }

    expect(store.markdownEditors.size).toBe(60);
    expect(store.markdownEditors.get("workspace:doc-0:summary")?.state.doc.toString()).toBe("0:summary");
    expect(store.markdownEditors.get("workspace:doc-19:body")?.state.doc.toString()).toBe("19:body");
  });

  it("uses stable, entity-scoped workspace keys for Markdown editor state", () => {
    const workspace = { checkoutId: "checkout:primary", expectedGeneration: 7 };
    const renamedDocument = {
      type: "design",
      id: "stable-id",
      path: "renamed:folder/new-name.md",
    };
    const originalDocument = {
      ...renamedDocument,
      path: "folder/old-name.md",
    };
    const documentKey = knowledgeDocumentEditorSessionKey(
      workspace,
      originalDocument as Parameters<typeof knowledgeDocumentEditorSessionKey>[1],
    );
    expect(knowledgeDocumentEditorSessionKey(
      workspace,
      renamedDocument as Parameters<typeof knowledgeDocumentEditorSessionKey>[1],
    )).toBe(documentKey);

    const directoryKey = knowledgeDirectoryEditorSessionKey(
      workspace,
      {
        type: "design",
        path: "stable-id",
      } as Parameters<typeof knowledgeDirectoryEditorSessionKey>[1],
    );
    expect(directoryKey).not.toBe(documentKey);
    expect(knowledgeDocumentEditorSessionKey(
      { ...workspace, expectedGeneration: 8 },
      originalDocument as Parameters<typeof knowledgeDocumentEditorSessionKey>[1],
    )).not.toBe(documentKey);
  });
});
