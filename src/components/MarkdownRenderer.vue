
<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";
import hljs from "../hljs";
import {
  markdownImageDirectSrc,
  prepareMarkdownImages,
  shouldResolveMarkdownImageSource,
} from "../composables/markdownImages";
import { renderHighlightedCodeLines } from "../composables/markdownCodeLines";
import { normalizeExternalMarkdownHref } from "../composables/markdownExternalLinks";
import { injectAssetRefs, injectFileRefs, injectWorkspaceMentions } from "../composables/markdownInject";
import { normalizeMarkdownForRender } from "../composables/markdownRender";
import { wrapMarkdownTables } from "../composables/markdownTableHtml";
import {
  armLocusFilePointerDrag,
  armUnityReferencePointerDrag,
  startLocusFileHtmlDrag,
  startUnityReferenceHtmlDrag,
} from "../composables/useUnityReferenceDragSource";
import { resolveMarkdownImage } from "../services/markdownImage";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";
import type { LocusFileDropRef } from "../services/unity";
import type { AssetRefAttachment } from "../types";

const props = defineProps<{
  content: string;
  cursor?: boolean;
  enableFileRefs?: boolean;
  highlightTerms?: string[];
}>();

const emit = defineEmits<{
  (e: "openImage", src: string): void;
}>();

const rootRef = ref<HTMLElement | null>(null);

function escapeHtml(source: string): string {
  return source
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

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

function shouldSkipHighlight(node: Text): boolean {
  let current: HTMLElement | null = node.parentElement;
  while (current) {
    const tagName = current.tagName;
    if (
      tagName === "PRE"
      || tagName === "SCRIPT"
      || tagName === "STYLE"
      || tagName === "TEXTAREA"
    ) {
      return true;
    }
    if (tagName === "MARK" && current.classList.contains("markdown-search-mark")) {
      return true;
    }
    current = current.parentElement;
  }
  return false;
}

function highlightHtml(html: string, terms: string[]): string {
  if (!html || !terms.length || typeof DOMParser === "undefined") return html;
  const regex = new RegExp(`(${terms.map(escapeRegExp).join("|")})`, "gi");
  const parser = new DOMParser();
  const doc = parser.parseFromString(`<body>${html}</body>`, "text/html");
  const root = doc.body;
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (!(node instanceof Text)) return NodeFilter.FILTER_REJECT;
      if (!node.nodeValue?.trim()) return NodeFilter.FILTER_REJECT;
      if (shouldSkipHighlight(node)) return NodeFilter.FILTER_REJECT;
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
      mark.className = "markdown-search-mark";
      mark.textContent = match[0];
      fragment.append(mark);
      lastIndex = match.index + match[0].length;
      if (match[0].length === 0) {
        regex.lastIndex += 1;
      }
    }
    if (!hasMatch) continue;
    if (lastIndex < value.length) {
      fragment.append(doc.createTextNode(value.slice(lastIndex)));
    }
    textNode.parentNode?.replaceChild(fragment, textNode);
  }

  return root.innerHTML;
}

const md = new Marked(
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code: string, lang: string) {
      const normalizedLang = lang.trim().toLowerCase();
      if (normalizedLang === "tree") {
        return renderHighlightedCodeLines(escapeHtml(code), false);
      }

      let highlighted = escapeHtml(code);
      if (normalizedLang && hljs.getLanguage(normalizedLang)) {
        highlighted = hljs.highlight(code, { language: normalizedLang }).value;
      }
      return renderHighlightedCodeLines(highlighted);
    },
  }),
  {
    breaks: true,
    gfm: true,
    hooks: {
      postprocess(html) {
        return wrapMarkdownTables(html);
      },
    },
  }
);

const renderedHtml = computed(() => {
  if (!props.content) return "";
  try {
    let html = md.parse(normalizeMarkdownForRender(props.content)) as string;
    html = prepareMarkdownImages(html);
    html = injectAssetRefs(html);
    html = injectWorkspaceMentions(html);
    if (props.enableFileRefs) {
      html = injectFileRefs(html);
    }
    if (props.cursor) {
      html = html.replace(
        /((?:\s*<\/[^>]+>)+\s*)$/,
        '<span class="streaming-cursor">▍</span>$1'
      );
    }
    const highlightTerms = normalizeHighlightTerms(props.highlightTerms);
    if (highlightTerms.length) {
      html = highlightHtml(html, highlightTerms);
    }
    return html;
  } catch {
    return props.content;
  }
});

