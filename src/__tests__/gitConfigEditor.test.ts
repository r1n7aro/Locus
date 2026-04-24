import { describe, expect, it } from "vitest";
import {
  gitConfigEntriesSignature,
  isSafeGitConfigKey,
  normalizeGitConfigEntries,
} from "../components/collab/gitConfigEditor";

describe("gitConfigEditor", () => {
  it("accepts common repo and global config keys", () => {
    expect(isSafeGitConfigKey("user.name")).toBe(true);
    expect(isSafeGitConfigKey("remote.origin.fetch")).toBe(true);
    expect(isSafeGitConfigKey("http.https://example.com.proxy")).toBe(true);
  });

  it("rejects empty, option-like, and newline config keys", () => {
    expect(isSafeGitConfigKey("")).toBe(false);
    expect(isSafeGitConfigKey("user")).toBe(false);
    expect(isSafeGitConfigKey("--global.user.name")).toBe(false);
    expect(isSafeGitConfigKey("user.\nname")).toBe(false);
  });

  it("trims keys without changing values before save", () => {
    expect(normalizeGitConfigEntries([
      { key: " user.name ", value: "  Jane  " },
    ])).toEqual([
      { key: "user.name", value: "  Jane  " },
    ]);
  });

  it("uses normalized entries for dirty checks", () => {
    expect(gitConfigEntriesSignature([
      { key: " user.email ", value: "dev@example.com" },
    ])).toBe(gitConfigEntriesSignature([
      { key: "user.email", value: "dev@example.com" },
    ]));
  });
});
