// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import {
  headingSlug,
  knowledgeTarget,
  normalizePath,
  referenceFromElement,
  relativeToRoot,
  resolveDocumentLink,
} from "../mobile/references";
import type { DocumentSource } from "../mobile/routes";

const workspaceRef = { checkoutId: "c1", expectedGeneration: 1, expectedMaterializationEpoch: null };
const file = (path: string): DocumentSource => ({ kind: "file", workspaceRef, path });
const knowledge = (path: string): DocumentSource => ({ kind: "knowledge", workspaceRef, docType: "design", path });

function element(html: string): Element {
  const host = document.createElement("div");
  host.innerHTML = html;
  return host.querySelector("[data-target]") ?? host.firstElementChild!;
}

describe("mobile references", () => {
  it("normalizes paths without escaping above absolute roots", () => {
    expect(normalizePath("docs/./a/../b.md")).toBe("docs/b.md");
    expect(normalizePath("../x")).toBe("../x");
    expect(normalizePath("C:\\repo\\..\\..\\x")).toBe("C:/x");
    expect(normalizePath("/a/b/../../../c")).toBe("/c");
  });

  it("recognizes knowledge documents like the desktop", () => {
    expect(knowledgeTarget("Locus/knowledge/plan/a/b.md")).toEqual({ docType: "plan", path: "plan/a/b.md" });
    expect(knowledgeTarget("Locus/knowledge/plan/table.csv")).toEqual({ docType: "plan", path: "plan/table.csv" });
    expect(knowledgeTarget("design/x.md")).toEqual({ docType: "design", path: "design/x.md" });
    expect(knowledgeTarget("design/x.csv")).toBeNull();
    expect(knowledgeTarget("src/x.md")).toBeNull();
  });

  it("reads reference markup", () => {
    expect(referenceFromElement(element('<span class="md-file-ref" data-file-path="src\\main.ts"><b data-target>x</b></span>')))
      .toEqual({ kind: "file", path: "src/main.ts" });
    expect(referenceFromElement(element('<span class="md-file-ref" data-file-path="src/" data-entry-kind="folder">src</span>')))
      .toEqual({ kind: "folder", path: "src" });
    expect(referenceFromElement(element('<span class="md-workspace-ref" data-workspace-path="Locus/knowledge/memory/m.md">m</span>')))
      .toEqual({ kind: "knowledge", docType: "memory", path: "memory/m.md" });
    expect(referenceFromElement(element('<span class="md-knowledge-ref" data-knowledge-path="a/b.md" data-knowledge-type="skill">b</span>')))
      .toEqual({ kind: "knowledge", docType: "skill", path: "a/b.md" });
    expect(referenceFromElement(element('<span class="md-asset-chip" data-asset-path="Assets/Player.prefab">p</span>')))
      .toEqual({ kind: "file", path: "Assets/Player.prefab" });
    expect(referenceFromElement(element("<span>plain</span>"))).toBeNull();
  });

  it("resolves links inside workspace files", () => {
    expect(resolveDocumentLink("#Set-up", file("docs/a.md"))).toEqual({ kind: "anchor", id: "Set-up" });
    expect(resolveDocumentLink("https://example.com", file("docs/a.md"))).toEqual({ kind: "external" });
    expect(resolveDocumentLink("./b.md#part", file("docs/guide/a.md"))).toEqual({ kind: "file", path: "docs/guide/b.md" });
    expect(resolveDocumentLink("../img/x%20y.png", file("docs/a.md"))).toEqual({ kind: "file", path: "img/x y.png" });
    expect(resolveDocumentLink("/README.md", file("docs/a.md"))).toEqual({ kind: "file", path: "README.md" });
    expect(resolveDocumentLink("sub/", file("docs/a.md"))).toEqual({ kind: "folder", path: "docs/sub" });
    expect(resolveDocumentLink("../Locus/knowledge/plan/p.md", file("docs/a.md")))
      .toEqual({ kind: "knowledge", docType: "plan", path: "plan/p.md" });
    expect(resolveDocumentLink("b.md", file("C:/repo/docs/a.md"))).toEqual({ kind: "file", path: "C:/repo/docs/b.md" });
    // Leaving the workspace, and schemes that are not followed.
    expect(resolveDocumentLink("../../x.md", file("docs/a.md"))).toBeNull();
    expect(resolveDocumentLink("file:///C:/Windows/win.ini", file("docs/a.md"))).toBeNull();
    expect(resolveDocumentLink("javascript:alert(1)", file("docs/a.md"))).toBeNull();
  });

  it("resolves links inside knowledge documents", () => {
    expect(resolveDocumentLink("other.md", knowledge("design/sub/a.md")))
      .toEqual({ kind: "knowledge", docType: "design", path: "design/sub/other.md" });
    expect(resolveDocumentLink("../../plan/p.md", knowledge("sub/a.md")))
      .toEqual({ kind: "knowledge", docType: "plan", path: "plan/p.md" });
    expect(resolveDocumentLink("img.png", knowledge("design/a.md")))
      .toEqual({ kind: "file", path: "Locus/knowledge/design/img.png" });
    expect(resolveDocumentLink("../../../src/main.ts", knowledge("design/a.md")))
      .toEqual({ kind: "file", path: "src/main.ts" });
    expect(resolveDocumentLink("../../../../x", knowledge("design/a.md"))).toBeNull();
  });

  it("relativizes paths against a checkout root", () => {
    expect(relativeToRoot("F:\\Repo\\src\\a.ts", "f:/repo")).toBe("src/a.ts");
    expect(relativeToRoot("F:/Repo", "F:/Repo/")).toBe("");
    expect(relativeToRoot("F:/Other/a.ts", "F:/Repo")).toBeNull();
    expect(relativeToRoot("src/a.ts", null)).toBe("src/a.ts");
    expect(relativeToRoot("C:/x", null)).toBeNull();
  });

  it("slugs headings like GitHub", () => {
    expect(headingSlug("Set up & Run")).toBe("set-up--run");
    expect(headingSlug("安装 步骤")).toBe("安装-步骤");
  });
});