interface ResolvedMarkdownImage {
  url: string;
  displayPath: string;
}

const markdownImageCache = new Map<string, ResolvedMarkdownImage>();
let markdownImageResolveRun = 0;

function setMarkdownImageState(image: HTMLImageElement, state: "pending" | "loading" | "ready" | "error") {
  image.dataset.mdImageState = state;
  const frame = image.closest("[data-md-image-frame]") as HTMLElement | null;
  if (frame) {
    frame.dataset.mdImageState = state;
  }
}

function applyResolvedMarkdownImage(
  image: HTMLImageElement,
  source: string,
  resolved: ResolvedMarkdownImage,
) {
  image.src = resolved.url;
  image.dataset.mdImageResolvedFor = source;
  if (resolved.displayPath) {
    image.title = resolved.displayPath;
  }
  setMarkdownImageState(image, "ready");
}

async function resolveMarkdownImages() {
  const root = rootRef.value;
  if (!root) return;

  const run = ++markdownImageResolveRun;
  const images = Array.from(root.querySelectorAll<HTMLImageElement>("img[data-md-image-source]"));
  for (const image of images) {
    const source = image.dataset.mdImageSource?.trim() ?? "";
    if (!source || image.dataset.mdImageResolvedFor === source) continue;

    if (!shouldResolveMarkdownImageSource(source)) {
      applyResolvedMarkdownImage(image, source, {
        url: markdownImageDirectSrc(source),
        displayPath: source,
      });
      continue;
    }

    const cached = markdownImageCache.get(source);
    if (cached) {
      applyResolvedMarkdownImage(image, source, cached);
      continue;
    }

    setMarkdownImageState(image, "loading");
    try {
      const preview = await resolveMarkdownImage(source);
      const resolved = {
        url: preview.url,
        displayPath: preview.displayPath || source,
      };
      markdownImageCache.set(source, resolved);
      if (
        run !== markdownImageResolveRun
        || !image.isConnected
        || image.dataset.mdImageSource?.trim() !== source
      ) {
        continue;
      }
      applyResolvedMarkdownImage(image, source, resolved);
    } catch (error) {
      if (
        run !== markdownImageResolveRun
        || !image.isConnected
        || image.dataset.mdImageSource?.trim() !== source
      ) {
        continue;
      }
      console.warn("Failed to resolve markdown image:", error);
      image.removeAttribute("src");
      setMarkdownImageState(image, "error");
    }
  }
}

watch(
  renderedHtml,
  () => {
    void nextTick(resolveMarkdownImages);
  },
  { immediate: true, flush: "post" },
);

function isHandledMarkdownMouseButton(event: MouseEvent): boolean {
  return event.button === 0 || event.button === 1;
}

async function openMarkdownHref(href: string): Promise<void> {
  try {
    await openUrl(href);
  } catch (error) {
    console.warn("Failed to open markdown link externally:", error);
    if (!hasTauriWindowRuntime()) {
      window.open(href, "_blank", "noopener,noreferrer");
    }
  }
}

function handleMarkdownContentActivation(event: MouseEvent) {
  if (event.defaultPrevented || !isHandledMarkdownMouseButton(event)) return;
  if (!(event.target instanceof Element)) return;

  const anchor = event.target.closest("a[href]") as HTMLAnchorElement | null;
  if (anchor) {
    event.preventDefault();
    event.stopPropagation();

    const href = normalizeExternalMarkdownHref(anchor.getAttribute("href"));
    if (!href) return;

    void openMarkdownHref(href);
    return;
  }

  if (event.type !== "click" || event.button !== 0) return;
  const image = event.target.closest("img.md-image-preview[src]") as HTMLImageElement | null;
  if (!image || image.dataset.mdImageState !== "ready") return;

  event.preventDefault();
  event.stopPropagation();
  emit("openImage", image.currentSrc || image.src);
}

function normalizeUnityRefDatasetPath(value?: string): string {
  return (value ?? "").trim().replace(/\\/g, "/").replace(/\/+$/g, "");
}

