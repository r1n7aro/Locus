import { shallowRef } from "vue";
import DOMPurify, { type Config } from "dompurify";
import type { Tokens, TokenizerAndRendererExtension, MarkedExtension } from "marked";
import type katexNamespace from "katex";

/**
 * Math rendering for every markdown surface. A marked extension recognizes
 * $...$, $$...$$, \(...\), and \[...\] outside code contexts and emits
 * private-use-area sentinels; resolveMathSentinels() expands them into KaTeX
 * HTML after the main DOMPurify pass.
 *
 * Sentinels exist because KaTeX layout depends on inline styles the chat
 * sanitizer forbids, and relaxing that rule (globally or keyed on a class
 * like .katex) would let hand-written HTML in model output smuggle arbitrary
 * styles through. Instead formula HTML never enters the main sanitize pass:
 * the renderer emits U+E000 `{d|i}:{encodeURIComponent(latex)}` U+E001,
 * spelled as escapes in this file because editor tooling has silently
 * dropped literal PUA characters before — inert
 * ASCII between PUA code points that parsers and serializers pass through
 * untouched, and the resolver renders it afterwards. Stripping both code
 * points from the source text (normalizeMarkdownForRender) keeps the marked
 * renderer the only possible producer, so a sentinel cannot be forged.
 *
 * KaTeX output is itself re-sanitized with a formula-scoped DOMPurify config
 * (inline styles allowed) before entering the render cache, so the relaxed
 * rules apply only to HTML KaTeX generated, never to document HTML.
 */

const SENTINEL_OPEN = "\uE000";
const SENTINEL_CLOSE = "\uE001";
const SENTINEL_RE = /\uE000([di]):([^\uE000\uE001]*)\uE001/g;

/** Removes the sentinel delimiter code points from source text. Applied in
 * normalizeMarkdownForRender so document text can never forge a sentinel. */
export function stripMathSentinelChars(source: string): string {
  if (!source.includes(SENTINEL_OPEN) && !source.includes(SENTINEL_CLOSE)) return source;
  return source.replace(/[\uE000\uE001]/g, "");
}

interface MathToken extends Tokens.Generic {
  type: "locusMath";
  raw: string;
  latex: string;
  display: boolean;
}

function mathToken(raw: string, latex: string, display: boolean): MathToken | undefined {
  const trimmed = latex.trim();
  if (!trimmed) return undefined;
  return { type: "locusMath", raw, latex: trimmed, display };
}

function renderSentinel(token: Tokens.Generic): string {
  const math = token as MathToken;
  let encoded: string;
  try {
    encoded = encodeURIComponent(math.latex);
  } catch {
    // Lone surrogates cannot URI-encode; degrade this one formula to escaped
    // literal text instead of letting the whole parse throw and drop the
    // entire message to the plain-text fallback.
    return escapeMathHtml(math.raw);
  }
  const mode = math.display ? "d" : "i";
  return `${SENTINEL_OPEN}${mode}:${encoded}${SENTINEL_CLOSE}`;
}

