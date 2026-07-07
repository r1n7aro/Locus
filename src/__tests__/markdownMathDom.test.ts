// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { markdownEngine } from "../composables/markdownEngine";
import {
  ensureMathRendererLoaded,
  resolveMathSentinels,
} from "../composables/markdownMath";
import { normalizeMarkdownForRender } from "../composables/markdownRender";
import { sanitizeRenderedMarkdownHtml } from "../composables/markdownSanitize";

const SENTINEL_OPEN = "\uE000";
const SENTINEL_CLOSE = "\uE001";

/** The full MarkdownRenderer pipeline, with the document DOMPurify pass
 * running against a real DOM — the load-bearing invariants live here. */
function renderMarkdownFull(source: string): string {
  return resolveMathSentinels(
    sanitizeRenderedMarkdownHtml(
      markdownEngine.parse(normalizeMarkdownForRender(source)) as string,
    ),
  );
}

function parseFragment(html: string): HTMLElement {
  const host = document.createElement("div");
  host.innerHTML = html;
  return host;
}

describe("markdownMath DOM contracts", () => {
  it("carries sentinels through the real document sanitize pass untouched", () => {
    const sentinel = `${SENTINEL_OPEN}i:${encodeURIComponent("x+1")}${SENTINEL_CLOSE}`;
    const sanitized = sanitizeRenderedMarkdownHtml(`<p>before ${sentinel} after</p>`);
    expect(sanitized).toContain(sentinel);
  });

  it("renders formulas end-to-end through sanitize with inline styles intact", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdownFull("能量 $E=mc^2$ 守恒");
    expect(html).toContain('class="katex"');
    // KaTeX layout styles survive because formula HTML goes through its own
    // formula-scoped pass, not the document pass that forbids style.
    expect(html).toContain("style=");
  });

  it("keeps the document style ban while formulas carry styles", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdownFull(
      '<b style="color:red">plain</b> 与 $x^2$ 同段',
    );
    const host = parseFragment(html);
    expect(host.querySelector("b")?.getAttribute("style")).toBeNull();
    expect(host.querySelector(".katex [style]")).not.toBeNull();
  });

  it("gives hand-written katex markup no exemption from the document pass", () => {
    const html = renderMarkdownFull(
      '<span class="katex" style="position:fixed;top:0">forged</span>',
    );
    expect(html).not.toContain("position:fixed");
    expect(html).not.toContain("style=");
  });

  it("restores formula source inside an image alt instead of splicing markup", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdownFull("![$m$](https://example.com/fig.png)");
    const host = parseFragment(html);
    const img = host.querySelector("img");
    expect(img).not.toBeNull();
    expect(img?.getAttribute("alt")).toBe("m");
    expect(html).not.toContain(SENTINEL_OPEN);
    expect(html).not.toContain(SENTINEL_CLOSE);
    expect(html).not.toContain("katex");
  });

  it("keeps an alt formula and a prose formula independent", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdownFull(
      "图 ![$a_1$](https://example.com/i.png) 与正文 $b_2$ 并存",
    );
    const host = parseFragment(html);
    expect(host.querySelector("img")?.getAttribute("alt")).toBe("a_1");
    expect(host.querySelectorAll(".katex").length).toBe(1);
    expect(html).not.toContain(SENTINEL_OPEN);
  });

  it("restores sentinels inside rawtext containers instead of expanding them", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdownFull("前文 <textarea>$x$</textarea> 后文");
    const host = parseFragment(html);
    const textarea = host.querySelector("textarea");
    if (textarea) {
      expect(textarea.innerHTML).not.toContain("katex");
      expect(textarea.innerHTML).not.toContain(SENTINEL_OPEN);
    }
    // Whether or not the tag survives sanitize policy, nothing may leak.
    expect(html).not.toContain(SENTINEL_OPEN);
  });

  it("still strips scripts on the formula-scoped pass boundary", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdownFull('公式 $x$ 与 <script>alert(1)</script> 同在');
    expect(html).not.toContain("<script");
    expect(html).toContain('class="katex"');
  });
});
