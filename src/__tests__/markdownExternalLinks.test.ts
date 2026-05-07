import { describe, expect, it } from "vitest";
import { normalizeExternalMarkdownHref } from "../composables/markdownExternalLinks";

describe("markdownExternalLinks", () => {
  it("keeps explicit web links for external opening", () => {
    expect(normalizeExternalMarkdownHref("https://github.com/yasirkula/UnityRuntimeInspector")).toBe(
      "https://github.com/yasirkula/UnityRuntimeInspector",
    );
    expect(normalizeExternalMarkdownHref(" http://example.com/docs ")).toBe(
      "http://example.com/docs",
    );
  });

  it("normalizes protocol-relative web links to https", () => {
    expect(normalizeExternalMarkdownHref("//github.com/org/repo")).toBe(
      "https://github.com/org/repo",
    );
  });

  it("allows external app protocols handled by the OS", () => {
    expect(normalizeExternalMarkdownHref("mailto:team@example.com")).toBe(
      "mailto:team@example.com",
    );
    expect(normalizeExternalMarkdownHref("tel:+15551234567")).toBe("tel:+15551234567");
  });

  it("blocks internal, relative, and unsafe hrefs from WebView navigation", () => {
    expect(normalizeExternalMarkdownHref("#section")).toBeNull();
    expect(normalizeExternalMarkdownHref("/docs/intro")).toBeNull();
    expect(normalizeExternalMarkdownHref("docs/intro.md")).toBeNull();
    expect(normalizeExternalMarkdownHref("javascript:alert(1)")).toBeNull();
    expect(normalizeExternalMarkdownHref("file:///C:/Windows/System32/drivers/etc/hosts")).toBeNull();
  });
});