function unityRefFromMarkdownDragTarget(target: Element): AssetRefAttachment | null {
  const sceneRef = target.closest(".md-unity-scene-object-ref") as HTMLElement | null;
  if (sceneRef) {
    const scenePath = normalizeUnityRefDatasetPath(sceneRef.dataset.scenePath);
    const objectPath = normalizeUnityRefDatasetPath(sceneRef.dataset.sceneObjectPath);
    if (scenePath && objectPath) {
      return {
        kind: "sceneObject",
        path: `${scenePath}/${objectPath}`,
        source: "manual",
      };
    }
  }

  const assetRef = target.closest(".md-unity-asset-ref, .md-file-ref[data-asset-path]") as HTMLElement | null;
  if (assetRef) {
    const assetPath = normalizeUnityRefDatasetPath(assetRef.dataset.assetPath || assetRef.dataset.filePath);
    if (/^(Assets|Packages)\//i.test(assetPath)) {
      return {
        kind: "asset",
        path: assetPath,
        source: "manual",
      };
    }
  }

  return null;
}

function localFileFromMarkdownDragTarget(target: Element): LocusFileDropRef | null {
  const workspaceRef = target.closest(".md-workspace-ref[data-workspace-path]") as HTMLElement | null;
  if (workspaceRef) {
    const path = normalizeUnityRefDatasetPath(workspaceRef.dataset.workspacePath);
    if (path) {
      return {
        path,
        isDir: workspaceRef.dataset.entryKind === "folder",
        source: "locus",
      };
    }
  }

  const fileRef = target.closest(".md-file-ref[data-file-path]") as HTMLElement | null;
  if (!fileRef || fileRef.classList.contains("md-knowledge-ref")) return null;
  if (fileRef.classList.contains("md-unity-asset-ref") || fileRef.classList.contains("md-unity-scene-object-ref")) {
    return null;
  }

  const path = normalizeUnityRefDatasetPath(fileRef.dataset.filePath);
  if (!path) return null;
  return {
    path,
    isDir: fileRef.dataset.entryKind === "folder",
    source: "locus",
  };
}

function handleMarkdownDragStart(event: DragEvent) {
  if (!(event.target instanceof Element)) return;
  const ref = unityRefFromMarkdownDragTarget(event.target);
  if (ref) {
    startUnityReferenceHtmlDrag(event, [ref]);
    return;
  }
  const file = localFileFromMarkdownDragTarget(event.target);
  if (!file) return;
  startLocusFileHtmlDrag(event, [file]);
}

function handleMarkdownPointerDown(event: PointerEvent) {
  if (!(event.target instanceof Element)) return;
  const ref = unityRefFromMarkdownDragTarget(event.target);
  if (ref) {
    armUnityReferencePointerDrag(event, [ref]);
    return;
  }
  const file = localFileFromMarkdownDragTarget(event.target);
  if (!file) return;
  armLocusFilePointerDrag(event, [file]);
}
</script>

<template>
  <div
    ref="rootRef"
    class="markdown-body ui-select-text"
    @click="handleMarkdownContentActivation"
    @auxclick="handleMarkdownContentActivation"
    @pointerdown="handleMarkdownPointerDown"
    @dragstart="handleMarkdownDragStart"
    v-html="renderedHtml"
  />
</template>

<style>
.markdown-body {
  font-family: var(--font-prose);
  font-size: 14px;
  line-height: 1.68;
  word-break: break-word;
  color: var(--text-color);
  text-rendering: optimizeLegibility;
}

.markdown-body h1,
.markdown-body h2,
.markdown-body h3,
.markdown-body h4,
.markdown-body h5,
.markdown-body h6 {
  margin: 24px 0 10px;
  font-weight: 600;
  line-height: 1.35;
  letter-spacing: -0.01em;
}

.markdown-body > :first-child {
  margin-top: 0;
}

.markdown-body > :last-child {
  margin-bottom: 0;
}

.markdown-body h1 {
  font-size: 1.58em;
  margin-bottom: 14px;
}

.markdown-body h2 {
  font-size: 1.3em;
  padding-bottom: 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
}

.markdown-body h3 {
  font-size: 1.12em;
}

.markdown-body h4,
.markdown-body h5,
.markdown-body h6 {
  font-size: 1em;
  color: var(--text-secondary);
}

.markdown-body p,
.markdown-body ul,
.markdown-body ol,
.markdown-body blockquote,
.markdown-body hr,
.markdown-body pre,
.markdown-body .md-table-wrap {
  margin: 0 0 12px;
}

.markdown-body ul,
.markdown-body ol {
  padding-left: 20px;
}

.markdown-body li {
  margin: 4px 0;
}

