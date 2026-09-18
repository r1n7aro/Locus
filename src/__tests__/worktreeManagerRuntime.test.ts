// @vitest-environment jsdom
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorktreeManager from "../components/workbench/WorktreeManager.vue";
import type { ManagedWorktree } from "../services/worktrees";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(), openAndFocus: vi.fn(), openWorkspace: vi.fn(), confirm: vi.fn(), open: vi.fn(), close: vi.fn(),
}));
vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));
vi.mock("@tauri-apps/api/core", () => ({ Channel: class { constructor(public onmessage: (event: unknown) => void) {} } }));
vi.mock("../services/project", () => ({ openWorkspace: mocks.openWorkspace }));
vi.mock("../stores/workspaceContext", () => ({
  useWorkspaceContextStore: () => ({ openAndFocus: mocks.openAndFocus }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: mocks.confirm, open: mocks.open }));

const sourceRoot = "F:/Game/source";
const originalShowModal = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
let app: App | undefined;
let root: HTMLDivElement;
let records: ManagedWorktree[];
const created = record({ checkoutId: "new-checkout", root: "F:/Game/new", branch: "refs/heads/codex/new" });

function record(overrides: Partial<ManagedWorktree> = {}): ManagedWorktree {
  return {
    checkoutId: "checkout-a", projectId: "project", root: "F:/Game/a", repoRoot: "F:/Game/a",
    projectRelativePath: "", branch: "refs/heads/codex/a", headOid: "abc1234567890",
    materializationEpoch: 7, managed: true, lifecycle: "active", dirty: false,
    poolSlot: false, assignmentId: null, editorVersion: "6000.5.8f1", lastError: null,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail; });
  return { promise, resolve, reject };
}

async function settle() {
  for (let index = 0; index < 8; index++) await Promise.resolve();
  await nextTick();
}

async function mount() {
  app = createApp(WorktreeManager, { sourceRoot, onClose: mocks.close });
  app.mount(root);
  await settle();
}

function button(key: string, scope: ParentNode = root): HTMLButtonElement {
  const found = Array.from(scope.querySelectorAll<HTMLButtonElement>("button"))
    .find((item) => item.textContent?.trim() === key);
  if (!found) throw new Error(`Missing button ${key}`);
  return found;
}

function field(key: string): HTMLInputElement {
  const label = Array.from(root.querySelectorAll("label"))
    .find((item) => item.textContent?.trim().startsWith(key));
  const found = label?.querySelector<HTMLInputElement>("input");
  if (!found) throw new Error(`Missing labeled input ${key}`);
  return found;
}

async function fill(key: string, value: string) {
  const input = field(key);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
}

async function beginCreate(pool = false) {
  button(pool ? "worktrees.acquire" : "worktrees.create").click();
  await settle();
  await fill("worktrees.branch", "codex/new");
  await fill(pool ? "worktrees.poolRoot" : "worktrees.destination", " F:/Game/new ");
}

function submit() {
  root.querySelector<HTMLFormElement>("form")!.requestSubmit();
}

beforeEach(() => {
  vi.resetAllMocks();
  records = [record()];
  mocks.openAndFocus.mockResolvedValue(null);
  mocks.openWorkspace.mockResolvedValue(null);
  mocks.confirm.mockResolvedValue(true);
  mocks.open.mockResolvedValue(null);
  mocks.invoke.mockImplementation(async (command: string, args: { request?: { startRef?: string } }) => {
    if (command === "list_managed_worktrees") return records;
    if (command === "get_worktree_pool_usage") return { items: records.map(worktree => ({ worktree, sizeBytes: 1024 })), totalBytes: records.length * 1024, availableProjects: 0 };
    if (command === "plan_worktree_creation") return { startOid: args.request?.startRef, requiresNewProject: false, atCapacity: false, budget: null };
    if (command === "create_worktree" || command === "import_worktree") return { ...created };
    if (command === "acquire_unity_project_slot") {
      return { worktree: { ...created }, reused: false, preservedLibrary: false };
    }
    if (command === "release_unity_project_slot") return record({ lifecycle: "available" });
    if (command === "remove_managed_worktree") return;
    throw new Error(`Unexpected command ${command}`);
  });
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value(this: HTMLDialogElement) { this.setAttribute("open", ""); },
  });
  root = document.createElement("div");
  document.body.append(root);
});

