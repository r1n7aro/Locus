// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorktreeSelectorPanel from "../components/chat/WorktreeSelectorPanel.vue";
import ModelEffortSelector from "../components/ModelEffortSelector.vue";
import type { ManagedWorktree, WorktreeBranchOption, WorktreeCreationPlan, WorktreeProgressHandler } from "../services/worktrees";
import { resetWorktreeCreationJobsForTests } from "../components/chat/worktreeCreationJobs";
import { resetWorktreeRepositoriesForTests } from "../components/chat/worktreeRepositories";

const mocks = vi.hoisted(() => ({ list: vi.fn(), plan: vi.fn(), select: vi.fn(), selected: vi.fn(), bind: vi.fn(), busy: vi.fn() }));
vi.mock("../services/worktrees", () => ({ listWorktreeBranches: mocks.list, planWorktreeSelection: mocks.plan, selectWorktreeBranch: mocks.select }));
vi.mock("../i18n", () => ({ t: (key: string, ...args: unknown[]) => [key, ...args].join(' ') }));
let app: App;
let host: HTMLDivElement;
const state = reactive({ locked: false, workspaceRef: { checkoutId: "source", expectedGeneration: 3, expectedMaterializationEpoch: 1 } });
const target = { checkoutId: "target", root: "F:/source.worktrees/slot", materializationEpoch: 2 } as ManagedWorktree;
const newProjectPlan = { startOid: "frozen-head", requiresNewProject: true, atCapacity: false, budget: {
  checkoutBytes: 1024, referenceCacheBytes: 2048, estimatedBytes: 3072, cacheKnown: true, freeBytes: 1000000, directory: "F:/source.worktrees",
}, directory: "F:/source.worktrees" };
const items: WorktreeBranchOption[] = [
  { branch: "main", root: "F:/source", current: true, dirty: true, headOid: "abc", unavailable: false },
  { branch: "feature/one", root: null, current: false, dirty: false, headOid: "def", unavailable: false },
];
async function settle() { for (let i = 0; i < 15; i++) await Promise.resolve(); await nextTick(); }
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((accept) => { resolve = accept; }); return { promise, resolve }; }
function mountPanel() {
  app = createApp({ render: () => h(WorktreeSelectorPanel, { ...state, selectWorktree: mocks.bind, onSelected: mocks.selected, onBusy: mocks.busy }) });
  app.mount(host);
  return settle();
}
function row(name: string, root: ParentNode = host) { return [...root.querySelectorAll<HTMLButtonElement>(".worktree-option")].find((button) => button.textContent?.includes(name))!; }
async function fill(selector: string, value: string) { const input = host.querySelector<HTMLInputElement>(selector)!; input.value = value; input.dispatchEvent(new Event("input", { bubbles: true })); await settle(); }
beforeEach(() => {
  vi.resetAllMocks(); resetWorktreeCreationJobsForTests(); resetWorktreeRepositoriesForTests(); state.locked = false; state.workspaceRef = { checkoutId: "source", expectedGeneration: 3, expectedMaterializationEpoch: 1 };
  mocks.list.mockResolvedValue(items); mocks.select.mockResolvedValue(target); mocks.bind.mockResolvedValue(undefined);
  mocks.plan.mockResolvedValue({ startOid: "frozen-head", requiresNewProject: false, atCapacity: false, budget: null });
  host = document.createElement("div"); document.body.append(host);
});
afterEach(() => { app?.unmount(); host.remove(); });

