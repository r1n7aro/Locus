import { describe, expect, it } from "vitest";
import { markdownEngine } from "../composables/markdownEngine";
import {
  ensureMathRendererLoaded,
  mathRenderCacheSizeForTest,
  resolveMathSentinels,
  stripMathSentinelChars,
} from "../composables/markdownMath";
import { normalizeMarkdownForRender } from "../composables/markdownRender";
import { StreamingMarkdownSplitter } from "../composables/streamingMarkdownBlocks";

const SENTINEL_OPEN = "\uE000";
const SENTINEL_CLOSE = "\uE001";

/** Mirrors MarkdownRenderer's pipeline shape: normalize -> parse -> resolve.
 * The document DOMPurify pass sits between parse and resolve in production;
 * it has no DOM in this environment, and the sentinel contract is exactly
 * that it passes through serialization untouched. */
function renderMarkdown(source: string): string {
  return resolveMathSentinels(
    markdownEngine.parse(normalizeMarkdownForRender(source)) as string,
  );
}

describe("markdownMath", () => {
  it("renders a pending fallback before KaTeX loads, then upgrades", async () => {
    const sentinel = `${SENTINEL_OPEN}i:${encodeURIComponent("x+1")}${SENTINEL_CLOSE}`;
    const pending = resolveMathSentinels(`<p>${sentinel}</p>`);
    // First touch races the lazy import kicked off above; both outcomes are
    // valid, but whichever path ran must not leak the sentinel.
    expect(pending).not.toContain(SENTINEL_OPEN);
    expect(pending).toMatch(/md-math-pending|katex/);

    await ensureMathRendererLoaded();
    const resolved = resolveMathSentinels(`<p>${sentinel}</p>`);
    expect(resolved).toContain('class="katex"');
    expect(resolved).not.toContain("md-math-pending");
  });

  it("renders all four delimiter forms", async () => {
    await ensureMathRendererLoaded();

    const blockDollar = renderMarkdown("$$\ne^{i\\pi} + 1 = 0\n$$");
    expect(blockDollar).toContain("katex-display");

    const blockBracket = renderMarkdown("\\[\n\\zeta(s) = \\sum_{n=1}^{\\infty} \\frac{1}{n^s}\n\\]");
    expect(blockBracket).toContain("katex-display");

    const inlineDollar = renderMarkdown("质量$m$与速度");
    expect(inlineDollar).toContain('class="katex"');
    expect(inlineDollar).not.toContain("katex-display");

    const inlineParen = renderMarkdown("能量 \\(E=mc^2\\) 守恒");
    expect(inlineParen).toContain('class="katex"');
    expect(inlineParen).not.toContain("katex-display");
  });

  it("renders $$...$$ and \\[...\\] inside a paragraph as display math", async () => {
    await ensureMathRendererLoaded();
    const midParagraph = renderMarkdown("如下:$$a^2+b^2=c^2$$ 证毕。");
    expect(midParagraph).toContain("katex-display");

    const bracketInline = renderMarkdown("公式 \\[x=1\\] 成立。");
    expect(bracketInline).toContain("katex-display");
  });

  it("renders block math whose body spans blank lines (post-stream correction shape)", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdown("$$\na = b\n\nc = d\n$$");
    expect(html).toContain("katex");
    expect(html).not.toContain("$$");
  });

  it("renders math nested in blockquotes and list items", async () => {
    await ensureMathRendererLoaded();
    const quoted = renderMarkdown("> $$\n> x = 1\n> $$");
    expect(quoted).toContain("katex-display");

    const listed = renderMarkdown("- 速度 $v$ 恒定\n- 加速度 \\(a\\) 为零");
    expect(listed.match(/class="katex"/g)?.length).toBe(2);
  });

  it("leaves unclosed block math as literal text while streaming", () => {
    const html = renderMarkdown("$$\n\\zeta(s) = \\sum");
    expect(html).not.toContain("katex");
    expect(html).toContain("$$");
    expect(html).toContain("\\zeta(s)");
  });

  it("never renders math inside code fences or inline code", () => {
    const fenced = renderMarkdown("```\n$$ x $$\n\\(y\\)\n```");
    expect(fenced).not.toContain("katex");
    expect(fenced).toContain("$$ x $$");

    const inlineCode = renderMarkdown("正则 `\\(foo\\)` 与价格 `$x$` 都是代码");
    expect(inlineCode).not.toContain("katex");

    const indented = renderMarkdown("段落。\n\n    $$ x $$\n");
    expect(indented).not.toContain("katex");
  });

  it("does not mistake currency or escapes for math", () => {
    expect(renderMarkdown("价格 $5 和 $10 而已")).not.toContain("katex");
    expect(renderMarkdown("价格$5和$10而已")).not.toContain("katex");
    expect(renderMarkdown("about $20,000 and $30,000 total")).not.toContain("katex");
    expect(renderMarkdown("转义 \\$x\\$ 不是公式")).not.toContain("katex");
    expect(renderMarkdown("$$ $$")).not.toContain("katex");
    expect(renderMarkdown("字面反斜杠 \\\\(x\\\\) 不是公式")).not.toContain("katex");
  });

  it("renders digit-led and CJK-adjacent formulas (game-context tuning)", async () => {
    await ensureMathRendererLoaded();
    expect(renderMarkdown("伤害 $2x$ 翻倍")).toContain('class="katex"');
    expect(renderMarkdown("圆周率 $3.14$ 也是公式")).toContain('class="katex"');
    expect(renderMarkdown("正文夹字 $速度v$ 也渲染")).toContain('class="katex"');
    expect(renderMarkdown("质量$m$与速度")).toContain('class="katex"');
    expect(renderMarkdown("那么$O(n \\log n)$成立")).toContain('class="katex"');
    // Accepted collateral of the game-context tuning: two amounts in one
    // sentence can fuse into a formula. `$`-as-currency is near-absent in
    // this product's chats, and the structural guards above still keep the
    // common money shapes (spaces, digit-after-closer) as text.
    expect(renderMarkdown("共$300万美元和$x的关系")).toContain('class="katex"');
  });

  it("never lets an unpaired or distant $ swallow surrounding text", () => {
    // Unclosed $ matches nothing: while streaming, everything after it
    // stays literal until the closer actually arrives.
    const unclosed = renderMarkdown("战力提升 $2x 之后的整段说明都不该消失");
    expect(unclosed).not.toContain("katex");
    expect(unclosed).toContain("$2x");
    expect(unclosed).toContain("整段说明都不该消失");
    // Paired but too far apart: the body-length cap refuses the fusion.
    const far = renderMarkdown(`左 $${"a".repeat(150)}$ 右`);
    expect(far).not.toContain("katex");
  });

  it("keeps CommonMark bracket escapes as text unless the body signals math", async () => {
    await ensureMathRendererLoaded();
    expect(renderMarkdown("参见\\[1\\]文献")).not.toContain("katex");
    expect(renderMarkdown("对比\\[1,2\\]与\\[Smith 2020\\]")).not.toContain("katex");
    expect(renderMarkdown("单变量\\[x\\]也当作转义")).not.toContain("katex");
    expect(renderMarkdown("公式 \\[E=mc^2\\] 成立")).toContain("katex-display");
    expect(renderMarkdown("命令 \\[\\alpha\\] 成立")).toContain("katex-display");
    // Block-level \[ on its own lines is unambiguous math, no signal needed.
    expect(renderMarkdown("\\[\n42\n\\]")).toContain("katex-display");
  });

  it("degrades a lone-surrogate formula without dropping the message", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdown(`孤代理 $a${"\uD800"}b$ 之后 **加粗** 仍是富文本`);
    expect(html).toContain("<strong>");
    expect(html).not.toContain("katex");
  });

  it("keeps the render cache bounded under eviction pressure", async () => {
    await ensureMathRendererLoaded();
    for (let index = 0; index < 2100; index += 1) {
      resolveMathSentinels(
        `<p>${SENTINEL_OPEN}i:${encodeURIComponent(`c_{${index}}`)}${SENTINEL_CLOSE}</p>`,
      );
    }
    expect(mathRenderCacheSizeForTest()).toBeLessThanOrEqual(2048);
  });

  it("strips sentinel code points from source so sentinels cannot be forged", () => {
    const forged = `${SENTINEL_OPEN}d:${encodeURIComponent("\\href{javascript:1}{x}")}${SENTINEL_CLOSE}`;
    const html = renderMarkdown(`正文 ${forged} 继续`);
    expect(html).not.toContain("katex");
    expect(html).not.toContain("md-math-error");
    expect(html).not.toContain(SENTINEL_OPEN);
    expect(html).not.toContain(SENTINEL_CLOSE);
    // The de-fanged payload must survive as visible literal text — if the
    // strip were ever disabled, the payload would render instead.
    expect(html).toContain("d:");

    expect(stripMathSentinelChars(`a${SENTINEL_OPEN}b${SENTINEL_CLOSE}c`)).toBe("abc");
    const clean = "no sentinels here";
    expect(stripMathSentinelChars(clean)).toBe(clean);
  });

  it("drops sentinels with undecodable payloads instead of throwing", async () => {
    await ensureMathRendererLoaded();
    const html = resolveMathSentinels(`<p>${SENTINEL_OPEN}i:%zz${SENTINEL_CLOSE}</p>`);
    expect(html).toBe("<p></p>");
  });

  it("renders parse errors inline without throwing", async () => {
    await ensureMathRendererLoaded();
    const html = renderMarkdown("$\\frac{$");
    // KaTeX throwOnError:false yields .katex-error; our own catch yields
    // .md-math-error. Either way the round survives.
    expect(html).toMatch(/katex-error|md-math-error|\$/);
  });

  it("returns sentinel-free HTML by reference on the fast path", () => {
    const html = "<p>no math at all</p>";
    expect(resolveMathSentinels(html)).toBe(html);
  });

  it("caches rendered formulas across re-renders", async () => {
    await ensureMathRendererLoaded();
    const first = renderMarkdown("缓存 $q_{unique}^{42}$ 键");
    const sizeAfterFirst = mathRenderCacheSizeForTest();
    const second = renderMarkdown("缓存 $q_{unique}^{42}$ 键");
    // A hit adds no entry and returns the identical cached string, even when
    // the eviction-pressure test above already filled the cache to its cap.
    expect(mathRenderCacheSizeForTest()).toBe(sizeAfterFirst);
    expect(second).toBe(first);
  });

  it("keeps a streaming $$ block in one splitter unit (no interior blank line)", () => {
    const splitter = new StreamingMarkdownSplitter();
    splitter.append("前文段落。\n\n");
    splitter.append("$$\n");
    splitter.append("\\zeta(s) = \\sum_{n=1}^{\\infty} \\frac{1}{n^s}\n");
    splitter.append("$$\n");
    splitter.append("\n后续段落。\n\n");
    const split = splitter.append("再一段,推动公式块冻结。\n\n收尾。\n");

    const units = [...split.blocks.map((block) => block.text), split.tail];
    const withOpener = units.filter((text) => text.includes("$$"));
    expect(withOpener.length).toBe(1);
    expect(withOpener[0]).toContain("$$\n\\zeta(s)");
    expect(withOpener[0]?.match(/\$\$/g)?.length).toBe(2);
  });
});