.markdown-body li > ul,
.markdown-body li > ol {
  margin-top: 6px;
  margin-bottom: 6px;
}

.markdown-body ul li::marker {
  color: color-mix(in srgb, var(--text-secondary) 72%, transparent);
}

.markdown-body ol li::marker {
  color: var(--text-secondary);
  font-weight: 600;
}

.markdown-body blockquote {
  padding: 8px 12px;
  border-left: 2px solid color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 44%, transparent);
  border-radius: 0 6px 6px 0;
}

.markdown-body blockquote > :last-child {
  margin-bottom: 0;
}

.markdown-body a {
  color: var(--accent-color);
  text-decoration-line: underline;
  text-decoration-thickness: 1px;
  text-underline-offset: 0.16em;
  text-decoration-color: color-mix(in srgb, var(--accent-color) 40%, transparent);
  transition: color 0.15s ease, text-decoration-color 0.15s ease;
}

.markdown-body a:hover {
  text-decoration-color: currentColor;
}

.markdown-body hr {
  border: none;
  border-top: 1px solid var(--border-color);
  opacity: 0.8;
}

.markdown-body .md-table-wrap {
  width: fit-content;
  max-width: 100%;
  box-sizing: border-box;
  overflow-x: auto;
  overflow-y: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.markdown-body table {
  width: max-content;
  min-width: 100%;
  margin: 0;
  border-collapse: separate;
  border-spacing: 0;
  table-layout: auto;
  font-size: 13px;
  background: transparent;
}

.markdown-body th,
.markdown-body td {
  min-width: 120px;
  padding: 7px 10px;
  text-align: left;
  vertical-align: top;
  white-space: normal;
  overflow-wrap: anywhere;
  word-break: normal;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent) !important;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent) !important;
  color: var(--text-color) !important;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%) !important;
}

.markdown-body th {
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 68%, var(--panel-bg) 32%) !important;
  font-weight: 600;
  color: var(--text-secondary) !important;
}

.markdown-body tr:last-child td {
  border-bottom: none;
}

.markdown-body th:last-child,
.markdown-body td:last-child {
  border-right: none;
}

.markdown-body tbody tr:nth-child(even) td {
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--hover-bg) 18%) !important;
}

.markdown-body code {
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  padding: 1px 6px;
  border-radius: 4px;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  color: color-mix(in srgb, var(--text-color) 92%, var(--accent-color) 8%);
}

.markdown-body pre {
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg, var(--panel-bg)) 76%, transparent);
  overflow-x: auto;
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--panel-bg) 32%, transparent);
}

.markdown-body pre code {
  display: block;
  font-family: var(--font-mono-block);
  padding: 10px 0;
  background: transparent;
  font-size: 13px;
  line-height: 1.55;
  white-space: pre;
  overflow-x: auto;
  counter-reset: line;
  border: none;
  color: inherit;
}

.markdown-body pre code .code-line {
  display: grid;
  grid-template-columns: 46px minmax(0, 1fr);
  align-items: start;
  min-width: 100%;
}

.markdown-body pre code .code-line-tree {
  grid-template-columns: minmax(0, 1fr);
}

.markdown-body pre code .line-number {
  display: block;
  padding: 0 10px 0 0;
  text-align: right;
  color: color-mix(in srgb, var(--text-secondary) 78%, transparent);
  user-select: none;
  opacity: 0.5;
  font-size: 11px;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.markdown-body pre code .line-content {
  display: block;
  padding: 0 14px 0 12px;
  min-width: 0;
}

.markdown-body pre code .code-line-tree .line-content {
  padding-left: 14px;
}

.markdown-body img {
  max-width: 100%;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  cursor: pointer;
}

.markdown-body .md-image-frame {
  display: block;
  width: fit-content;
  max-width: 100%;
  margin: 4px 0 12px;
  padding: 4px;
  box-sizing: border-box;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 42%, transparent);
  overflow: hidden;
}

.markdown-body .md-image-frame[data-md-image-state="pending"],
.markdown-body .md-image-frame[data-md-image-state="loading"],
.markdown-body .md-image-frame[data-md-image-state="error"] {
  width: min(320px, 100%);
  min-height: 120px;
  background:
    linear-gradient(
      135deg,
      color-mix(in srgb, var(--panel-bg) 80%, transparent),
      color-mix(in srgb, var(--hover-bg) 58%, transparent)
    );
}