describe("worktree picker", () => {
  it("creates a named local branch from a remote ref without copying current changes", async () => {
    mocks.list.mockResolvedValue([...items, { ...items[1]!, branch: "origin/白盒-4", remote: true }]);
    await mountPanel(); row("origin/白盒-4").click(); await settle();
    expect(host.querySelector<HTMLInputElement>(".worktree-field input")!.value).toBe("白盒-4");
    expect(host.querySelector(".worktree-source")?.textContent).toContain("origin/白盒-4");
    expect(host.querySelector(".worktree-dirty")).toBeNull();
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); await settle();
    expect(mocks.select).toHaveBeenCalledWith(expect.objectContaining({ branch: "白盒-4", createBranch: true, includeDirty: false, startRef: "refs/remotes/origin/白盒-4" }), expect.any(Function));
  });
  it("requires another local name if the remote branch name already exists", async () => {
    mocks.list.mockResolvedValue([...items, { ...items[1]!, branch: "origin/main", remote: true }]);
    await mountPanel(); row("origin/main").click(); await settle();
    expect(host.querySelector<HTMLInputElement>(".worktree-field input")!.getAttribute("aria-invalid")).toBe("true");
    expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
    await fill(".worktree-field input", "codex/from-remote");
    expect(host.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(false);
  });
  it("shows live measurements, allows canceling a preview and ignores its late result", async () => {
    const pending = deferred<WorktreeCreationPlan>();
    let report!: WorktreeProgressHandler;
    mocks.plan.mockImplementationOnce((_request, onProgress) => { report = onProgress; return pending.promise; });
    await mountPanel(); row("feature/one").click(); await settle();
    expect(host.querySelector('[role="status"]')?.textContent).toContain("worktrees.progress.checking");
    expect(mocks.busy).not.toHaveBeenCalledWith(true);
    report({ phase: "cache", files: 4000, totalFiles: null, bytes: 4096 }); await settle();
    expect(host.querySelector('[role="status"]')?.textContent).toContain("4,000 4.00 KB");
    host.querySelector<HTMLButtonElement>(".worktree-progress button")!.click(); await settle();
    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(row("feature/one").disabled).toBe(false);
    pending.resolve(newProjectPlan); await settle();
    expect(host.querySelector(".worktree-budget")).toBeNull(); expect(mocks.select).not.toHaveBeenCalled();
  });
  it("does not let a canceled preview overwrite a newer preview", async () => {
    const old = deferred<WorktreeCreationPlan>(); const newer = deferred<WorktreeCreationPlan>();
    mocks.plan.mockReturnValueOnce(old.promise).mockReturnValueOnce(newer.promise);
    await mountPanel(); row("feature/one").click(); await settle();
    host.querySelector<HTMLButtonElement>(".worktree-progress button")!.click(); await settle();
    row("feature/one").click(); await settle();
    old.resolve(newProjectPlan); await settle();
    expect(host.querySelector(".worktree-budget")).toBeNull(); expect(host.querySelector('[role="status"]')).not.toBeNull();
    newer.resolve(newProjectPlan); await settle(); expect(host.querySelector(".worktree-budget")).not.toBeNull();
  });
  it("selects the current workspace without consulting or allocating a pool", async () => {
    await mountPanel(); row("main").click(); await settle();
    expect(mocks.plan).not.toHaveBeenCalled();
    expect(mocks.select).not.toHaveBeenCalled();
    expect(mocks.bind).not.toHaveBeenCalled();
    expect(mocks.selected).toHaveBeenCalledOnce();
  });
  it("searches branches, identifies current checkout and binds a prepared selection", async () => {
    mocks.list.mockResolvedValueOnce([...items, ...Array.from({ length: 7 }, (_, i) => ({ ...items[1]!, branch: `extra/${i}` }))]);
    await mountPanel();
    expect(row("main").getAttribute("aria-pressed")).toBe("true");
    expect(row("main").textContent).toContain("worktrees.dirty");
    await fill('input[type="search"]', "FEATURE");
    expect(row("main")).toBeUndefined(); row("feature/one").click(); await settle();
    expect(mocks.select).toHaveBeenCalledWith({ workspaceRef: state.workspaceRef, branch: "feature/one", createBranch: false, includeDirty: false, allowNewProject: false, expectedStartOid: "frozen-head" }, expect.any(Function));
    expect(mocks.bind).toHaveBeenCalledWith(target);
    expect(mocks.selected).toHaveBeenCalledOnce();
  });
  it.each([2, 8, 9])("shows search only above the branch threshold: %s", async (count) => {
    mocks.list.mockResolvedValueOnce(Array.from({ length: count }, (_, i) => ({ ...items[1]!, branch: `branch/${i}` })));
    await mountPanel();
    expect(host.querySelector('input[type="search"]') !== null).toBe(count > 8);
    expect(host.querySelectorAll(".worktree-options button")).toHaveLength(count);
  });
  it("hides unused pool staging refs without hiding attached checkouts", async () => {
    mocks.list.mockResolvedValueOnce([...items,
      { ...items[1]!, branch: `locus/pool/unity-${"a".repeat(32)}` },
      { ...items[1]!, branch: `locus/pool/unity-${"b".repeat(32)}`, root: "F:/source.worktrees/attached" },
    ]);
    await mountPanel();
    expect(row(`locus/pool/unity-${"a".repeat(32)}`)).toBeUndefined();
    expect(row(`locus/pool/unity-${"b".repeat(32)}`)).toBeDefined();
    expect(host.querySelectorAll(".worktree-options button")).toHaveLength(3);
  });
  it("creates in the background with checkout progress, survives closing the panel and adopts the result once", async () => {
    await mountPanel(); row("worktrees.selector.create").click(); await settle();
    await fill(".worktree-field input", "codex/new");
    const creating = deferred<ManagedWorktree>();
    let report!: WorktreeProgressHandler;
    mocks.select.mockImplementationOnce((_request, onProgress) => { report = onProgress; return creating.promise; });
    host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); await settle();
    expect(mocks.select).toHaveBeenCalledWith({ workspaceRef: state.workspaceRef, branch: "codex/new", createBranch: true, includeDirty: true, allowNewProject: false, expectedStartOid: "frozen-head" }, expect.any(Function));
    // The popup may close while creating; no second creation can start meanwhile.
    expect(mocks.busy).not.toHaveBeenCalledWith(true);
    expect(host.querySelector("form")).toBeNull();
    expect(row("feature/one").disabled).toBe(true);
    const status = () => host.querySelector('[role="status"]')?.textContent ?? "";
    expect(status()).toContain("codex/new"); expect(status()).toContain("worktrees.progress.background");
    report({ phase: "creating", files: 45, totalFiles: 100, bytes: 0 }); await settle();
    expect(status()).toContain("worktrees.progress.checkedOut 45 45 100");
    app.unmount(); host.innerHTML = ""; await mountPanel();
    expect(status()).toContain("worktrees.progress.checkedOut 45 45 100");
    expect(row("feature/one").disabled).toBe(true);
    creating.resolve(target); await settle();
    expect(mocks.bind).toHaveBeenCalledExactlyOnceWith(target);
    expect(mocks.selected).toHaveBeenCalledOnce();
    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(row("feature/one").disabled).toBe(false);
  });
  it("requires disk-budget confirmation when the pool needs a new project", async () => {
    mocks.plan.mockResolvedValue(newProjectPlan); await mountPanel();
    row("feature/one").click(); await settle();
    expect(mocks.select).not.toHaveBeenCalled();
    expect(host.querySelector(".worktree-budget")?.textContent).toContain("3.00 KB");
    expect(mocks.busy).toHaveBeenLastCalledWith(true);
    expect(host.querySelector(".budget-path")?.getAttribute("title")).toBe("F:/source.worktrees");
    [...host.querySelectorAll<HTMLButtonElement>(".budget-actions button")].find(el=>el.textContent === "worktrees.budget.confirm")!.click(); await settle();
    expect(mocks.select).toHaveBeenCalledWith(expect.objectContaining({ allowNewProject: true, expectedStartOid: "frozen-head" }), expect.any(Function));
  });
  it("cancels a creation estimate without creating or focusing a project", async () => {
    mocks.plan.mockResolvedValue(newProjectPlan); await mountPanel();
    row("feature/one").click(); await settle();
    [...host.querySelectorAll<HTMLButtonElement>(".budget-actions button")].find(el=>el.textContent === "common.cancel")!.click(); await settle();
    expect(host.querySelector(".worktree-budget")).toBeNull();
    expect(mocks.busy).toHaveBeenLastCalledWith(false);
    expect(mocks.select).not.toHaveBeenCalled(); expect(mocks.bind).not.toHaveBeenCalled();
  });
  it.each(["capacity", "disk"])("blocks allocating a project when constrained by %s", async (reason) => {
    mocks.plan.mockResolvedValue({ ...newProjectPlan, atCapacity: reason === "capacity", budget: { ...newProjectPlan.budget, freeBytes: reason === "disk" ? 1 : 1000000 } });
    await mountPanel(); row("feature/one").click(); await settle();
    const confirm = [...host.querySelectorAll<HTMLButtonElement>(".budget-actions button")].find(el=>el.textContent === "worktrees.budget.confirm");
    expect(!confirm || confirm.disabled).toBe(true); expect(mocks.select).not.toHaveBeenCalled();
  });
  it("locks branches and creation after the first message, including an already open form", async () => {
    await mountPanel(); row("worktrees.selector.create").click(); await settle();
    state.locked = true; await settle();
    expect(host.querySelector("form")).toBeNull();
    expect([...host.querySelectorAll<HTMLButtonElement>("button")].every((button) => button.disabled)).toBe(true);
    row("feature/one").click(); await settle(); expect(mocks.select).not.toHaveBeenCalled();
  });
  it("keeps failures visible across reopening, never binds a failed creation and clears it on retry", async () => {
    await mountPanel(); mocks.select.mockRejectedValueOnce(new Error("Pool is at capacity"));
    row("feature/one").click(); await settle();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Pool is at capacity");
    expect(mocks.bind).not.toHaveBeenCalled();
    app.unmount(); host.innerHTML = ""; await mountPanel();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Pool is at capacity");
    const preparing = deferred<ManagedWorktree>(); mocks.select.mockReturnValueOnce(preparing.promise);
    row("feature/one").click(); await settle();
    expect(host.querySelector('[role="alert"]')).toBeNull();
    app.unmount(); host.innerHTML = "";
    preparing.resolve(target); await settle();
    expect(mocks.bind).toHaveBeenCalledExactlyOnceWith(target);
    expect(mocks.selected).not.toHaveBeenCalled();
    await mountPanel(); expect(host.querySelector('[role="status"]')).toBeNull();
  });
  it("ignores late branch results after the workspace changes", async () => {
    const old = deferred<WorktreeBranchOption[]>(); mocks.list.mockReturnValueOnce(old.promise);
    await mountPanel(); state.workspaceRef = { ...state.workspaceRef, checkoutId: "other" }; await settle();
    old.resolve([{ ...items[0]!, branch: "stale" }]); await settle();
    expect(row("stale")).toBeUndefined(); expect(row("main")).toBeDefined();
  });
  it("shows the session branch after the effort only while worktrees are enabled", async () => {
    const enabled = reactive({ value: true });
    app = createApp({ render: () => h(ModelEffortSelector, { models: [], selectedId: "", effort: "high", effortSupported: true,
      worktreeEnabled: enabled.value, worktreeBranch: "master", workspaceRef: state.workspaceRef, selectWorktree: mocks.bind }) }); app.mount(host);
    await settle();
    const trigger = host.querySelector<HTMLButtonElement>(".model-effort-trigger")!;
    expect(trigger.querySelector(".model-effort-branch")?.textContent).toBe("master");
    expect(trigger.lastElementChild?.previousElementSibling?.className).toBe("model-effort-branch");
    expect(trigger.title).toContain("master");
    enabled.value = false; await settle();
    expect(trigger.querySelector(".model-effort-branch")).toBeNull();
  });
  it("adds the column only when enabled and keeps it to the right of effort", async () => {
    const enabled = reactive({ value: false });
    app = createApp({ render: () => h(ModelEffortSelector, { models: [], selectedId: "", effort: "high", worktreeEnabled: enabled.value,
      workspaceRef: state.workspaceRef, selectWorktree: mocks.bind }) }); app.mount(host);
    host.querySelector<HTMLButtonElement>(".model-effort-trigger")!.click(); await settle();
    expect(document.body.querySelector(".worktree-selector-panel")).toBeNull(); expect(mocks.list).not.toHaveBeenCalled();
    enabled.value = true; await settle();
    const panel = document.body.querySelector(".worktree-selector-panel")!;
    expect(panel.previousElementSibling?.className).toBe("model-effort-effort-panel");
    expect(panel.parentElement?.lastElementChild).toBe(panel);
    row("worktrees.selector.create", document.body).click(); await settle();
    expect(document.body.querySelector(".worktree-create")).not.toBeNull();
    expect(document.body.querySelector(".model-effort-dropdown")).not.toBeNull();
  });
  it("hides the column outside a Git repository and restores it once branches appear", async () => {
    const open = () => {
      app?.unmount(); document.body.querySelectorAll(".model-effort-dropdown").forEach((element) => element.remove());
      app = createApp({ render: () => h(ModelEffortSelector, { models: [], selectedId: "", effort: "high", worktreeEnabled: true,
        workspaceRef: state.workspaceRef, selectWorktree: mocks.bind }) }); app.mount(host);
      host.querySelector<HTMLButtonElement>(".model-effort-trigger")!.click();
    };
    const dropdown = () => document.body.querySelector<HTMLElement>(".model-effort-dropdown");
    const panel = () => document.body.querySelector<HTMLElement>(".worktree-selector-panel");
    mocks.list.mockResolvedValue([]);
    open(); await settle();
    expect(panel()?.style.display).toBe("none");
    expect(dropdown()?.classList.contains("has-worktree")).toBe(false);
    expect(document.body.querySelector('.worktree-selector-panel [role="alert"]')).toBeNull();
    // A known non-repository hides the column from the first frame of a later open.
    mocks.list.mockReturnValueOnce(new Promise(() => {}));
    open(); await nextTick();
    expect(dropdown()?.classList.contains("has-worktree")).toBe(false);
    // The hidden panel still lists branches, so `git init` brings the column back.
    mocks.list.mockResolvedValue(items);
    open(); await settle();
    expect(panel()?.style.display).toBe("");
    expect(dropdown()?.classList.contains("has-worktree")).toBe(true);
    expect(row("feature/one", document.body)).toBeDefined();
  });
});
