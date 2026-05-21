import { readdirSync, readFileSync } from "node:fs";
import { extname, join, resolve, relative } from "node:path";
import { describe, expect, it } from "vitest";

const TEXT_EXTENSIONS = new Set([".css", ".js", ".ts", ".vue"]);
const cwd = process.cwd();
const srcRoot = resolve(cwd, "src");

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function collectTextFiles(dir: string): string[] {
  const files: string[] = [];

  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const fullPath = join(dir, entry.name);

    if (entry.isDirectory()) {
      files.push(...collectTextFiles(fullPath));
      continue;
    }

    if (TEXT_EXTENSIONS.has(extname(entry.name))) {
      files.push(fullPath);
    }
  }

  return files;
}

describe("selection source policy", () => {
  it("does not use legacy body.style.userSelect mutations", () => {
    const violations: string[] = [];

    for (const file of collectTextFiles(srcRoot)) {
      if (file.includes(`${join("src", "__tests__")}`)) continue;
      const content = readFileSync(file, "utf8");
      if (!/style\.userSelect/.test(content)) continue;
      violations.push(relative(cwd, file).replaceAll("\\", "/"));
    }

    expect(violations).toEqual([]);
  });

  it("keeps container-level selection rules out of critical layouts", () => {
    const checks = [
      {
        file: "src/App.vue",
        pattern: /\.app-layout\s*\{[^}]*user-select\s*:/,
      },
      {
        file: "src/components/ChatView.vue",
        pattern: /\.chat-view-layout\.dragging-session\s*\{[^}]*user-select\s*:/,
      },
      {
        file: "src/components/CollabView.vue",
        pattern: /\.collab-view\.dragging-(?:v|sidebar|h)\s*\{[^}]*user-select\s*:/,
      },
      {
        file: "src/components/collab/MergeSemanticView.vue",
        pattern: /\.merge-semantic-view\.sidebar-dragging\s*\{[^}]*user-select\s*:/,
      },
    ];

    const violations = checks
      .filter(({ file, pattern }) => pattern.test(read(file)))
      .map(({ file }) => file);

    expect(violations).toEqual([]);
  });

  it("routes drag selection lock through the shared helper", () => {
    const helperUsers = [
      "src/components/AgentView.vue",
      "src/components/ChatView.vue",
      "src/components/collab/MergeSemanticView.vue",
      "src/components/collab/StagingArea.vue",
      "src/components/diff/ImageBinaryPreview.vue",
      "src/components/diff/RasterBinaryPreview.vue",
      "src/composables/useAssetState.ts",
      "src/composables/useCollabState.ts",
      "src/composables/useKnowledgeState.ts",
    ];

    const missing = helperUsers.filter((file) => !read(file).includes("acquireSelectionLock"));
    expect(missing).toEqual([]);
  });
});
