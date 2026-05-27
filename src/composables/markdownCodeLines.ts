type OpenHtmlTag = {
  name: string;
  html: string;
};

const HTML_TOKEN_RE = /<\/?[a-z][^>]*>|\n|[^<\n]+|./gi;
const OPEN_TAG_RE = /^<([a-z][\w:-]*)(?:\s[^>]*)?>$/i;
const CLOSE_TAG_RE = /^<\/([a-z][\w:-]*)>$/i;
const SELF_CLOSING_TAG_RE = /^<([a-z][\w:-]*)(?:\s[^>]*)?\/>$/i;
const VOID_TAGS = new Set([
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
]);

function closingHtmlForOpenTags(openTags: OpenHtmlTag[]): string {
  return openTags
    .slice()
    .reverse()
    .map((tag) => `</${tag.name}>`)
    .join("");
}

function openingHtmlForOpenTags(openTags: OpenHtmlTag[]): string {
  return openTags.map((tag) => tag.html).join("");
}

function popClosingTag(openTags: OpenHtmlTag[], tagName: string): void {
  const normalized = tagName.toLowerCase();
  for (let i = openTags.length - 1; i >= 0; i -= 1) {
    if (openTags[i].name === normalized) {
      openTags.splice(i, 1);
      return;
    }
  }
}

export function splitHighlightedHtmlLines(source: string): string[] {
  const tokens = source.match(HTML_TOKEN_RE) ?? [];
  const lines: string[] = [];
  const openTags: OpenHtmlTag[] = [];
  let currentLine = "";

  for (const token of tokens) {
    if (token === "\n") {
      lines.push(currentLine + closingHtmlForOpenTags(openTags));
      currentLine = openingHtmlForOpenTags(openTags);
      continue;
    }

    currentLine += token;

    const closeMatch = token.match(CLOSE_TAG_RE);
    if (closeMatch) {
      popClosingTag(openTags, closeMatch[1]);
      continue;
    }

    if (SELF_CLOSING_TAG_RE.test(token)) {
      continue;
    }

    const openMatch = token.match(OPEN_TAG_RE);
    if (openMatch) {
      const name = openMatch[1].toLowerCase();
      if (!VOID_TAGS.has(name)) {
        openTags.push({ name, html: token });
      }
    }
  }

  lines.push(currentLine + closingHtmlForOpenTags(openTags));
  return lines;
}

export function renderHighlightedCodeLines(source: string, showLineNumbers = true): string {
  const lines = splitHighlightedHtmlLines(source);
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines
    .map((line, i) => (
      showLineNumbers
        ? `<span class="code-line"><span class="line-number">${i + 1}</span><span class="line-content">${line || " "}</span></span>`
        : `<span class="code-line code-line-tree"><span class="line-content">${line || " "}</span></span>`
    ))
    .join("");
}
