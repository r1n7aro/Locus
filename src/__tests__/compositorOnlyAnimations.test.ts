import { readdirSync, readFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

// Animations of these properties run on the compositor thread. Any other
// animated property (mask-position, background-position, clip-path, filter,
// box-shadow, colors, sizes) makes the main thread restyle, paint and commit
// the whole window on every frame. Running-session indicators did exactly
// that and kept the workspace window's main thread ~85% busy (2026-10).
const COMPOSITED_PROPERTIES = new Set(["transform", "opacity", "translate", "rotate", "scale"]);

// Finite one-shot animations may animate other properties; each entry needs a reason.
const ALLOWED_MAIN_THREAD_KEYFRAMES = new Map<string, string>([
  ["conflict-pulse", "two-iteration highlight of a merge conflict"],
  ["workspace-tree-tab-attention-a", "single attention flash on a tree tab"],
  ["workspace-tree-tab-attention-b", "single attention flash on a tree tab"],
]);

const SRC = resolve(__dirname, "..");

function styleSources(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) return entry.name === "__tests__" ? [] : styleSources(path);
    return /\.(vue|css|scss)$/.test(entry.name) ? [path] : [];
  });
}

function keyframeBlocks(source: string): Array<{ name: string; body: string }> {
  const blocks: Array<{ name: string; body: string }> = [];
  const pattern = /@keyframes\s+([\w-]+)\s*\{/g;
  for (let match = pattern.exec(source); match; match = pattern.exec(source)) {
    let depth = 1;
    let index = pattern.lastIndex;
    while (depth > 0 && index < source.length) {
      if (source[index] === "{") depth += 1;
      else if (source[index] === "}") depth -= 1;
      index += 1;
    }
    blocks.push({ name: match[1]!, body: source.slice(pattern.lastIndex, index - 1) });
  }
  return blocks;
}

function animatedProperties(body: string): string[] {
  const properties = new Set<string>();
  for (const match of body.matchAll(/(?:^|[;{\s])(-{0,2}[a-z][a-z-]*)\s*:/g)) properties.add(match[1]!);
  return [...properties];
}

describe("CSS keyframes", () => {
  it("animate only compositor properties unless explicitly allowed", () => {
    const violations: string[] = [];
    for (const file of styleSources(SRC)) {
      for (const { name, body } of keyframeBlocks(readFileSync(file, "utf8"))) {
        if (ALLOWED_MAIN_THREAD_KEYFRAMES.has(name)) continue;
        const mainThread = animatedProperties(body).filter((property) => !COMPOSITED_PROPERTIES.has(property));
        if (mainThread.length) violations.push(`${relative(SRC, file)}: @keyframes ${name} animates ${mainThread.join(", ")}`);
      }
    }
    expect(violations).toEqual([]);
  });

  it("keeps the allowlist current", () => {
    const names = new Set(styleSources(SRC).flatMap((file) => keyframeBlocks(readFileSync(file, "utf8")).map(({ name }) => name)));
    expect([...ALLOWED_MAIN_THREAD_KEYFRAMES.keys()].filter((name) => !names.has(name))).toEqual([]);
  });
});
