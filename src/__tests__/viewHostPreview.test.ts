import { describe, expect, it } from "vitest";
import {
  buildViewPreviewSrcdoc,
  extractVueTemplate,
  sanitizeCssForPreview,
  viewPackageRelPath,
} from "../components/view/viewHostPreview";
import type { ViewPackageDetail } from "../services/view";

describe("viewHostPreview", () => {
  it("extracts the Vue template body", () => {
    expect(extractVueTemplate("<template><main>View</main></template>")).toBe("<main>View</main>");
  });

  it("extracts the full Vue template body when nested template tags are used", () => {
    const source = `<script setup lang="ts">
const mode = "group";
</script>

<template>
  <main class="view-shell">
    <section>
      <template v-if="mode === 'group'">
        <div>Group</div>
      </template>
      <template v-else>
        <div>Other</div>
      </template>
    </section>
  </main>
</template>`;

    expect(extractVueTemplate(source)).toBe(`<main class="view-shell">
    <section>
      <template v-if="mode === 'group'">
        <div>Group</div>
      </template>
      <template v-else>
        <div>Other</div>
      </template>
    </section>
  </main>`);
  });

  it("removes script blocks and css urls from preview srcdoc", () => {
    const detail: ViewPackageDetail = {
      summary: {
        id: "test-view",
        name: "Test View",
        version: "0.1.0",
        template: "blank",
        displayPath: "Project/test-view",
        packageRelPath: "Project/test-view",
        packageRoot: "F:/Project/Locus/View/Project/test-view",
        manifestPath: "F:/Project/Locus/View/Project/test-view/view.json",
        updatedAt: 1,
        apiVersion: "v0.3.0",
        capabilities: { unity: false },
        requirements: { unityConnection: false },
      },
      manifest: {
        schema: "locus.view.v1",
        apiVersion: "v0.3.0",
        id: "test-view",
        name: "Test View",
        version: "0.1.0",
        template: "blank",
        entry: "src/main.ts",
        style: "src/style.css",
        scripts: [],
        capabilities: { unity: false },
        requirements: { unityConnection: false },
      },
      files: [
        {
          relPath: "src/App.vue",
          kind: "source",
          content: "<template><main onclick=\"alert(1); return false\" onmouseover=alert(2)>View<script>alert(1)</script></main></template>",
          size: 1,
          truncated: false,
        },
        {
          relPath: "src/style.css",
          kind: "style",
          content: "@import \"https://example.com/a.css\";body{background:url(https://example.com/a.png)}",
          size: 1,
          truncated: false,
        },
      ],
    };

    const srcdoc = buildViewPreviewSrcdoc(detail);

    expect(srcdoc).toContain("<main>View</main>");
    expect(srcdoc).toContain("Content-Security-Policy");
    expect(srcdoc).toContain("body class=\"locus-view-runtime\"");
    expect(srcdoc).toContain("--bg-color");
    expect(srcdoc).toContain(".locus-button");
    expect(srcdoc).toContain(".view-panel");
    expect(srcdoc).not.toContain("script-src");
    expect(srcdoc).not.toContain("<script");
    expect(srcdoc).not.toContain("onclick");
    expect(srcdoc).not.toContain("onmouseover");
    expect(srcdoc).not.toContain("return false");
    expect(srcdoc).not.toContain("@import");
    expect(srcdoc).not.toContain("https://example.com");
  });

  it("keeps template previews static without injected runtime scripts", () => {
    const detail: ViewPackageDetail = {
      summary: {
        id: "graph-view",
        name: "Graph View",
        version: "0.1.0",
        template: "node-graph",
        displayPath: "Project/graph-view",
        packageRelPath: "Project/graph-view",
        packageRoot: "F:/Project/Locus/View/Project/graph-view",
        manifestPath: "F:/Project/Locus/View/Project/graph-view/view.json",
        updatedAt: 1,
        apiVersion: "v0.3.0",
        capabilities: { unity: false },
        requirements: { unityConnection: false },
      },
      manifest: {
        schema: "locus.view.v1",
        apiVersion: "v0.3.0",
        id: "graph-view",
        name: "Graph View",
        version: "0.1.0",
        template: "node-graph",
        entry: "src/main.ts",
        style: "src/style.css",
        scripts: [],
        capabilities: { unity: false },
        requirements: { unityConnection: false },
      },
      files: [
        {
          relPath: "src/App.vue",
          kind: "source",
          content:
            '<template><main data-locus-template="node-graph"><section data-graph-canvas><svg data-graph-edges></svg><button data-node-id="a"></button><button data-node-id="b"></button></section><pre data-graph-output></pre></main></template>',
          size: 1,
          truncated: false,
        },
        {
          relPath: "src/style.css",
          kind: "style",
          content: ".graph-canvas{height:300px}",
          size: 1,
          truncated: false,
        },
      ],
    };

    const srcdoc = buildViewPreviewSrcdoc(detail);

    expect(srcdoc).toContain('data-locus-template="node-graph"');
    expect(srcdoc).toContain("data-node-id");
    expect(srcdoc).not.toContain("data-locus-view-runtime");
    expect(srcdoc).not.toContain("<script");
  });

  it("sanitizes css url references", () => {
    expect(sanitizeCssForPreview("@import \"x.css\";.x{background:url(test.png)}")).toBe(".x{background:none}");
  });

  it("resolves View files relative to the package workspace view folder", () => {
    const detail: ViewPackageDetail = {
      summary: {
        id: "material-inspector",
        name: "Material Inspector",
        version: "0.1.0",
        template: "blank",
        displayPath: "Gameplay/material-inspector",
        packageRelPath: "Gameplay/material-inspector",
        packageRoot: "F:/Project/Locus/View/Gameplay/material-inspector",
        manifestPath: "F:/Project/Locus/View/Gameplay/material-inspector/view.json",
        updatedAt: 1,
        apiVersion: "v0.3.0",
        capabilities: { unity: false },
        requirements: { unityConnection: false },
      },
      manifest: {
        schema: "locus.view.v1",
        apiVersion: "v0.3.0",
        id: "material-inspector",
        name: "Material Inspector",
        version: "0.1.0",
        template: "blank",
        entry: "src/main.ts",
        style: "src/style.css",
        scripts: [],
        capabilities: { unity: false },
        requirements: { unityConnection: false },
      },
      files: [
        {
          relPath: "Gameplay/material-inspector/src/App.vue",
          kind: "source",
          content: "<template><main /></template>",
          size: 1,
          truncated: false,
        },
        {
          relPath: "Gameplay/src/index.ts",
          kind: "source",
          content: "export {};",
          size: 1,
          truncated: false,
        },
      ],
    };

    expect(viewPackageRelPath(detail, "src/App.vue")).toBe("Gameplay/material-inspector/src/App.vue");
    expect(viewPackageRelPath(detail, "Gameplay/src/index.ts")).toBe("Gameplay/src/index.ts");
  });
});
