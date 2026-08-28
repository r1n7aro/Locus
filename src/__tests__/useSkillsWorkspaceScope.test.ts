import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const knowledgeMocks = vi.hoisted(() => ({
  listSkills: vi.fn(),
}));

vi.mock("../services/knowledge", () => ({
  listSkills: knowledgeMocks.listSkills,
}));

import { useSkills } from "../composables/useSkills";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type { SkillManifest } from "../types";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function skill(name: string): SkillManifest {
  return {
    name,
    description: name,
    argumentHint: "",
    dirName: name.toLowerCase(),
    source: "project",
    relPath: `${name}.md`,
    updatedAt: 1,
    skillEnabled: true,
    skillSurface: "both",
    skillDescription: null,
    commandTrigger: `/${name.toLowerCase()}`,
    tools: [],
    kind: "document",
    hasUnity: false,
    writable: true,
  };
}

function bindCheckout(checkoutId: string, generation: number) {
  const store = useWorkspaceContextStore();
  store.checkoutsById[checkoutId] = {
    checkoutId,
    projectId: "project-shared",
    root: `F:/work/${checkoutId}`,
    normalizedRoot: `f:/work/${checkoutId}`,
    lastOpenedAt: 1,
    runtime: {
      projectId: "project-shared",
      checkoutId,
      root: `F:/work/${checkoutId}`,
      workspaceGeneration: generation,
      leaseCount: 1,
      detectedServices: [],
    },
  };
  store.paneContexts["main\u0000main"] = {
    windowId: "main",
    paneId: "main",
    focusedCheckoutId: checkoutId,
    workspaceGeneration: generation,
    activeSessionId: null,
    intentEpoch: generation,
    revision: generation,
  };
}

describe("Skill manifests by checkout", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    knowledgeMocks.listSkills.mockReset();
  });

  it("keeps A and B caches isolated when A completes after B", async () => {
    const responseA = deferred<SkillManifest[]>();
    const responseB = deferred<SkillManifest[]>();
    knowledgeMocks.listSkills.mockImplementation((scope: { checkoutId: string }) => (
      scope.checkoutId === "checkout-a" ? responseA.promise : responseB.promise
    ));
    const { skillItems, loadSkills } = useSkills();

    bindCheckout("checkout-a", 10);
    const loadA = loadSkills({ force: true });
    bindCheckout("checkout-b", 20);
    const loadB = loadSkills({ force: true });

    responseB.resolve([skill("B")]);
    await loadB;
    expect(skillItems.value.map((item) => item.name)).toEqual(["B"]);

    responseA.resolve([skill("A")]);
    await loadA;
    expect(skillItems.value.map((item) => item.name)).toEqual(["B"]);

    bindCheckout("checkout-a", 10);
    expect(skillItems.value.map((item) => item.name)).toEqual(["A"]);
  });
});
