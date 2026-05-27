<script setup lang="ts">
import { computed } from "vue";
import {
  normalizeSemanticCodeForDisplay,
  renderSemanticCodeHtml,
  type SemanticCodeLanguage,
} from "../../composables/semanticCodeRendering";

const props = defineProps<{
  content: string;
  language: SemanticCodeLanguage;
  highlightTerms?: string[];
}>();

const languageClass = computed(() => `language-${props.language}`);
const renderedHtml = computed(() => {
  const html = renderSemanticCodeHtml(props.content, props.language);
  return highlightCodeHtml(html, normalizeHighlightTerms(props.highlightTerms));
});
const parseError = computed(() =>
  normalizeSemanticCodeForDisplay(props.content, props.language).parseError
);

function escapeRegExp(source: string): string {
  return source.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function normalizeHighlightTerms(terms?: string[]): string[] {
  if (!terms?.length) return [];
  return [...new Set(
    terms
      .map((value) => value.trim())
      .filter(Boolean),
  )].sort((left, right) => right.length - left.length);
}

function highlightCodeHtml(html: string, terms: string[]): string {
  if (!html || !terms.length || typeof DOMParser === "undefined") return html;
  const regex = new RegExp(`(${terms.map(escapeRegExp).join("|")})`, "gi");
  const parser = new DOMParser();
  const doc = parser.parseFromString(`<body>${html}</body>`, "text/html");
  const root = doc.body;
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (!(node instanceof Text)) return NodeFilter.FILTER_REJECT;
      if (!node.nodeValue?.trim()) return NodeFilter.FILTER_REJECT;
      if (node.parentElement?.closest("mark.semantic-code-search-mark")) {
        return NodeFilter.FILTER_REJECT;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });

  const textNodes: Text[] = [];
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (node instanceof Text) textNodes.push(node);
  }

  for (const textNode of textNodes) {
    const value = textNode.nodeValue ?? "";
    regex.lastIndex = 0;
    let match: RegExpExecArray | null;
    let lastIndex = 0;
    let hasMatch = false;
    const fragment = doc.createDocumentFragment();

    while ((match = regex.exec(value)) !== null) {
      hasMatch = true;
      if (match.index > lastIndex) {
        fragment.append(doc.createTextNode(value.slice(lastIndex, match.index)));
      }
      const mark = doc.createElement("mark");
      mark.className = "semantic-code-search-mark markdown-search-mark";
      mark.textContent = match[0];
      fragment.append(mark);
      lastIndex = match.index + match[0].length;
      if (match[0].length === 0) regex.lastIndex += 1;
    }

    if (!hasMatch) continue;
    if (lastIndex < value.length) {
      fragment.append(doc.createTextNode(value.slice(lastIndex)));
    }
    textNode.parentNode?.replaceChild(fragment, textNode);
  }

  return root.innerHTML;
}
</script>

<template>
  <div class="semantic-code-renderer" :data-parse-error="parseError || undefined">
    <pre class="semantic-code-pre hljs" :class="languageClass"><code v-html="renderedHtml"></code></pre>
  </div>
</template>

<style scoped>
.semantic-code-renderer {
  min-width: 0;
  min-height: 100%;
  color: var(--text-color);
}

.semantic-code-pre.hljs {
  min-height: 100%;
  margin: 0;
  overflow-x: auto;
  border: none;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
}

.semantic-code-pre code {
  display: block;
  min-width: max-content;
  padding: 10px 0;
  border: none;
  background: transparent;
  color: inherit;
  font-family: var(--font-mono-block);
  font-size: 13px;
  line-height: 1.55;
  white-space: pre;
}

.semantic-code-pre :deep(.code-line) {
  display: grid;
  grid-template-columns: 46px minmax(0, 1fr);
  align-items: start;
  min-width: 100%;
}

.semantic-code-pre :deep(.line-number) {
  display: block;
  padding: 0 10px 0 0;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  color: color-mix(in srgb, var(--text-secondary) 78%, transparent);
  font-size: 11px;
  text-align: right;
  user-select: none;
  opacity: 0.5;
}

.semantic-code-pre :deep(.line-content) {
  display: block;
  min-width: 0;
  padding: 0 14px 0 12px;
}

.semantic-code-pre :deep(.semantic-code-search-mark) {
  padding: 0 2px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--accent-color) 22%, var(--hover-bg));
  color: var(--text-color);
}
</style>
