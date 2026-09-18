import { beforeEach, describe, expect, it, vi } from "vitest";
import zh from "../language/zh.json";
import en from "../language/en.json";
import type { GitCommitInfo, GitGraphRef, GitStashEntry } from "../types";
import { buildGitMenu, validateGitName, type GitContextTarget, type GitMenuContext } from "../components/collab/gitContextMenu";

const language = vi.hoisted(() => ({ value: "zh" }));
vi.mock("../i18n", () => ({ t: (key: string, ...args: (string | number)[]) => {
  const catalog: Record<string, string> = language.value === "zh" ? zh : en;
  return (catalog[key] ?? key).replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)] ?? `{${index}}`));
} }));
const commit: GitCommitInfo = { hash: "1234567890", shortHash: "1234567", message: "菜单测试", author: "tester", date: 1, parents: [], refs: [], isStash: false };
const stash: GitStashEntry = { ...commit, index: 0, refName: "stash@{0}", parentHashes: ["base"], baseHash: "base" };
const local: GitContextTarget = { kind: "localBranch", branch: { name: "main", isCurrent: true, message: "main", shortHash: "1234567" } };
const remote: GitContextTarget = { kind: "remoteBranch", remoteName: "upstream", branch: { name: "feature/a", shortHash: "1234567", message: "feature" } };
const file: GitContextTarget = { kind: "file", source: "gitUnstaged", file: { path: "Assets/Test.cs", status: "M", lfs: false }, selectedFiles: [{ path: "Assets/Test.cs", status: "M", lfs: false }] };
const graphRefs: GitGraphRef[] = [{ fullName: "refs/heads/main", shortName: "main", branchName: "main", isCurrent: true, kind: "localBranch", targetHash: commit.hash }];
const context: GitMenuContext = { busy: false, conflict: false, conflictHint: "conflict", currentBranch: "main", localBranches: [local.branch], graphRefs, unityConnected: true };
const items = (target: GitContextTarget, overrides: Partial<GitMenuContext> = {}) => buildGitMenu(target, { ...context, ...overrides }).flat();

