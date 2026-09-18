// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import StagingArea from "../components/collab/StagingArea.vue";
import type { GitFileChange } from "../types";

const mocks = vi.hoisted(() => ({ commit: vi.fn(), generate: vi.fn() }));
vi.mock("../services/git", () => ({ gitCommit: mocks.commit, gitGenerateCommitMessage: mocks.generate }));
vi.mock("../i18n", () => ({ t: (key: string) => key }));

const workspace = { checkoutId: "checkout-a", expectedGeneration: 3, expectedMaterializationEpoch: 7 };
const stagedFile: GitFileChange = { path: "Assets/Player.cs", status: "M", lfs: false };
let app: App | undefined;

async function flush() {
  for (let index = 0; index < 6; index++) await nextTick();
}

async function mountStaging() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const props = reactive({
    workspaceRef: { ...workspace },
    unstagedFiles: [] as GitFileChange[],
    stagedFiles: [stagedFile],
    blockedFiles: [],
    selectedModelId: "test-model",
    models: [],
    currentBranch: "main",
    totalChanges: 1,
    activeFilePath: null,
    pendingStagePaths: new Set<string>(),
    pendingUnstagePaths: new Set<string>(),
    pendingDiscardPaths: new Set<string>(),
    stageOperationBusy: false,
  });
  const committed = vi.fn();
  app = createApp(() => h(StagingArea, { ...props, onCommitted: committed }));
  app.mount(host);
  await flush();
  return { host, props, committed };
}

function button(host: HTMLElement) {
  return host.querySelector<HTMLButtonElement>(".commit-btn")!;
}

function titleInput(host: HTMLElement) {
  return host.querySelector<HTMLInputElement>(".staging-commit-input")!;
}

async function fill(host: HTMLElement, title: string, description = "") {
  const input = titleInput(host);
  input.value = title;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;
  textarea.value = description;
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  await flush();
}

async function expand(host: HTMLElement) {
  button(host).click();
  await flush();
}

