import { describe, expect, it } from "vitest";
import {
  buildUniquePath,
  resolveStableExternalImportTargetPath,
} from "../components/knowledge/referenceExternalImportPaths";

describe("referenceExternalImportPaths", () => {
  it("keeps a materialized target path stable after the directory is created", () => {
    const existingPaths = new Set<string>();
    const pathExists = (path: string) => existingPaths.has(path);

    const initialPath = resolveStableExternalImportTargetPath({
      basePath: "unity-official-docs",
      pathExists,
    });

    expect(initialPath).toBe("unity-official-docs");

    existingPaths.add(initialPath);

    expect(buildUniquePath("unity-official-docs", pathExists)).toBe("unity-official-docs-2");
    expect(resolveStableExternalImportTargetPath({
      basePath: "unity-official-docs",
      materializedTargetPath: initialPath,
      pathExists,
    })).toBe("unity-official-docs");
  });

  it("prefers an existing bound path over the materialized fallback", () => {
    expect(resolveStableExternalImportTargetPath({
      basePath: "unity-official-docs",
      preferredTargetPath: "external/unity-docs",
      materializedTargetPath: "unity-official-docs",
    })).toBe("external/unity-docs");
  });
});
