import { describe, expect, it, vi } from "vitest";
import { resolveWorkbenchFileTarget } from "../components/workbench/workbenchFileTarget";
import type { KnowledgeDocumentType } from "../types";
import type { ProjectKnowledgeDocument } from "../types/workbench";

const checkout = { checkoutId: "session-checkout", projectId: "game", root: "F:/Game" };

function document(overrides: Partial<ProjectKnowledgeDocument> = {}): ProjectKnowledgeDocument {
  return {
    id: "plan-document",
    type: "plan",
    path: "ECS动画系统Raw全链路与ACL分阶段实施计划.md",
    title: "ECS 动画实施计划",
    injectMode: "full",
    effectiveInjectMode: "full",
    readOnly: false,
    aiMaintained: false,
    effectiveAiMaintained: false,
    modifiedAt: 0,
    sourceCheckoutId: checkout.checkoutId,
    sourceRoot: checkout.root,
    availableCheckoutIds: [checkout.checkoutId],
    ...overrides,
  };
}

function catalog(items: ProjectKnowledgeDocument[]) {
  return { documents: () => items, refresh: vi.fn<() => Promise<void>>().mockResolvedValue(undefined) };
}

describe("Workbench file viewer selection", () => {
  it.each<KnowledgeDocumentType>(["design", "plan", "memory", "skill", "reference"])(
    "opens %s documents with the knowledge viewer",
    async (type) => {
      const doc = document({ type });
      const source = catalog([doc]);
      expect(await resolveWorkbenchFileTarget(`Locus/knowledge/${type}/${doc.path}`, checkout, source))
        .toEqual({ kind: "knowledge", projectId: checkout.projectId, documentId: doc.id });
      expect(source.refresh).not.toHaveBeenCalled();
    },
  );

  it.each([
    " ./Locus/knowledge/plan/systems/animation.md ",
    "F:\\Game\\Locus\\knowledge\\plan\\systems\\animation.md",
    "f:/game/LOCUS/KNOWLEDGE/PLAN/systems/ANIMATION.MD",
    "F:/Game/./Locus/knowledge/plan/systems/animation.md",
    "Locus/knowledge/plan/systems/draft/../animation.md",
  ])("resolves knowledge file paths: %s", async (path) => {
    const doc = document({ path: "systems/animation.md" });
    expect(await resolveWorkbenchFileTarget(path, checkout, catalog([doc])))
      .toEqual({ kind: "knowledge", projectId: checkout.projectId, documentId: doc.id });
  });

  it("accepts catalog paths with a document type prefix", async () => {
    const doc = document({ path: "plan/systems/animation.md" });
    expect(await resolveWorkbenchFileTarget("Locus/knowledge/plan/systems/animation.md", checkout, catalog([doc])))
      .toEqual({ kind: "knowledge", projectId: checkout.projectId, documentId: doc.id });
  });

  it("refreshes the catalog when a newly written document is opened", async () => {
    const doc = document();
    const items: ProjectKnowledgeDocument[] = [];
    const source = catalog(items);
    source.refresh.mockImplementation(async () => { items.push(doc); });
    expect(await resolveWorkbenchFileTarget(`Locus/knowledge/plan/${doc.path}`, checkout, source))
      .toEqual({ kind: "knowledge", projectId: checkout.projectId, documentId: doc.id });
    expect(source.refresh).toHaveBeenCalledOnce();
  });

  it("selects the session checkout's document when another checkout has the same path", async () => {
    const doc = document();
    const sibling = document({
      id: "sibling-document",
      sourceCheckoutId: "other-checkout",
      sourceRoot: "F:/Other",
      availableCheckoutIds: ["other-checkout"],
    });
    expect(await resolveWorkbenchFileTarget(`Locus/knowledge/plan/${doc.path}`, checkout, catalog([sibling, doc])))
      .toEqual({ kind: "knowledge", projectId: checkout.projectId, documentId: doc.id });
    const source = catalog([sibling]);
    await expect(resolveWorkbenchFileTarget(`Locus/knowledge/plan/${doc.path}`, checkout, source)).rejects.toThrow();
    expect(source.refresh).toHaveBeenCalledOnce();
  });

  it("uses shared catalog documents available in the session checkout", async () => {
    const doc = document({ sourceCheckoutId: "other-checkout", availableCheckoutIds: ["other-checkout", checkout.checkoutId] });
    expect(await resolveWorkbenchFileTarget(`Locus/knowledge/plan/${doc.path}`, checkout, catalog([doc])))
      .toEqual({ kind: "knowledge", projectId: checkout.projectId, documentId: doc.id });
  });

  it("reports missing knowledge documents instead of falling back to Markdown", async () => {
    await expect(resolveWorkbenchFileTarget("Locus/knowledge/plan/missing.md", checkout, catalog([]))).rejects.toThrow();
  });

  it.each([
    ["README.md", "README.md"],
    ["F:\\Game\\Assets\\Player.cs", "Assets/Player.cs"],
    ["docs/plan/overview.md", "docs/plan/overview.md"],
    ["Locus/knowledge/plan/settings.json", "Locus/knowledge/plan/settings.json"],
    ["Locus/knowledge/plan/../../../README.md", "README.md"],
    ["F:/Game-other/Locus/knowledge/plan/overview.md", "F:/Game-other/Locus/knowledge/plan/overview.md"],
  ])("keeps ordinary files in the file viewer: %s", async (filePath, path) => {
    const source = catalog([]);
    expect(await resolveWorkbenchFileTarget(filePath, checkout, source))
      .toEqual({ kind: "workspaceFile", projectId: checkout.projectId, path });
    expect(source.refresh).not.toHaveBeenCalled();
  });
});
