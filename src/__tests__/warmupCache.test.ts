import { beforeEach, describe, expect, it } from "vitest";
import { clearWarmup, getWarmup, setScope, setWarmup } from "../composables/warmupCache";

describe("warmupCache", () => {
  beforeEach(() => {
    clearWarmup();
  });

  it("drops stale writes after the scope changes", () => {
    const firstGeneration = setScope("F:/repo-a");
    expect(setWarmup("collab:probe", { isRepo: false }, firstGeneration)).toBe(true);
    expect(getWarmup("collab:probe")).toEqual({ isRepo: false });

    const secondGeneration = setScope("F:/repo-b");

    expect(setWarmup("collab:probe", { isRepo: false }, firstGeneration)).toBe(false);
    expect(getWarmup("collab:probe")).toBeUndefined();

    expect(setWarmup("collab:probe", { isRepo: true }, secondGeneration)).toBe(true);
    expect(getWarmup("collab:probe")).toEqual({ isRepo: true });
  });
});
