import { describe, expect, it } from "vitest";
import {
  buildViewPreviewSrcdoc,
  extractVueTemplate,
  sanitizeCssForPreview,
} from "../components/view/viewHostPreview";
import type { ViewPackageDetail } from "../services/view";

describe("viewHostPreview", () => {
  it("extracts the Vue template body", () => {
    expect(extractVueTemplate("<template><main>View</main></template>")).toBe("<main>View</main>");
  });

  it("removes script blocks and css urls from preview srcdoc", () => {
    const detail: ViewPackageDetail = {
      summary: {
        id: "test-view",
        name: "Test View",
        version: "0.1.0",
        template: "blank",
        packageRoot: "F:/Project/Locus/views/test-view",
        manifestPath: "F:/Project/Locus/views/test-view/view.json",
        updatedAt: 1,
        capabilities: { unity: false, bindings: false, writeBack: false },
      },
      manifest: {
        schema: "locus.view.v1",
        id: "test-view",
        name: "Test View",
        version: "0.1.0",
        template: "blank",
        entry: "src/main.ts",
        style: "src/style.css",
        bindings: "bindings.json",
        scripts: [],
        capabilities: { unity: false, bindings: false, writeBack: false },
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
    expect(srcdoc).toContain("script-src 'unsafe-inline'");
    expect(srcdoc).toContain("data-locus-view-runtime");
    expect(srcdoc).toContain("initNodeGraph");
    expect(srcdoc).not.toContain("<script>alert(1)</script>");
    expect(srcdoc).not.toContain("onclick");
    expect(srcdoc).not.toContain("onmouseover");
    expect(srcdoc).not.toContain("return false");
    expect(srcdoc).not.toContain("@import");
    expect(srcdoc).not.toContain("https://example.com");
  });

  it("injects controlled runtime behavior for graph templates", () => {
    const detail: ViewPackageDetail = {
      summary: {
        id: "graph-view",
        name: "Graph View",
        version: "0.1.0",
        template: "node-graph",
        packageRoot: "F:/Project/Locus/views/graph-view",
        manifestPath: "F:/Project/Locus/views/graph-view/view.json",
        updatedAt: 1,
        capabilities: { unity: false, bindings: false, writeBack: false },
      },
      manifest: {
        schema: "locus.view.v1",
        id: "graph-view",
        name: "Graph View",
        version: "0.1.0",
        template: "node-graph",
        entry: "src/main.ts",
        style: "src/style.css",
        bindings: "bindings.json",
        scripts: [],
        capabilities: { unity: false, bindings: false, writeBack: false },
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
    expect(srcdoc).toContain("data-locus-view-runtime");
    expect(srcdoc).toContain("data-node-id");
  });

  it("sanitizes css url references", () => {
    expect(sanitizeCssForPreview("@import \"x.css\";.x{background:url(test.png)}")).toBe(".x{background:none}");
  });
});
