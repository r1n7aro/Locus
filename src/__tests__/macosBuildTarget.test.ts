import { describe, expect, it } from "vitest";
import { resolveMacosTarget } from "../../scripts/macos-build-target.mjs";

describe("macOS build target selection", () => {
  it("does not change the Windows default", () => {
    expect(resolveMacosTarget([], {}, "win32", "x64")).toBeNull();
  });
  it.each([
    ["aarch64-apple-darwin", "arm64"], ["x86_64-apple-darwin", "x64"],
  ])("uses the requested %s target on a Windows host", (triple, arch) => {
    expect(resolveMacosTarget(["build", "--target", triple!], {}, "win32", "x64"))
      .toEqual({ triple, arch });
    expect(resolveMacosTarget([], { TAURI_ENV_TARGET_TRIPLE: triple }, "win32", "x64"))
      .toEqual({ triple, arch });
  });
  it("uses the native Mac architecture only in the absence of a target", () => {
    expect(resolveMacosTarget([], {}, "darwin", "arm64")?.arch).toBe("arm64");
    expect(resolveMacosTarget(["--target=x86_64-apple-darwin"], {}, "darwin", "arm64")?.arch).toBe("x64");
    expect(resolveMacosTarget(["--target=x86_64-pc-windows-msvc"], {}, "darwin", "arm64")).toBeNull();
  });
  it("does not guess for unsupported Darwin targets", () => {
    expect(() => resolveMacosTarget(["--target=universal-apple-darwin"], {}, "win32", "x64")).toThrow("Unsupported macOS target");
  });
});
