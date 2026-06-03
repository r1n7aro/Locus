import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { isUnityConnectionError } from "../services/errors";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("isUnityConnectionError", () => {
  it("detects Unity pipe connection failures", () => {
    expect(isUnityConnectionError(
      "Failed to connect to Unity Editor (\\\\.\\pipe\\locus_unity_project): 系统找不到指定的文件。 (os error 2)",
    )).toBe(true);
  });

  it("keeps asset content errors separate", () => {
    expect(isUnityConnectionError("Asset was not found: Assets/Missing.prefab")).toBe(false);
  });

  it("uses the compact Unity connection message in property fences", () => {
    const component = read("src/components/unity/UnityPropertyFenceBlock.vue");
    expect(component).toContain("isUnityConnectionError");
    expect(component).toContain("asset.preview.unityConnectionRequired");
  });
});
