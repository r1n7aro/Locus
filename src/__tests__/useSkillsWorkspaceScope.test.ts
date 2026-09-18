import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { computed, effectScope, nextTick, ref } from "vue";

const knowledgeMocks = vi.hoisted(() => ({
  listSkills: vi.fn(),
}));

vi.mock("../services/knowledge", () => ({
  listSkills: knowledgeMocks.listSkills,
}));

import { invalidateSkills, useSkills } from "../composables/useSkills";
import { useCommandRegistry } from "../composables/useCommandRegistry";
import type { WorkspaceRef } from "../services/project";
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
  let scope: ReturnType<typeof effectScope>;
  beforeEach(() => {
    scope = effectScope();
    setActivePinia(createPinia());
    knowledgeMocks.listSkills.mockReset();
    invalidateSkills();
  });
  afterEach(() => scope.stop());

  it("keeps A and B caches isolated when A completes after B", async () => {
    const responseA = deferred<SkillManifest[]>();
    const responseB = deferred<SkillManifest[]>();
    knowledgeMocks.listSkills.mockImplementation((scope: { checkoutId: string }) => (
      scope.checkoutId === "checkout-a" ? responseA.promise : responseB.promise
    ));
    const { skillItems, loadSkills } = scope.run(() => useSkills())!;

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

  it("loads slash commands as soon as a cold-start workspace arrives, without a knowledge page", async () => {
    knowledgeMocks.listSkills.mockResolvedValue([skill("Startup")]);
    const consumer = scope.run(() => useSkills())!;
    const registry = useCommandRegistry(computed(() => consumer.skillItems.value), ref("unity"));
    await consumer.loadSkills();
    expect(knowledgeMocks.listSkills).not.toHaveBeenCalled();
    expect(consumer.skillsLoaded.value).toBe(false);
    bindCheckout("startup", 1);
    await nextTick();
    await consumer.loadSkills();
    expect(knowledgeMocks.listSkills).toHaveBeenCalledTimes(1);
    expect(registry.findExactAvailableCommand("/startup")?.commandType).toBe("skill");
  });

  it("keeps explicit editor scopes independent of focus and reloads changed incarnations", async () => {
    const left = ref<WorkspaceRef | null>({ checkoutId: "left", expectedGeneration: 1 });
    const right = ref<WorkspaceRef | null>({ checkoutId: "right", expectedGeneration: 2 });
    knowledgeMocks.listSkills.mockImplementation((workspace: WorkspaceRef) => Promise.resolve([
      skill(`${workspace.checkoutId}-${workspace.expectedGeneration}-${workspace.expectedMaterializationEpoch ?? 0}`),
    ]));
    const a = scope.run(() => useSkills(left))!;
    const b = scope.run(() => useSkills(right))!;
    await Promise.all([a.loadSkills(), b.loadSkills()]);
    bindCheckout("unrelated", 5);
    await nextTick();
    expect(knowledgeMocks.listSkills).toHaveBeenCalledTimes(2);
    expect(a.skillItems.value[0]?.name).toBe("left-1-0");
    expect(b.skillItems.value[0]?.name).toBe("right-2-0");
    left.value = { checkoutId: "left", expectedGeneration: 3, expectedMaterializationEpoch: 2 };
    await nextTick();
    await a.loadSkills();
    expect(a.skillItems.value[0]?.name).toBe("left-3-2");
    left.value = { ...left.value, expectedMaterializationEpoch: 3 };
    await nextTick();
    await a.loadSkills();
    expect(a.skillItems.value[0]?.name).toBe("left-3-3");
    expect(b.skillItems.value[0]?.name).toBe("right-2-0");
  });

  it("deduplicates consumers and ignores superseded results after invalidation", async () => {
    const workspace = { checkoutId: "shared", expectedGeneration: 1 };
    const slow = deferred<SkillManifest[]>();
    knowledgeMocks.listSkills.mockReturnValueOnce(slow.promise).mockResolvedValue([skill("New")]);
    const a = scope.run(() => useSkills(workspace))!;
    const b = scope.run(() => useSkills(workspace))!;
    const pending = a.loadSkills();
    expect(knowledgeMocks.listSkills).toHaveBeenCalledTimes(1);
    invalidateSkills(workspace);
    await nextTick();
    await b.loadSkills();
    slow.resolve([skill("Old")]);
    await pending;
    expect(knowledgeMocks.listSkills).toHaveBeenCalledTimes(2);
    expect(a.skillItems.value[0]?.name).toBe("New");
    expect(b.skillItems.value[0]?.name).toBe("New");
  });

  it("leaves failures retryable, including a failed forced refresh of a loaded cache", async () => {
    const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
    knowledgeMocks.listSkills.mockRejectedValueOnce(new Error("Unavailable"))
      .mockResolvedValueOnce([skill("Initial")])
      .mockRejectedValueOnce(new Error("Refresh failed"))
      .mockResolvedValue([skill("Recovered")]);
    const consumer = scope.run(() => useSkills({ checkoutId: "retry", expectedGeneration: 1 }))!;
    await consumer.loadSkills();
    expect(consumer.skillsLoaded.value).toBe(false);
    await consumer.loadSkills();
    expect(consumer.skillsLoaded.value).toBe(true);
    await consumer.loadSkills({ force: true });
    expect(consumer.skillsLoaded.value).toBe(false);
    expect(consumer.skillItems.value).toEqual([]);
    await consumer.loadSkills();
    expect(consumer.skillItems.value[0]?.name).toBe("Recovered");
    warning.mockRestore();
  });

  it("does not replace an explicit empty editor scope with the focused workspace", async () => {
    bindCheckout("focused", 1);
    const consumer = scope.run(() => useSkills(ref(null)))!;
    await consumer.loadSkills();
    await consumer.loadSkills({ workspaceRef: null });
    expect(knowledgeMocks.listSkills).not.toHaveBeenCalled();
    expect(consumer.skillItems.value).toEqual([]);
  });

  it("refreshes every active scope after an app plugin change", async () => {
    knowledgeMocks.listSkills.mockResolvedValue([skill("Before")]);
    const a = scope.run(() => useSkills({ checkoutId: "plugin-a", expectedGeneration: 1 }))!;
    const b = scope.run(() => useSkills({ checkoutId: "plugin-b", expectedGeneration: 1 }))!;
    await Promise.all([a.loadSkills(), b.loadSkills()]);
    knowledgeMocks.listSkills.mockResolvedValue([skill("After")]);
    invalidateSkills();
    await nextTick();
    await Promise.all([a.loadSkills(), b.loadSkills()]);
    expect(a.skillItems.value[0]?.name).toBe("After");
    expect(b.skillItems.value[0]?.name).toBe("After");
  });
});