describe("Collab Git menu", () => {
  beforeEach(() => { language.value = "zh"; });
  it("offers commit cherry-pick, separate branch creation, tags and copy actions", () => {
    expect(items({ kind: "commit", commit }).map(item => item.action)).toEqual([
      "checkoutBranch", "checkoutDetached", "createBranch", "createBranchAndCheckout", "createTag", "cherryPick", "revert",
      "resetSoft", "resetMixed", "resetHard", "copyHash", "copyMessage",
    ]);
  });
  it("disables switch, merge, rebase and delete on the current local branch", () => {
    const menu = items(local);
    for (const action of ["checkoutBranch", "mergeIntoCurrent", "rebaseCurrentOnto", "deleteBranch"]) {
      expect(menu.find(item => item.action === action)?.disabled).toBe(true);
    }
    for (const action of ["pull", "push", "renameBranch", "copyHash"]) {
      expect(menu.find(item => item.action === action)?.disabled).toBe(false);
    }
  });
  it("labels a remote checkout as switching the existing local branch, without promising a remote update", () => {
    const menu = items(remote, { localBranches: [{ ...local.branch, name: "feature/a", isCurrent: false }] });
    expect(menu[0]?.label).toBe("切换到分支「feature/a」");
    expect(menu[0]?.title).toContain("不会更新其提交");
    const tracking = items(remote, { localBranches: [] });
    expect(tracking[0]?.label).toBe("跟踪远程分支「upstream/feature/a」");
    expect(tracking.map(item => item.action)).toContain("deleteRemoteBranch");
    expect(tracking.map(item => item.action)).toContain("fetch");
    expect(tracking.map(item => item.action)).not.toContain("push");
  });
  it("does not offer branch integration when HEAD is detached", () => {
    for (const action of ["mergeIntoCurrent", "rebaseCurrentOnto"]) {
      expect(items(remote, { currentBranch: "" }).find(item => item.action === action)?.disabled).toBe(true);
    }
  });
  it("keeps copies available while mutations are busy or conflicted", () => {
    for (const state of [{ busy: true }, { conflict: true }]) {
      for (const target of [{ kind: "commit" as const, commit }, local, remote, { kind: "stash" as const, stash }]) {
        for (const item of items(target, state)) {
          if (!item.action.startsWith("copy")) expect(item.disabled, item.action).toBe(true);
        }
        expect(items(target, state).find(item => item.action === "copyMessage" || item.action === "copyBranch")?.disabled).toBe(false);
      }
    }
  });
  it("offers stash index restore and branch creation, and only drop for multiple stashes", () => {
    expect(items({ kind: "stash", stash }).map(item => item.action)).toContain("stashApplyIndex");
    expect(items({ kind: "stash", stash }).map(item => item.action)).toContain("stashBranch");
    const multi = items({ kind: "stash", stash, selectedStashes: [stash, { ...stash, index: 1 }] });
    expect(multi).toHaveLength(1);
    expect(multi[0]).toMatchObject({ action: "stashDrop", label: "删除 2 个 Stash…", danger: true });
  });
  it("keeps historical files read-only and supports current-file opening and path copy", () => {
    const actions = items({ ...file, source: "gitCommit", commitHash: commit.hash }).map(item => item.action);
    expect(actions).toEqual(["viewDiff", "openEditor", "selectUnity", "showInFolder", "copyRelativePath", "copyAbsolutePath"]);
  });
  it("uses batch labels and disables single-file actions for multi-selection", () => {
    const menu = items({ ...file, selectedFiles: [file.file, { ...file.file, path: "Assets/B.cs" }] });
    expect(menu.find(item => item.action === "stage")?.label).toBe("暂存 2 个文件");
    expect(menu.find(item => item.action === "viewDiff")?.disabled).toBe(true);
    expect(menu.find(item => item.action === "copyRelativePath")?.disabled).toBe(false);
  });
  it("allows staging conflict resolutions but blocks discard and nonexistent-file opening", () => {
    const menu = items({ ...file, file: { ...file.file, status: "D" } }, { conflict: true });
    expect(menu.find(item => item.action === "stage")?.disabled).toBe(false);
    expect(menu.find(item => item.action === "discard")?.disabled).toBe(true);
    expect(menu.find(item => item.action === "openEditor")?.disabled).toBe(true);
  });
  it("resolves every label and hint in both languages, with matching parameters", () => {
    const zhKeys = Object.keys(zh).filter(key => key.startsWith("collab.menu."));
    expect(zhKeys.sort()).toEqual(Object.keys(en).filter(key => key.startsWith("collab.menu.")).sort());
    for (const key of zhKeys) {
      expect((zh as Record<string, string>)[key]!.match(/\{\d+\}/g)).toEqual((en as Record<string, string>)[key]!.match(/\{\d+\}/g));
    }
    for (const locale of ["zh", "en"]) {
      language.value = locale;
      for (const target of [{ kind: "commit" as const, commit }, local, remote, { kind: "stash" as const, stash }, file]) {
        for (const item of items(target)) {
          expect(item.label).not.toMatch(/collab\.menu\.|\{\d+\}/);
          expect(item.title ?? "").not.toContain("collab.menu.");
        }
      }
    }
  });
  it.each(["--force", "bad name", "a..b", "a@{b", "a.lock", ".hidden/a", "a//b", "a/b.", "a\\b", "a:b", "a["])("rejects invalid Git names: %s", name => {
    expect(validateGitName(name)).toBe(false);
  });
  it.each(["feature/白盒-4", "release/v1.0", "my-branch"])("accepts valid Git names: %s", name => {
    expect(validateGitName(name)).toBe(true);
  });
});
