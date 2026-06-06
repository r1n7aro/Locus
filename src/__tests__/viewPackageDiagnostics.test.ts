import { describe, expect, it } from "vitest";
import { findMigratedViewRuntimeApiUsage } from "../components/view/viewPackageDiagnostics";
import type { ViewPackageFile } from "../services/view";

function sourceFile(relPath: string, content: string): ViewPackageFile {
  return {
    relPath,
    kind: "source",
    content,
    size: content.length,
    truncated: false,
  };
}

describe("viewPackageDiagnostics", () => {
  it("detects migrated view binding member usage", () => {
    const message = findMigratedViewRuntimeApiUsage(sourceFile(
      "src/App.vue",
      `<script setup lang="ts">
import { view } from "@locus/view-runtime";
await view.binding.read({ bindingId: "main" });
</script>`,
    ));

    expect(message).toContain("src/App.vue:3:7");
    expect(message).toContain("`view.binding`");
    expect(message).toContain("`property`");
  });

  it("detects migrated view binding helper usage", () => {
    const message = findMigratedViewRuntimeApiUsage(sourceFile(
      "src/main.ts",
      `import { view } from "@locus/view-runtime";
await view.writeBinding("main", true);`,
    ));

    expect(message).toContain("`view.writeBinding`");
    expect(message).toContain("property.write");
  });

  it("detects migrated runtime imports", () => {
    const message = findMigratedViewRuntimeApiUsage(sourceFile(
      "src/App.vue",
      `import {
  defineView,
  useUnityBinding as useMaterialBinding,
} from "@locus/view-runtime";`,
    ));

    expect(message).toContain("src/App.vue:3:3");
    expect(message).toContain("`useUnityBinding`");
    expect(message).toContain("property.fromPath");
  });

  it("passes current property runtime usage", () => {
    const message = findMigratedViewRuntimeApiUsage(sourceFile(
      "src/main.ts",
      `import { property, view } from "@locus/view-runtime";
await property.write("selection/property/m_Name", "Player");
view.reload();`,
    ));

    expect(message).toBeNull();
  });
});