const BLOCK_DOLLAR_RE = /^ {0,3}\$\$([\s\S]+?)\$\$[ \t]*(?:\n|$)/;
const BLOCK_BRACKET_RE = /^ {0,3}\\\[([\s\S]+?)\\\][ \t]*(?:\n|$)/;
const BLOCK_START_RE = /(?:^|\n)(?= {0,3}(?:\$\$|\\\[))/;

/** Block-level $$...$$ / \[...\]. Fenced code never reaches this tokenizer
 * (the fence tokenizer consumes whole fences), and >=4-space indents fail
 * the {0,3} prefix so indented code stays code. An unclosed opener simply
 * doesn't match and renders as paragraph text until the closer streams in. */
const mathBlockExtension: TokenizerAndRendererExtension = {
  name: "locusMath",
  level: "block",
  start(src: string) {
    const match = BLOCK_START_RE.exec(src);
    if (!match) return undefined;
    return match.index + match[0].length;
  },
  tokenizer(src: string) {
    const dollar = BLOCK_DOLLAR_RE.exec(src);
    if (dollar) return mathToken(dollar[0], dollar[1], true);
    const bracket = BLOCK_BRACKET_RE.exec(src);
    if (bracket) return mathToken(bracket[0], bracket[1], true);
    return undefined;
  },
  renderer(token) {
    return `${renderSentinel(token)}\n`;
  },
};

const INLINE_DOUBLE_DOLLAR_RE = /^\$\$([\s\S]+?)\$\$/;
const INLINE_BRACKET_RE = /^\\\[([\s\S]+?)\\\]/;
const INLINE_PAREN_RE = /^\\\(([\s\S]+?)\\\)/;
/**
 * Single-dollar inline math, tuned for a product where `$` almost always
 * means math: currency is rare in game-development chat, so digit-led
 * ($2x$) and CJK-body ($v$ with CJK) formulas render, and the occasional
 * pair of amounts fusing into one formula is accepted collateral. The
 * remaining guards are structural, not statistical: no whitespace just
 * inside the delimiters, no digit right after the closer (keeps
 * no-space amount pairs as money), no bare `$` or newline in the body,
 * and a body length cap so a lone unpaired `$` can never fuse with a
 * distant one and swallow a paragraph. An unclosed `$` matches nothing
 * and stays literal text, so streaming text renders as-is until the
 * closer arrives.
 */
const INLINE_SINGLE_DOLLAR_RE = /^\$(?!\s)((?:\\.|[^\\\n$])+?)\$(?!\d)/;
const INLINE_SINGLE_DOLLAR_BODY_MAX = 120;
/**
 * `\[...\]` doubles as the CommonMark escape for literal brackets —
 * citations like `参见\[1\]文献` must stay text. Inline occurrences only
 * count as math when the body carries a math-signal character; block-level
 * `\[` on its own line is unambiguous and skips this test.
 */
const MATH_SIGNAL_RE = /[\\^_={}+]/;
const INLINE_START_RE = /\$|\\[([]/;

/** Inline math, tried ahead of built-in tokenizers at each position. Escapes
 * still win: at `\$` or `\\(` the leading backslash fails every pattern here,
 * so marked's escape tokenizer consumes the pair. Inline code wins likewise
 * because a backtick fails every pattern. */
const mathInlineExtension: TokenizerAndRendererExtension = {
  name: "locusMath",
  level: "inline",
  start(src: string) {
    const index = src.search(INLINE_START_RE);
    return index < 0 ? undefined : index;
  },
  tokenizer(src: string) {
    if (src.startsWith("$")) {
      const double = INLINE_DOUBLE_DOLLAR_RE.exec(src);
      if (double) return mathToken(double[0], double[1], true);
      const single = INLINE_SINGLE_DOLLAR_RE.exec(src);
      if (
        single
        && !/\s$/.test(single[1])
        && single[1].length <= INLINE_SINGLE_DOLLAR_BODY_MAX
      ) {
        return mathToken(single[0], single[1], false);
      }
      return undefined;
    }
    if (src.startsWith("\\(")) {
      const paren = INLINE_PAREN_RE.exec(src);
      if (paren) return mathToken(paren[0], paren[1], false);
      return undefined;
    }
    if (src.startsWith("\\[")) {
      const bracket = INLINE_BRACKET_RE.exec(src);
      if (bracket && MATH_SIGNAL_RE.test(bracket[1])) {
        return mathToken(bracket[0], bracket[1], true);
      }
    }
    return undefined;
  },
  renderer: renderSentinel,
};

export const markdownMathExtension: MarkedExtension = {
  extensions: [mathBlockExtension, mathInlineExtension],
};

// -- KaTeX loading and rendering --

type KatexLib = typeof katexNamespace;

let katexModule: KatexLib | null = null;
let katexReadyPromise: Promise<void> | null = null;
/** Bumped once when KaTeX finishes loading. Computeds that rendered the
 * pending fallback read this ref, so they re-evaluate exactly once — frozen
 * streaming blocks included. The resolved path never reads it. */
const katexReadyTick = shallowRef(0);

/** Kick off (or await) the lazy KaTeX load. Exported for tests. */
export function ensureMathRendererLoaded(): Promise<void> {
  if (!katexReadyPromise) {
    katexReadyPromise = (async () => {
      // The stylesheet ships the fonts; its failure (e.g. vitest, where css
      // imports are stubbed out) must not block the renderer itself.
      const css = import("katex/dist/katex.min.css").catch(() => undefined);
      const mod = await import("katex");
      await css;
      katexModule = (mod as { default?: KatexLib }).default ?? (mod as unknown as KatexLib);
      katexReadyTick.value += 1;
    })().catch((error) => {
      katexReadyPromise = null;
      console.warn("Failed to load KaTeX renderer:", error);
    });
  }
  return katexReadyPromise;
}

/**
 * Formula-scoped sanitize pass. KaTeX output is documented as injection-safe
 * with trust unset, but it flows into v-html without the main sanitize pass,
 * so it gets its own: svg allowed (stretchy delimiters), inline styles
 * allowed (KaTeX layout lives in them) — rules the document pass must never
 * adopt. Applied once per cache miss, so the cost amortizes to zero.
 */
const KATEX_SANITIZE_CONFIG: Config = {
  USE_PROFILES: { html: true, svg: true },
  ALLOW_ARIA_ATTR: true,
};

function sanitizeKatexHtml(html: string): string {
  // Without a DOM (bare-node vitest) dompurify's default export is the
  // unbound factory and `sanitize` does not exist; the WebView always has
  // one. KaTeX output is used as emitted there — v-html never runs either.
  if (typeof DOMPurify.sanitize !== "function") return html;
  return DOMPurify.sanitize(html, KATEX_SANITIZE_CONFIG);
}

function escapeMathHtml(source: string): string {
  return source
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/**
 * Rendered-formula LRU. Streaming re-renders the active tail every throttle
 * tick, so without this each tick would re-run KaTeX for every formula in
 * the tail; with it each distinct formula renders once in any realistic
 * session (history re-renders and frozen blocks hit it too). LRU thrashes
 * when the live working set exceeds the cap, so the cap sits far above what
 * a 24k-char streaming tail plus visible history can hold.
 */
const MATH_RENDER_CACHE_CAP = 2048;
const mathRenderCache = new Map<string, string>();

function renderKatexCached(latex: string, display: boolean): string {
  const key = `${display ? "d" : "i"} ${latex}`;
  const cached = mathRenderCache.get(key);
  if (cached !== undefined) {
    mathRenderCache.delete(key);
    mathRenderCache.set(key, cached);
    return cached;
  }

  let html: string;
  try {
    html = sanitizeKatexHtml(
      katexModule!.renderToString(latex, {
        displayMode: display,
        throwOnError: false,
        output: "html",
        strict: "ignore",
        maxExpand: 1000,
        maxSize: 100,
      }),
    );
  } catch {
    html = `<span class="md-math-error">${escapeMathHtml(latex)}</span>`;
  }

  if (mathRenderCache.size >= MATH_RENDER_CACHE_CAP) {
    mathRenderCache.delete(mathRenderCache.keys().next().value!);
  }
  mathRenderCache.set(key, html);
  return html;
}

/** Test-only visibility into the render cache. */
export function mathRenderCacheSizeForTest(): number {
  return mathRenderCache.size;
}

function decodeSentinelPayload(encoded: string): string {
  try {
    return decodeURIComponent(encoded);
  } catch {
    return "";
  }
}

function restoreSentinelSource(value: string): string {
  return value
    .replace(SENTINEL_RE, (_match, _mode, encoded: string) => decodeSentinelPayload(encoded))
    .replace(/[\uE000\uE001]/g, "");
}

let sentinelGuardsInstalled = false;

/**
 * Installed into the document DOMPurify pass by markdownSanitize. Sentinels
 * are only meaningful in text nodes, but marked's image renderer flattens
 * alt text through extension renderers, so a formula inside `![...](...)`
 * lands a sentinel in an attribute value. Expanding it there via string
 * replacement would splice quoted KaTeX markup into the attribute and break
 * out of it — so during the document sanitize pass, sentinels found in
 * attribute values or rawtext containers are restored to their LaTeX source
 * at the DOM layer, where the serializer re-escapes whatever is set.
 */
export function installMathSentinelGuards(purifier: typeof DOMPurify): void {
  if (sentinelGuardsInstalled || typeof purifier.addHook !== "function") return;
  sentinelGuardsInstalled = true;

  purifier.addHook("afterSanitizeAttributes", (node) => {
    if (!node.hasAttributes || !node.hasAttributes()) return;
    for (const attr of Array.from(node.attributes)) {
      if (!attr.value.includes(SENTINEL_OPEN) && !attr.value.includes(SENTINEL_CLOSE)) {
        continue;
      }
      node.setAttribute(attr.name, restoreSentinelSource(attr.value));
    }
  });

  purifier.addHook("afterSanitizeElements", (node) => {
    if (node.nodeName !== "TEXTAREA" && node.nodeName !== "TITLE") return;
    const text = node.textContent;
    if (!text || (!text.includes(SENTINEL_OPEN) && !text.includes(SENTINEL_CLOSE))) return;
    node.textContent = restoreSentinelSource(text);
  });
}

/**
 * Expand math sentinels into KaTeX HTML. Runs after the main sanitize pass
 * (see module doc). Sentinel-free HTML returns on a fast path with zero
 * added work. Before KaTeX loads, formulas render as escaped source in a
 * pending span and the reactive tick re-triggers the caller once it lands.
 */
export function resolveMathSentinels(html: string): string {
  if (!html || !html.includes(SENTINEL_OPEN)) return html;

  if (!katexModule) {
    void ensureMathRendererLoaded();
    // Establish the reactive dependency that re-runs this computed on load.
    void katexReadyTick.value;
    return html.replace(SENTINEL_RE, (_match, _mode, encoded: string) => {
      const latex = decodeSentinelPayload(encoded);
      if (!latex) return "";
      return `<span class="md-math-pending">${escapeMathHtml(latex)}</span>`;
    });
  }

  return html.replace(SENTINEL_RE, (_match, mode: string, encoded: string) => {
    const latex = decodeSentinelPayload(encoded);
    if (!latex) return "";
    return renderKatexCached(latex, mode === "d");
  });
}
