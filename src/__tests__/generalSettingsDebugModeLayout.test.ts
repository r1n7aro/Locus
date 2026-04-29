import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("general settings debug mode switch", () => {
  it("renders the switch only after debug mode is hydrated", () => {
    const source = read("src/components/settings/GeneralSettings.vue");
    const permissions = read("src/services/permissions.ts");

    expect(source).toContain("const initialDebugMode = getCachedDebugMode();");
    expect(source).toContain("const debugReady = ref(initialDebugMode !== null);");
    expect(source).toContain("if (!debugReady.value) return t(\"common.loading\");");
    expect(source).toContain('v-if="debugReady"');
    expect(source).toContain('class="debug-toggle-placeholder"');
    expect(permissions).toContain("let cachedDebugMode: boolean | null = null;");
    expect(permissions).toContain("export function getCachedDebugMode(): boolean | null");
  });
});
