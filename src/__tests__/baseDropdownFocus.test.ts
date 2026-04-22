import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("BaseDropdown focus ownership", () => {
  it("handles keyboard input only inside the dropdown tree", () => {
    const dropdown = read("src/components/ui/BaseDropdown.vue");

    expect(dropdown).toContain('@keydown.capture="onKeydown"');
    expect(dropdown).not.toContain('document.addEventListener("keydown", onKeydown)');
    expect(dropdown).not.toContain('@keydown="onKeydown"');
  });
});