function submit(host: HTMLElement) {
  host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.commit.mockResolvedValue(undefined);
  mocks.generate.mockResolvedValue({ title: "Generated title", description: "Generated description" });
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("inline staging commit", () => {
  it("expands two fields above the existing button and preserves a collapsed draft", async () => {
    const { host } = await mountStaging();
    const trigger = button(host);
    expect(host.querySelector("input")).toBeNull();
    await expand(host);
    const fields = host.querySelector(".staging-commit-fields")!;
    expect(fields.nextElementSibling).toBe(trigger);
    expect(host.querySelectorAll("input, textarea")).toHaveLength(2);
    expect(trigger.textContent).toContain("collab.confirmCommit");
    expect(trigger.disabled).toBe(true);
    expect(document.activeElement).toBe(titleInput(host));
    expect(document.querySelector(".commit-modal-overlay")).toBeNull();
    expect(mocks.commit).not.toHaveBeenCalled();

    await fill(host, "Draft title", "Draft description");
    titleInput(host).dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flush();
    expect(host.querySelector("input")).toBeNull();
    expect(document.activeElement).toBe(trigger);
    await expand(host);
    expect(titleInput(host).value).toBe("Draft title");
    expect(host.querySelector("textarea")!.value).toBe("Draft description");
    expect(mocks.commit).not.toHaveBeenCalled();
  });

  it("submits once to the current checkout and clears the form on success", async () => {
    let finish!: () => void;
    mocks.commit.mockReturnValueOnce(new Promise<void>(resolve => { finish = resolve; }));
    const { host, committed } = await mountStaging();
    await expand(host);
    await fill(host, "Fix player movement", "Handle grounded state");
    button(host).click();
    submit(host);
    await flush();
    expect(mocks.commit).toHaveBeenCalledExactlyOnceWith("Fix player movement", "Handle grounded state", workspace);
    expect(button(host).disabled).toBe(true);
    expect(titleInput(host).disabled).toBe(true);
    expect(host.querySelector<HTMLButtonElement>('[aria-label="collab.collapse"]')!.disabled).toBe(true);
    finish();
    await flush();
    expect(committed).toHaveBeenCalledOnce();
    expect(host.querySelector("input")).toBeNull();
    await expand(host);
    expect(titleInput(host).value).toBe("");
    expect(host.querySelector("textarea")!.value).toBe("");
  });

  it("keeps the draft and shows commit errors inline for retry", async () => {
    mocks.commit.mockRejectedValueOnce(new Error("Pre-commit hook failed"));
    const { host, committed } = await mountStaging();
    await expand(host);
    await fill(host, "Keep this draft");
    button(host).click();
    await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Pre-commit hook failed");
    expect(titleInput(host).value).toBe("Keep this draft");
    expect(button(host).disabled).toBe(false);
    expect(committed).not.toHaveBeenCalled();
    button(host).click();
    await flush();
    expect(mocks.commit).toHaveBeenLastCalledWith("Keep this draft", null, workspace);
    expect(committed).toHaveBeenCalledOnce();
  });

  it("blocks blank titles and commits during staging or after unstaging all files", async () => {
    const { host, props } = await mountStaging();
    await expand(host);
    await fill(host, "   ");
    submit(host);
    await fill(host, "Draft");
    props.stageOperationBusy = true;
    await flush();
    expect(button(host).disabled).toBe(true);
    submit(host);
    props.stageOperationBusy = false;
    props.stagedFiles = [];
    await flush();
    expect(titleInput(host).value).toBe("Draft");
    expect(button(host).disabled).toBe(true);
    submit(host);
    expect(mocks.commit).not.toHaveBeenCalled();
    props.stagedFiles = [stagedFile];
    await flush();
    expect(button(host).disabled).toBe(false);
  });

  it("does not submit when Enter is used to confirm IME composition", async () => {
    const { host } = await mountStaging();
    await expand(host);
    await fill(host, "修复移动");
    titleInput(host).dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }));
    await flush();
    expect(mocks.commit).not.toHaveBeenCalled();
    titleInput(host).dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    expect(mocks.commit).toHaveBeenCalledExactlyOnceWith("修复移动", null, workspace);
  });

  it("fills both fields with AI and blocks submission until generation finishes", async () => {
    let finish!: (value: { title: string; description: string }) => void;
    mocks.generate.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const { host } = await mountStaging();
    await expand(host);
    await fill(host, "Existing draft");
    host.querySelector<HTMLButtonElement>('[aria-label="collab.aiGenerate"]')!.click();
    await flush();
    expect(mocks.generate).toHaveBeenCalledExactlyOnceWith("test-model", workspace);
    expect(button(host).disabled).toBe(true);
    submit(host);
    expect(mocks.commit).not.toHaveBeenCalled();
    finish({ title: "Generated title", description: "Generated description" });
    await flush();
    expect(titleInput(host).value).toBe("Generated title");
    expect(host.querySelector("textarea")!.value).toBe("Generated description");
    expect(button(host).disabled).toBe(false);
  });

  it("ignores a pending AI result after the form is collapsed", async () => {
    let finish!: (value: { title: string; description: string }) => void;
    mocks.generate.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const { host } = await mountStaging();
    await expand(host);
    await fill(host, "Keep draft");
    host.querySelector<HTMLButtonElement>('[aria-label="collab.aiGenerate"]')!.click();
    await flush();
    host.querySelector<HTMLButtonElement>('[aria-label="collab.collapse"]')!.click();
    await flush();
    await expand(host);
    finish({ title: "Stale title", description: "Stale description" });
    await flush();
    expect(titleInput(host).value).toBe("Keep draft");
    expect(button(host).disabled).toBe(false);
  });

  it.each(["checkoutId", "expectedGeneration", "expectedMaterializationEpoch"] as const)(
    "resets the draft and ignores pending results when %s changes",
    async (field) => {
      let finish!: () => void;
      mocks.commit.mockReturnValueOnce(new Promise<void>(resolve => { finish = resolve; }));
      const { host, props, committed } = await mountStaging();
      await expand(host);
      await fill(host, "Old checkout draft");
      button(host).click();
      await flush();
      if (field === "checkoutId") props.workspaceRef.checkoutId = "checkout-b";
      else props.workspaceRef[field] += 1;
      await flush();
      await expand(host);
      await fill(host, "New checkout draft");
      finish();
      await flush();
      expect(titleInput(host).value).toBe("New checkout draft");
      expect(committed).not.toHaveBeenCalled();
    },
  );
});
