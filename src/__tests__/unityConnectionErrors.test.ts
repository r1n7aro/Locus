import { describe, expect, it } from "vitest";
import { isUnityConnectionError } from "../services/errors";

describe("isUnityConnectionError", () => {
  it("detects Unity pipe connection failures", () => {
    expect(isUnityConnectionError(
      "Failed to connect to Unity Editor (\\\\.\\pipe\\locus_unity_project): 系统找不到指定的文件。 (os error 2)",
    )).toBe(true);
  });

  it("keeps asset content errors separate", () => {
    expect(isUnityConnectionError("Asset was not found: Assets/Missing.prefab")).toBe(false);
  });
});
