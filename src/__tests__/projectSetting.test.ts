// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import type { WorkspaceRef } from "../services/project";

vi.mock("../stores/workspaceContext", async () => {
  const { reactive } = await import("vue");
  const context = reactive({
    focusedWorkspaceRef: null as WorkspaceRef | null,
    focusedCheckout: null as object | null,
  });
  return { useWorkspaceContextStore: () => context };
});

import { installLocusRuntime } from "../services/locusRuntime";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { useProjectSetting } from "../composables/useProjectSetting";
import { readBool } from "../composables/useSettingsStore";

const KEY = "project.knowledgeEnabled";
const projectValues: Record<string, boolean> = {};
const calls: { command: string; args?: Record<string, unknown> }[] = [];
let failNextWrite: string | null = null;
let uninstall: (() => void) | null = null;

async function settle() {
  for (let i = 0; i < 4; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  await nextTick();
}

function focus(checkoutId: string | null) {
  const context = useWorkspaceContextStore() as unknown as {
    focusedWorkspaceRef: WorkspaceRef | null;
    focusedCheckout: object | null;
  };
  context.focusedWorkspaceRef = checkoutId ? { checkoutId, expectedGeneration: 1 } : null;
  context.focusedCheckout = checkoutId ? { checkoutId } : null;
}

beforeEach(() => {
  calls.length = 0;
  failNextWrite = null;
  for (const key of Object.keys(projectValues)) delete projectValues[key];
  projectValues["a"] = false;
  projectValues["b"] = true;
  uninstall = installLocusRuntime({
    kind: "tauri",
    async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      calls.push({ command, args });
      const checkoutId = (args?.workspaceRef as WorkspaceRef | null)?.checkoutId ?? "";
      if (command === "settings_get") return { [KEY]: projectValues[checkoutId] } as T;
      if (command === "settings_set") {
        if (failNextWrite) {
          const message = failNextWrite;
          failNextWrite = null;
          throw { code: "settings.persistence", message };
        }
        const [change] = args?.changes as { key: string; value: boolean }[];
        const old = projectValues[checkoutId];
        projectValues[checkoutId] = change.value;
        return [{ key: change.key, old, new: change.value }] as T;
      }
      return undefined as T;
    },
    async subscribe() {
      return () => {};
    },
  });
  focus(null);
});

afterEach(() => {
  uninstall?.();
  uninstall = null;
});

describe("project settings of the focused workspace", () => {
  it("stays unready without a focused project", async () => {
    const setting = useProjectSetting(KEY, readBool(true));
    await settle();
    expect(setting.ready.value).toBe(false);
    expect(calls).toEqual([]);
    await expect(setting.set(false)).rejects.toThrow();
  });

  it("reads and writes the focused project's value", async () => {
    focus("a");
    const setting = useProjectSetting(KEY, readBool(true));
    await settle();
    expect(calls[0]).toEqual({
      command: "settings_get",
      args: { keys: [KEY], workspaceRef: { checkoutId: "a", expectedGeneration: 1 } },
    });
    expect(setting.ready.value).toBe(true);
    expect(setting.value.value).toBe(false);

    await setting.set(true);
    expect(calls.at(-1)).toEqual({
      command: "settings_set",
      args: { changes: [{ key: KEY, value: true }], workspaceRef: { checkoutId: "a", expectedGeneration: 1 } },
    });
    expect(projectValues.a).toBe(true);
    expect(setting.value.value).toBe(true);
    expect(setting.busy.value).toBe(false);
  });

  it("rolls back and reports a failed write", async () => {
    focus("a");
    const setting = useProjectSetting(KEY, readBool(true));
    await settle();
    failNextWrite = "Disk full";
    await expect(setting.set(true)).rejects.toMatchObject({ message: "Disk full" });
    expect(setting.value.value).toBe(false);
    expect(setting.error.value).toBe("Disk full");
  });

  it("rereads when the focus moves or the checkout state changes", async () => {
    focus("a");
    const setting = useProjectSetting(KEY, readBool(true));
    await settle();
    expect(setting.value.value).toBe(false);

    focus("b");
    expect(setting.ready.value).toBe(false);
    await settle();
    expect(setting.ready.value).toBe(true);
    expect(setting.value.value).toBe(true);

    // Another window or the agent turned it off; the checkout descriptor is
    // replaced by the workspace-setup-changed update.
    projectValues.b = false;
    focus("b");
    await settle();
    expect(setting.value.value).toBe(false);
  });
});