afterEach(() => {
  app?.unmount(); app = undefined; root?.remove(); vi.restoreAllMocks();
  if (originalShowModal) Object.defineProperty(HTMLDialogElement.prototype, "showModal", originalShowModal);
  else Reflect.deleteProperty(HTMLDialogElement.prototype, "showModal");
});

describe("worktree manager interactions", () => {
  it("shows budget progress and releases the form when a preview is canceled", async () => {
    await mount(); await beginCreate();
    const estimate = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(estimate.promise);
    submit(); await settle();
    const args = [...mocks.invoke.mock.calls].reverse().find(([command]) => command === "plan_worktree_creation")![1];
    args.onProgress.onmessage({ phase: "cache", files: 123, totalFiles: null, bytes: 1024 }); await settle();
    expect(root.querySelector('[role="status"]')?.textContent).toContain("worktrees.progress.cache");
    expect(button("common.close").disabled).toBe(false);
    button("common.cancel", root.querySelector(".worktree-progress")!).click(); await settle();
    expect(field("worktrees.branch").value).toBe("codex/new");
    expect(field("worktrees.branch").disabled).toBe(false);
    estimate.resolve({ startOid: "late", requiresNewProject: false, atCapacity: false, budget: null }); await settle();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "create_worktree")).toBe(false);
  });
  it("shows a disk budget before creating a new local project and supports cancellation", async () => {
    await mount(); await beginCreate();
    mocks.invoke.mockResolvedValueOnce({ startOid: "budget-head", requiresNewProject: true, atCapacity: false, budget: {
      checkoutBytes: 1024, referenceCacheBytes: 0, estimatedBytes: 1024, cacheKnown: false, freeBytes: 102400, directory: "F:/Game/new",
    } });
    submit(); await settle();
    expect(mocks.invoke.mock.calls.some(([command])=>command === "create_worktree")).toBe(false);
    expect(root.querySelector(".worktree-budget")?.textContent).toContain("worktrees.budget.cacheUnknown");
    button("common.cancel", root.querySelector(".worktree-budget")!).click(); await settle();
    expect(root.querySelector(".worktree-budget")).toBeNull();
    expect(field("worktrees.branch").value).toBe("codex/new");
  });
  it("creates only after confirming the reviewed disk estimate", async () => {
    await mount(); await beginCreate(true);
    mocks.invoke.mockResolvedValueOnce({ startOid: "budget-head", requiresNewProject: true, atCapacity: false, budget: {
      checkoutBytes: 1024, referenceCacheBytes: 4096, estimatedBytes: 5120, cacheKnown: true, freeBytes: 102400, directory: "F:/Game/new",
    } });
    submit(); await settle(); button("worktrees.budget.confirm").click(); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("acquire_unity_project_slot", { allowNewProject: true, request: {
      sourceRoot, poolRoot: "F:/Game/new", commit: "budget-head", branch: "codex/new", maxSlots: 4,
    } });
  });
  it("registers the source without changing focus and blocks actions during registration", async () => {
    const opening = deferred<null>();
    mocks.openWorkspace.mockReturnValueOnce(opening.promise);
    await mount();
    expect(mocks.openWorkspace).toHaveBeenCalledWith(sourceRoot);
    expect(mocks.openAndFocus).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(Array.from(root.querySelectorAll<HTMLButtonElement>("button")).every((item) => item.disabled)).toBe(true);
    expect(root.querySelector("dialog")?.getAttribute("aria-labelledby")).toBe("worktree-dialog-title");
    opening.resolve(null);
    await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("list_managed_worktrees", { sourceRoot });
  });

  it("copies dirty work from HEAD and focuses only the successfully created checkout", async () => {
    await mount();
    await beginCreate();
    await fill("worktrees.start", "feature/other");
    const checkbox = root.querySelector<HTMLButtonElement>('[role="checkbox"]')!;
    checkbox.click();
    await settle();
    expect(checkbox.getAttribute("aria-checked")).toBe("true");
    expect(checkbox.getAttribute("aria-label")).toBe("worktrees.includeDirty");
    expect(field("worktrees.start").value).toBe("HEAD");
    expect(field("worktrees.start").disabled).toBe(true);
    mocks.openAndFocus.mockClear();
    submit();
    await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("create_worktree", { request: {
      sourceRoot, destination: "F:/Game/new", branch: "codex/new", startRef: "HEAD", includeDirty: true,
    } });
    expect(mocks.openAndFocus).toHaveBeenCalledExactlyOnceWith(created.root);
    expect(root.querySelector("form")).toBeNull();
  });

  it("keeps the explicit revision for a clean worktree", async () => {
    await mount(); await beginCreate(); await fill("worktrees.start", "feature/base~2");
    submit(); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("create_worktree", { request: {
      sourceRoot, destination: "F:/Game/new", branch: "codex/new", startRef: "feature/base~2", includeDirty: false,
    } });
  });

  it("passes the pool root, chosen commit, optional branch and user capacity", async () => {
    await mount(); await beginCreate(true);
    await fill("worktrees.branch", " ");
    await fill("worktrees.start", " feature/pool~3 ");
    await fill("worktrees.maxSlots", "2");
    expect(root.querySelector('[role="checkbox"]')).toBeNull();
    submit(); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("acquire_unity_project_slot", { allowNewProject: false, request: {
      sourceRoot, poolRoot: "F:/Game/new", commit: "feature/pool~3", branch: null, maxSlots: 2,
    } });
  });

  it("rejects missing or invalid creation inputs before sending any mutation", async () => {
    await mount(); await beginCreate(true);
    for (const value of ["", "0", "-1", "1.5", "9007199254740992"]) {
      await fill("worktrees.maxSlots", value);
      expect(root.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
      submit(); await settle();
    }
    expect(mocks.invoke.mock.calls.some(([command]) => command === "acquire_unity_project_slot")).toBe(false);
  });

  it("keeps focus and editable form when creation fails, then permits retry", async () => {
    await mount(); await beginCreate();
    mocks.openAndFocus.mockClear();
    mocks.invoke.mockRejectedValueOnce(new Error("destination already exists"));
    submit(); await settle();
    expect(mocks.openAndFocus).not.toHaveBeenCalled();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("destination already exists");
    expect(field("worktrees.branch").value).toBe("codex/new");
    expect(field("worktrees.branch").disabled).toBe(false);
    submit(); await settle();
    expect(mocks.openAndFocus).toHaveBeenCalledExactlyOnceWith(created.root);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("blocks duplicate submissions and dismissal while a create is pending", async () => {
    await mount(); await beginCreate();
    const pending = deferred<ManagedWorktree>();
    mocks.invoke.mockResolvedValueOnce({ startOid: "HEAD", requiresNewProject: false, atCapacity: false, budget: null });
    mocks.invoke.mockReturnValueOnce(pending.promise);
    submit(); await settle();
    expect(Array.from(root.querySelectorAll<HTMLButtonElement>("button")).every((item) => item.disabled)).toBe(true);
    root.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    root.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true }));
    await settle();
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "create_worktree")).toHaveLength(1);
    expect(mocks.close).not.toHaveBeenCalled();
    pending.resolve(created); await settle();
    button("common.close").click();
    expect(mocks.close).toHaveBeenCalledOnce();
  });

  it("captures the selected epoch before delete confirmation and locks concurrent actions", async () => {
    await mount();
    const confirmation = deferred<boolean>();
    mocks.confirm.mockReturnValueOnce(confirmation.promise);
    button("worktrees.remove").click(); await settle();
    expect(button("common.refresh").disabled).toBe(true);
    records[0]!.materializationEpoch = 99;
    confirmation.resolve(true); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("remove_managed_worktree", {
      sourceRoot, checkoutId: "checkout-a", expectedEpoch: 7,
    });
  });

  it("does not remove or change focus when confirmation is cancelled or deletion fails", async () => {
    await mount(); mocks.openAndFocus.mockClear();
    mocks.confirm.mockResolvedValueOnce(false);
    button("worktrees.remove").click(); await settle();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "remove_managed_worktree")).toBe(false);
    mocks.invoke.mockRejectedValueOnce(new Error("Stale checkout materialization epoch"));
    button("worktrees.remove").click(); await settle();
    expect(mocks.openAndFocus).not.toHaveBeenCalled();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Stale checkout materialization epoch");
    expect(root.querySelectorAll(".worktree-row")).toHaveLength(1);
  });

  it("disables opening unavailable checkouts and removal of dirty, assigned or source checkouts", async () => {
    records = [
      record({ checkoutId: "active" }),
      ...["preparing", "quarantined", "missing", "available", "removed"].map((lifecycle) => record({ checkoutId: lifecycle, lifecycle })),
      record({ checkoutId: "dirty", dirty: true }),
      record({ checkoutId: "assigned", poolSlot: true, assignmentId: "assignment-a" }),
      record({ checkoutId: "source", root: "f:\\GAME\\source\\" }),
      record({ checkoutId: "external", managed: false }),
    ];
    await mount();
    const rows = root.querySelectorAll<HTMLElement>(".worktree-row");
    expect(rows[0]!.querySelector<HTMLButtonElement>(".worktree-target")!.disabled).toBe(false);
    for (const index of [1, 2, 3, 4, 5]) expect(rows[index]!.querySelector<HTMLButtonElement>(".worktree-target")!.disabled).toBe(true);
    for (const index of [1, 2, 3, 6, 7, 8]) expect(button("worktrees.remove", rows[index]!).disabled).toBe(true);
    expect(button("worktrees.remove", rows[4]!).disabled).toBe(false);
    expect(rows[9]!.textContent).not.toContain("worktrees.remove");
  });

  it("releases the selected pool assignment with its captured epoch without changing focus", async () => {
    records = [record({ poolSlot: true, assignmentId: "job-seven" })];
    await mount(); mocks.openAndFocus.mockClear();
    button("worktrees.release").click(); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("release_unity_project_slot", {
      sourceRoot, checkoutId: "checkout-a", expectedEpoch: 7, assignmentId: "job-seven",
    });
    expect(mocks.openAndFocus).not.toHaveBeenCalled();
  });

  it("disables release for dirty or unfinished pool assignments", async () => {
    records = [record({ poolSlot: true, assignmentId: "dirty", dirty: true }), record({ checkoutId: "pending", poolSlot: true, assignmentId: "pending", lifecycle: "preparing" })];
    await mount();
    for (const row of root.querySelectorAll(".worktree-row")) expect(button("worktrees.release", row).disabled).toBe(true);
  });

  it("imports an existing directory and reports native picker failures in the dialog", async () => {
    await mount(); mocks.openAndFocus.mockClear();
    mocks.open.mockRejectedValueOnce(new Error("picker unavailable"));
    button("worktrees.import").click(); await settle();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("picker unavailable");
    expect(mocks.openAndFocus).not.toHaveBeenCalled();
    mocks.open.mockResolvedValueOnce("F:/Game/external");
    button("worktrees.import").click(); await settle();
    expect(mocks.invoke).toHaveBeenCalledWith("import_worktree", { sourceRoot, targetRoot: "F:/Game/external" });
    expect(mocks.openAndFocus).toHaveBeenCalledExactlyOnceWith(created.root);
  });

  it("retries source registration after it fails instead of querying an unopened source", async () => {
    mocks.openWorkspace.mockRejectedValueOnce(new Error("source temporarily unavailable"));
    await mount();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("source temporarily unavailable");
    button("common.refresh").click(); await settle();
    expect(mocks.openWorkspace).toHaveBeenCalledTimes(2);
    expect(mocks.invoke).toHaveBeenCalledWith("list_managed_worktrees", { sourceRoot });
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });
});