.markdown-body .md-image-preview {
  display: block;
  max-width: min(720px, 100%);
  max-height: 420px;
  object-fit: contain;
  border: none;
  border-radius: 4px;
  background: transparent;
}

.markdown-body .md-image-preview[data-md-image-state="pending"],
.markdown-body .md-image-preview[data-md-image-state="loading"],
.markdown-body .md-image-preview[data-md-image-state="error"] {
  width: 100%;
  min-height: 110px;
  cursor: default;
}

.markdown-body strong {
  font-weight: 600;
}

.markdown-body em {
  color: color-mix(in srgb, var(--text-color) 82%, var(--text-secondary) 18%);
}

.markdown-body mark.markdown-search-mark {
  padding: 0 2px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--accent-color) 22%, var(--hover-bg));
  color: inherit;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 12%, transparent);
}

.markdown-body mark.markdown-search-mark-target {
  background: color-mix(in srgb, var(--accent-color) 34%, var(--hover-bg));
  box-shadow:
    inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 28%, transparent),
    0 0 0 1px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.markdown-body :not(pre) > code a,
.markdown-body :not(pre) > code {
  text-decoration: none;
}

.md-asset-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 7px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  cursor: pointer;
  font-size: 0.88em;
  line-height: 1.5;
  vertical-align: baseline;
  font-weight: 500;
  color: var(--text-secondary);
  user-select: none;
  -webkit-user-select: none;
}

.md-asset-chip:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.md-asset-chip-icon {
  font-size: 10px;
  opacity: 0.58;
}

.md-file-ref {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 22px;
  padding: 1px 6px 1px 5px;
  box-sizing: border-box;
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  cursor: pointer;
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  line-height: 18px;
  vertical-align: -2px;
  font-weight: 400;
  color: color-mix(in srgb, var(--text-color) 90%, var(--text-secondary) 10%);
  user-select: none;
  -webkit-user-select: none;
}

.md-unity-asset-ref,
.md-unity-scene-object-ref {
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 54%, transparent);
  border-color: color-mix(in srgb, var(--border-color) 78%, transparent);
  color: color-mix(in srgb, var(--text-color) 90%, var(--text-secondary) 10%);
}

.md-workspace-ref {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 22px;
  padding: 1px 6px 1px 5px;
  box-sizing: border-box;
  border-radius: 4px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 52%, transparent);
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  cursor: pointer;
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  line-height: 18px;
  vertical-align: -2px;
  font-weight: 400;
  color: color-mix(in srgb, var(--text-color) 86%, var(--text-secondary) 14%);
  user-select: none;
  -webkit-user-select: none;
}

.markdown-body :is(.md-asset-chip, .md-file-ref, .md-workspace-ref) {
  user-select: none;
  -webkit-user-select: none;
}

.md-file-ref:hover,
.md-file-ref:active {
  background: color-mix(in srgb, var(--hover-bg) 78%, var(--sidebar-bg, var(--hover-bg)) 22%);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.md-unity-asset-ref:hover,
.md-unity-asset-ref:active,
.md-unity-scene-object-ref:hover,
.md-unity-scene-object-ref:active {
  background: color-mix(in srgb, var(--accent-color) 5%, var(--hover-bg) 95%);
  border-color: color-mix(in srgb, var(--accent-color) 18%, var(--border-strong) 82%);
}

.md-workspace-ref:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.md-workspace-ref-prefix {
  margin-right: 1px;
  opacity: 0.58;
}

.md-workspace-ref-icon {
  margin-right: 2px;
}

.md-ref-label {
  min-width: 0;
  display: block;
  line-height: 18px;
}

.md-ref-icon {
  display: block;
  width: 14px;
  min-width: 14px;
  height: 14px;
  align-self: center;
  flex-shrink: 0;
  object-fit: contain;
  max-width: none;
  border: none;
  border-radius: 0;
  background: transparent;
  opacity: 0.82;
  cursor: inherit;
  pointer-events: none;
  user-select: none;
}

.md-ref-icon-lucide {
  display: block;
  opacity: 0.95;
  filter: none;
}

img.md-ref-icon-image {
  display: none;
}

.md-workspace-ref-prefix {
  display: none;
}

.streaming-cursor {
  color: var(--accent-color);
  font-weight: 400;
  margin-left: 1px;
  animation: streaming-cursor-blink 0.8s step-end infinite;
}

@keyframes streaming-cursor-blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0; }
}
</style>
