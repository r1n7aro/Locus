<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import MarkdownRenderer from "../components/MarkdownRenderer.vue";
import { useFileChangeRevalidation } from "../composables/useFileChangeRevalidation";
import { parseCsvDocument } from "../document/csv/csvDocument";
import { normalizeAppError } from "../services/errors";
import { knowledgeRead } from "../services/knowledge";
import { openRemoteReference } from "../services/remoteClient";
import { watchWindowEventInterests } from "../services/windowEventInterests";
import { workspaceFilePreview, workspaceFileRevision } from "../services/workspaceExplorer";
import type { KnowledgeChangedEvent, KnowledgeDocument } from "../types";
import type { ProjectExplorerFilePreview, ProjectExplorerFileRevision } from "../types/workbench";
import { t } from "../i18n";
import { checkoutRoot } from "./api";
import { onWorkspaceEvent } from "./events";
import { retainFileWatch } from "./fileWatches";
import {
  capturePosition,
  documentFontScale,
  FONT_SCALE_DEFAULT,
  FONT_SCALE_MAX,
  FONT_SCALE_MIN,
  loadReadingPosition,
  pinchFontScale,
  positionDelta,
  readingKey,
  saveReadingPosition,
  setDocumentFontScale,
  stepFontScale,
  type ReadingPosition,
} from "./reading";
import {
  headingSlug,
  knowledgeFilePath,
  referenceFromElement,
  relativeToRoot,
  resolveDocumentLink,
  type ReferenceTarget,
} from "./references";
import { baseName, type DocumentSource } from "./routes";

// Read-only viewer for workspace files and knowledge documents: Markdown is
// rendered (links and references open in place), CSV shown as a table, code
// highlighted, images and media inline. Follows changes made on the desktop.
// Text documents take the phone's text size (pinch or menu) and reopen where
// they were left off.
const props = defineProps<{
  source: DocumentSource;
  title: string;
  connected: boolean;
  /** The screen is on top of the stack. */
  active: boolean;
  /** Bumped after a reconnect or dropped events. */
  refreshToken: number;
}>();

const emit = defineEmits<{ back: []; openImage: [src: string, title: string] }>();

type View =
  | { kind: "markdown"; text: string }
  | { kind: "csv"; rows: string[][]; truncatedRows: boolean; text: string }
  | { kind: "code"; text: string; language: string }
  /** Too large to highlight on a phone. */
  | { kind: "plain"; text: string }
  | { kind: "image"; src: string }
  | { kind: "media"; src: string; element: "audio" | "video" }
  | { kind: "unsupported" };

type Heading = { level: number; text: string };

const CSV_ROW_LIMIT = 500;
const HIGHLIGHT_LIMIT = 200_000;
const view = ref<View | null>(null);
const preview = ref<ProjectExplorerFilePreview | null>(null);
const knowledgeDoc = ref<KnowledgeDocument | null>(null);
const revision = ref<ProjectExplorerFileRevision | null>(null);
const truncatedLines = ref<number | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);
const stale = ref(false);
const bodyRef = ref<HTMLElement | null>(null);
const headings = ref<Heading[]>([]);
const outlineOpen = ref(false);
const menuOpen = ref(false);
const toast = ref<string | null>(null);
let toastTimer: ReturnType<typeof setTimeout> | null = null;
let loadRun = 0;

const extension = computed(() => {
  const name = props.source.path.split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
});

/** Workspace-relative path of the file behind the document, when known. */
const workspacePath = computed(() => {
  if (props.source.kind === "knowledge") {
    return knowledgeDoc.value?.storageSource === "app" ? null : knowledgeFilePath(props.source.docType, props.source.path);
  }
  return relativeToRoot(props.source.path, checkoutRoot(props.source.workspaceRef.checkoutId));
});

/** Base for relative Markdown images. */
const contentPath = computed(() => (props.source.kind === "file" ? props.source.path : workspacePath.value));

const displayPath = computed(() => {
  if (props.source.kind === "knowledge") return props.source.path;
  return workspacePath.value ?? props.source.path;
});

/** Knowledge documents show their own title once loaded (links name files). */
const displayTitle = computed(() => (props.source.kind === "knowledge" && knowledgeDoc.value?.title) || props.title);

const subtitle = computed(() => {
  const source = props.source;
  if (source.kind === "knowledge") {
    const path = source.path.replace(/^\/+/, "");
    const relative = path.toLowerCase().startsWith(`${source.docType}/`) ? path.slice(source.docType.length + 1) : path;
    return `${t(`knowledge.type.${source.docType}`)} · ${relative}`;
  }
  const root = checkoutRoot(source.workspaceRef.checkoutId);
  return root && workspacePath.value !== null ? `${baseName(root)}/${workspacePath.value}` : displayPath.value;
});

/** The viewer is titled after the tapped image, not the document. */
function openMarkdownImage(src: string) {
  const image = [...(bodyRef.value?.querySelectorAll<HTMLImageElement>(".m-doc-markdown img") ?? [])]
    .find((candidate) => (candidate.currentSrc || candidate.src) === src);
  const name = image?.dataset.mdImageSource ? baseName(image.dataset.mdImageSource) : image?.alt;
  emit("openImage", src, name || displayTitle.value);
}

function formatSize(bytes: number | null | undefined): string {
  if (bytes == null) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatTime(ms: number | null | undefined): string {
  if (!ms) return "";
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString(undefined, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

/** One line of facts under the title. */
const facts = computed(() => {
  if (props.source.kind === "knowledge") {
    const doc = knowledgeDoc.value;
    if (!doc) return [];
    const metadata = doc.fileMetadata;
    return [
      doc.storageSource === "app" ? t("mobile.document.appSource") : t("mobile.document.projectSource"),
      doc.readOnly ? t("mobile.document.readOnly") : "",
      formatTime(metadata?.modifiedAt ?? doc.modifiedAt) ? t("mobile.document.updated", formatTime(metadata?.modifiedAt ?? doc.modifiedAt)) : "",
      metadata?.lineCount ? t("mobile.document.lines", String(metadata.lineCount)) : "",
    ].filter(Boolean);
  }
  const file = preview.value;
  if (!file) return [];
  return [
    formatSize(file.size),
    file.kind === "text" && file.totalLines ? t("mobile.document.lines", String(file.totalLines)) : "",
    file.kind !== "text" ? file.mimeType : "",
  ].filter(Boolean);
});

const summary = computed(() => knowledgeDoc.value?.summary?.trim() || "");
const copyableText = computed(() => {
  const current = view.value;
  if (!current) return null;
  return "text" in current ? current.text : null;
});

function csvView(text: string): View {
  try {
    const parsed = parseCsvDocument(text);
    const rows = parsed.records.slice(0, CSV_ROW_LIMIT).map((record) => record.fields.map((field) => field.value));
    return { kind: "csv", rows, truncatedRows: parsed.records.length > CSV_ROW_LIMIT, text };
  } catch {
    return codeView(text, "csv");
  }
}

function codeView(text: string, language: string): View {
  return text.length > HIGHLIGHT_LIMIT ? { kind: "plain", text } : { kind: "code", text, language };
}

/** Fenced so MarkdownRenderer highlights it; the fence outlasts any
 * backtick run inside the text. */
const codeMarkdown = computed(() => {
  if (view.value?.kind !== "code") return "";
  const longest = Math.max(2, ...[...view.value.text.matchAll(/`+/g)].map((match) => match[0].length));
  const fence = "`".repeat(longest + 1);
  return `${fence}${view.value.language}\n${view.value.text}\n${fence}`;
});

function fileView(file: ProjectExplorerFilePreview): View {
  switch (file.kind) {
    case "text": {
      const text = file.text ?? "";
      if (["md", "markdown", "mdx"].includes(extension.value)) return { kind: "markdown", text };
      if (extension.value === "csv" || extension.value === "tsv") return csvView(text);
      return codeView(text, extension.value);
    }
    case "image":
      return file.blobUrl ? { kind: "image", src: file.blobUrl } : { kind: "unsupported" };
    case "audio":
    case "video":
      return file.blobUrl ? { kind: "media", src: file.blobUrl, element: file.kind } : { kind: "unsupported" };
    default:
      return { kind: "unsupported" };
  }
}

function sameView(left: View | null, right: View): boolean {
  if (!left || left.kind !== right.kind) return false;
  if ("text" in left && "text" in right) return left.text === right.text;
  if ("src" in left && "src" in right) return left.src === right.src;
  return left.kind === "unsupported";
}

/** `silent`: a refresh after a change on the desktop; the current content and
 * scroll position stay until the new content is in. */
async function load(options: { silent?: boolean } = {}) {
  const run = ++loadRun;
  if (!options.silent) loading.value = true;
  error.value = null;
  stale.value = false;
  try {
    let next: View;
    if (props.source.kind === "file") {
      const file = await workspaceFilePreview(props.source.path, props.source.workspaceRef);
      if (run !== loadRun) return;
      preview.value = file;
      revision.value = file.revision;
      truncatedLines.value = file.truncated ? file.totalLines ?? 0 : null;
      next = fileView(file);
    } else {
      const result = await knowledgeRead(
        { kind: "document", path: props.source.path, type: props.source.docType, part: "full" },
        props.source.workspaceRef,
      );
      if (run !== loadRun) return;
      knowledgeDoc.value = result.document ?? null;
      const body = result.document?.body ?? "";
      next = extension.value === "csv" ? csvView(body) : { kind: "markdown", text: body };
    }
    if (sameView(view.value, next)) return;
    const scroller = bodyRef.value;
    const scrollTop = scroller?.scrollTop ?? 0;
    view.value = next;
    await nextTick();
    if (options.silent && scroller) scroller.scrollTop = scrollTop;
    collectHeadings();
    restoreProgress();
    scheduleProgress();
  } catch (cause) {
    if (run === loadRun) error.value = normalizeAppError(cause).message;
  } finally {
    if (run === loadRun) loading.value = false;
  }
}

function refreshWhenVisible() {
  if (props.active && props.connected) void load({ silent: true });
  else stale.value = true;
}

// ----- following changes: files by revision, knowledge by its change events.
useFileChangeRevalidation({
  active: () => props.active && props.connected && props.source.kind === "file" && view.value !== null,
  currentRevision: () => revision.value,
  probe: () => workspaceFileRevision(props.source.path, props.source.workspaceRef),
  workspaceRef: () => props.source.workspaceRef,
  workspacePath: () => (props.source.kind === "file" ? workspacePath.value : null),
  onChanged: () => load({ silent: true }),
  onError: () => {},
});

const stopInterests = watchWindowEventInterests(() => [{ workspaceRef: props.source.workspaceRef }]);
// The desktop forwards file changes only for watched paths.
let releaseWatch: (() => void) | null = null;
watch(
  () => (props.source.kind === "file" ? workspacePath.value : null),
  (path) => {
    releaseWatch?.();
    releaseWatch = path === null ? null : retainFileWatch(props.source.workspaceRef.checkoutId, path);
  },
  { immediate: true },
);

function samePath(left: string, right: string): boolean {
  const normalize = (value: string) => value.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "").toLowerCase();
  return normalize(left) === normalize(right);
}

function knowledgeChangeAffects(change: KnowledgeChangedEvent): boolean {
  const source = props.source;
  if (source.kind !== "knowledge") return false;
  if (change.docType && change.docType !== source.docType) return false;
  if (!change.path || change.targetKind === "workspace" || change.targetKind === "type") return true;
  const own = source.path.replace(/^\/+/, "");
  const withType = own.toLowerCase().startsWith(`${source.docType}/`) ? own : `${source.docType}/${own}`;
  const withoutType = withType.slice(source.docType.length + 1);
  if (samePath(change.path, withType) || samePath(change.path, withoutType)) return true;
  return change.targetKind === "directory" && !!change.subtree;
}

const stopEvents = onWorkspaceEvent((event) => {
  if (props.source.kind !== "knowledge" || event.eventName !== "knowledge-changed") return;
  if (event.checkoutId !== props.source.workspaceRef.checkoutId) return;
  if (knowledgeChangeAffects(event.payload as KnowledgeChangedEvent)) refreshWhenVisible();
});

watch(() => props.refreshToken, refreshWhenVisible);
watch(() => [props.active, props.connected] as const, ([active, connected], previous) => {
  if (!active || !connected) return;
  if (!view.value && !loading.value) void load();
  else if (stale.value || (previous && !previous[1])) void load({ silent: true });
});

// ----- links, references, outline.
function open(target: ReferenceTarget) {
  openRemoteReference({ ...target, workspaceRef: props.source.workspaceRef });
}

/** Headings as rendered now: the renderer may replace its DOM after
 * references resolve, so elements are looked up when needed. */
function headingElements(): HTMLElement[] {
  const root = bodyRef.value?.querySelector(".m-doc-markdown");
  return root
    ? [...root.querySelectorAll<HTMLElement>("h1, h2, h3, h4")].filter((element) => element.textContent?.trim())
    : [];
}

function collectHeadings() {
  headings.value = headingElements().map((element) => ({
    level: Number(element.tagName[1]),
    text: element.textContent?.trim() ?? "",
  }));
}

function toggleOutline() {
  menuOpen.value = false;
  if (!outlineOpen.value) collectHeadings();
  outlineOpen.value = !outlineOpen.value;
}

function scrollToHeading(index: number) {
  outlineOpen.value = false;
  headingElements()[index]?.scrollIntoView({ block: "start" });
}

function scrollToAnchor(id: string) {
  const slug = headingSlug(id);
  const byId = bodyRef.value?.querySelector<HTMLElement>(`[id="${CSS.escape(id)}"], a[name="${CSS.escape(id)}"]`);
  const target = byId ?? headingElements().find((element) => headingSlug(element.textContent ?? "") === slug);
  target?.scrollIntoView({ block: "start" });
}

/** Capture phase: runs before MarkdownRenderer, which opens only external
 * links and ignores the rest. */
function onBodyClick(event: MouseEvent) {
  if (!(event.target instanceof Element)) return;
  const anchor = event.target.closest("a[href]");
  if (anchor) {
    const link = resolveDocumentLink(anchor.getAttribute("href"), props.source);
    if (!link || link.kind === "external") return;
    event.preventDefault();
    event.stopPropagation();
    if (link.kind === "anchor") scrollToAnchor(link.id);
    else open(link);
    return;
  }
  const reference = referenceFromElement(event.target);
  if (!reference) return;
  event.preventDefault();
  event.stopPropagation();
  open(reference);
}

// ----- menu actions.
function showToast(text: string) {
  toast.value = text;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { toast.value = null; }, 1800);
}

async function copy(text: string) {
  menuOpen.value = false;
  try {
    await navigator.clipboard.writeText(text);
    showToast(t("mobile.document.copied"));
  } catch {
    showToast(t("mobile.document.copyFailed"));
  }
}

function refresh() {
  menuOpen.value = false;
  void load({ silent: view.value !== null });
}

// ----- text size and reading position. Both work on the rendered layout
// directly: a re-render per scroll or pinch step would diff whole tables.
const SAVE_DELAY_MS = 400;
/** Views whose text follows the text size and whose position is kept. */
const readable = computed(() => {
  const kind = view.value?.kind;
  return kind === "markdown" || kind === "code" || kind === "plain" || kind === "csv";
});
const fontPercent = computed(() => Math.round(documentFontScale.value * 100));
const canShrinkFont = computed(() => documentFontScale.value > FONT_SCALE_MIN);
const canGrowFont = computed(() => documentFontScale.value < FONT_SCALE_MAX);
const contentRef = ref<HTMLElement | null>(null);
const progressRef = ref<HTMLElement | null>(null);
/** Storage key, fixed when the document is first shown. */
let progressKey: string | null = null;
/** Saved position being restored: kept in view while images and references
 * still change the layout, until the reader touches the page. */
let restoring: ReadingPosition | null = null;
/** Position waiting to be stored: `null` forgets it, `undefined` is none. */
let unsavedPosition: ReadingPosition | null | undefined;
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let progressFrame = 0;
let appliedScale: number | null = null;
let resizeObserver: ResizeObserver | null = null;

/** Top-level blocks of the shown text, in document order. */
function readingBlocks(): Element[] {
  const body = bodyRef.value;
  if (!body || !readable.value) return [];
  const markdown = body.querySelector(".m-doc-markdown > .markdown-body, .m-doc-code > .markdown-body");
  if (markdown) return [...markdown.children];
  const rows = body.querySelector(".m-doc-table > tbody");
  if (rows) return [...rows.children];
  return [...body.querySelectorAll(".m-doc-plain")];
}

function scrollRatio(scroller: HTMLElement): number {
  const range = scroller.scrollHeight - scroller.clientHeight;
  return range > 0 ? Math.min(1, Math.max(0, scroller.scrollTop / range)) : 0;
}

/** `y`: the line to describe, in client coordinates (the viewport top by
 * default). Null while the screen is hidden. */
function currentPosition(y?: number): ReadingPosition | null {
  const scroller = bodyRef.value;
  if (!scroller || scroller.clientHeight === 0) return null;
  return capturePosition(readingBlocks(), y ?? scroller.getBoundingClientRect().top, scrollRatio(scroller));
}

/** Scrolls so `position` is at `y` (the viewport top by default). */
function scrollToPosition(position: ReadingPosition, y?: number) {
  const scroller = bodyRef.value;
  if (!scroller || scroller.clientHeight === 0) return;
  const delta = positionDelta(readingBlocks(), position, y ?? scroller.getBoundingClientRect().top);
  if (delta !== null) scroller.scrollTop += delta;
  else scroller.scrollTop = position.ratio * (scroller.scrollHeight - scroller.clientHeight);
}

/** Once per document, after it is first shown. */
function restoreProgress() {
  if (progressKey !== null || !readable.value) return;
  progressKey = readingKey(props.source, workspacePath.value);
  const saved = loadReadingPosition(progressKey);
  if (!saved) return;
  restoring = saved;
  scrollToPosition(saved);
  const scroller = bodyRef.value;
  if (scroller && scroller.scrollTop > 0) {
    showToast(t("mobile.document.resumed", String(Math.round(scrollRatio(scroller) * 100))));
  }
}

function stopRestoring() {
  restoring = null;
}

function scheduleProgress() {
  if (!progressFrame) progressFrame = requestAnimationFrame(updateProgress);
}

/** Moves the progress bar and notes the position to save. */
function updateProgress() {
  progressFrame = 0;
  const scroller = bodyRef.value;
  if (!scroller || scroller.clientHeight === 0) return;
  const bar = progressRef.value;
  if (bar) {
    bar.hidden = !readable.value || scroller.scrollHeight - scroller.clientHeight <= 1;
    bar.style.transform = `scaleX(${scrollRatio(scroller)})`;
  }
  if (restoring || progressKey === null) return;
  // Back at the top there is nothing to return to.
  unsavedPosition = scroller.scrollTop < 1 ? null : currentPosition();
  if (!saveTimer) saveTimer = setTimeout(saveProgress, SAVE_DELAY_MS);
}

function saveProgress() {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = null;
  if (progressKey === null || unsavedPosition === undefined) return;
  saveReadingPosition(progressKey, unsavedPosition);
  unsavedPosition = undefined;
}

function onContentResize() {
  if (restoring) scrollToPosition(restoring);
  scheduleProgress();
}

function onVisibilityChange() {
  if (document.visibilityState === "hidden") saveProgress();
}

/** Keeps the line at `y` (the viewport top by default) in place. */
function applyFontScale(scale: number, y?: number) {
  const scroller = bodyRef.value;
  if (!scroller || scale === appliedScale) return;
  const anchor = restoring ? null : currentPosition(y);
  scroller.style.setProperty("--text-viewer-font-scale", String(scale));
  appliedScale = scale;
  if (restoring) scrollToPosition(restoring);
  else if (anchor) scrollToPosition(anchor, y);
  scheduleProgress();
}

function changeFontScale(scale: number, options: { y?: number; persist?: boolean } = {}) {
  applyFontScale(setDocumentFontScale(scale, { persist: options.persist }), options.y);
}

// Changed on another document screen (screens below keep the same size).
watch(documentFontScale, (scale) => applyFontScale(scale));

function stepFont(direction: 1 | -1) {
  changeFontScale(stepFontScale(documentFontScale.value, direction));
}

function resetFont() {
  changeFontScale(FONT_SCALE_DEFAULT);
}

// Two-finger pinch sets the text size; the page itself does not zoom
// (`touch-action` in mobile.css). Listeners stay passive so scrolling never
// waits for this page.
let pinch: { distance: number; scale: number } | null = null;
let pinchTarget: { scale: number; y: number } | null = null;
let pinchFrame = 0;

function fingerDistance(touches: TouchList): number {
  const first = touches[0]!;
  const second = touches[1]!;
  return Math.hypot(first.clientX - second.clientX, first.clientY - second.clientY);
}

function applyPinch() {
  if (pinchFrame) cancelAnimationFrame(pinchFrame);
  pinchFrame = 0;
  const target = pinchTarget;
  pinchTarget = null;
  if (target) changeFontScale(target.scale, { y: target.y, persist: false });
}

function finishPinch() {
  const started = pinch;
  if (!started) return;
  pinch = null;
  applyPinch();
  const scale = setDocumentFontScale(documentFontScale.value);
  if (scale !== started.scale) showToast(t("mobile.document.fontScale", String(Math.round(scale * 100))));
}

function onTouchStart(event: TouchEvent) {
  stopRestoring();
  if (event.touches.length !== 2 || !readable.value) {
    finishPinch();
    return;
  }
  const distance = fingerDistance(event.touches);
  if (distance > 0) pinch = { distance, scale: documentFontScale.value };
}

function onTouchMove(event: TouchEvent) {
  if (!pinch || event.touches.length !== 2) return;
  pinchTarget = {
    scale: pinchFontScale(pinch.scale, fingerDistance(event.touches) / pinch.distance),
    // Keeps the text between the fingers in place.
    y: (event.touches[0]!.clientY + event.touches[1]!.clientY) / 2,
  };
  if (!pinchFrame) pinchFrame = requestAnimationFrame(applyPinch);
}

function onTouchEnd(event: TouchEvent) {
  if (event.touches.length < 2) finishPinch();
}

onMounted(() => {
  applyFontScale(documentFontScale.value);
  document.addEventListener("visibilitychange", onVisibilityChange);
  if (typeof ResizeObserver !== "undefined" && contentRef.value) {
    resizeObserver = new ResizeObserver(onContentResize);
    resizeObserver.observe(contentRef.value);
  }
  if (props.connected) void load();
});

onBeforeUnmount(() => {
  stopInterests();
  stopEvents();
  releaseWatch?.();
  if (toastTimer) clearTimeout(toastTimer);
  if (pinch) setDocumentFontScale(documentFontScale.value);
  if (pinchFrame) cancelAnimationFrame(pinchFrame);
  if (progressFrame) cancelAnimationFrame(progressFrame);
  saveProgress();
  resizeObserver?.disconnect();
  document.removeEventListener("visibilitychange", onVisibilityChange);
});
</script>

<template>
  <section class="m-screen m-document">
    <header class="m-topbar">
      <button class="m-icon-button" type="button" :aria-label="t('mobile.common.back')" @click="emit('back')">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 5l-7 7 7 7" /></svg>
      </button>
      <div class="m-topbar-text">
        <span class="m-topbar-title">{{ displayTitle }}</span>
        <span class="m-topbar-sub">{{ subtitle }}</span>
      </div>
      <button
        v-if="headings.length > 1"
        class="m-icon-button"
        type="button"
        :aria-label="t('mobile.document.outline')"
        :aria-expanded="outlineOpen"
        @click="toggleOutline"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M8 12h12M8 18h12" /></svg>
      </button>
      <div class="m-menu-anchor">
        <button
          class="m-icon-button"
          type="button"
          :aria-label="t('mobile.common.more')"
          :aria-expanded="menuOpen"
          @click="menuOpen = !menuOpen; outlineOpen = false"
        >
          <svg v-if="!loading" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 6h.01M12 12h.01M12 18h.01" /></svg>
          <svg v-else viewBox="0 0 24 24" aria-hidden="true" class="is-spinning">
            <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v5h-5" />
          </svg>
        </button>
        <div v-if="menuOpen" class="m-menu" role="menu">
          <div v-if="readable" class="m-menu-zoom" role="group" :aria-label="t('mobile.document.fontSize')">
            <button
              class="m-icon-button"
              type="button"
              :aria-label="t('mobile.document.fontSmaller')"
              :disabled="!canShrinkFont"
              @click="stepFont(-1)"
            >
              <span class="m-zoom-glyph is-small" aria-hidden="true">A</span>
            </button>
            <button class="m-menu-zoom-value" type="button" :aria-label="t('mobile.document.fontReset')" @click="resetFont">
              {{ fontPercent }}%
            </button>
            <button
              class="m-icon-button"
              type="button"
              :aria-label="t('mobile.document.fontLarger')"
              :disabled="!canGrowFont"
              @click="stepFont(1)"
            >
              <span class="m-zoom-glyph is-large" aria-hidden="true">A</span>
            </button>
          </div>
          <button class="m-menu-item" type="button" role="menuitem" :disabled="loading || !connected" @click="refresh">
            {{ t("mobile.common.refresh") }}
          </button>
          <button class="m-menu-item" type="button" role="menuitem" @click="copy(displayPath)">
            {{ t("mobile.document.copyPath") }}
          </button>
          <button
            v-if="copyableText !== null"
            class="m-menu-item"
            type="button"
            role="menuitem"
            @click="copy(copyableText ?? '')"
          >
            {{ t("mobile.document.copyText") }}
          </button>
        </div>
      </div>
    </header>
    <div v-if="menuOpen || outlineOpen" class="m-backdrop" @click="menuOpen = false; outlineOpen = false" />

    <div class="m-doc-frame">
      <nav v-if="outlineOpen" class="m-outline" :aria-label="t('mobile.document.outline')">
        <button
          v-for="(heading, index) in headings"
          :key="index"
          type="button"
          class="m-outline-item"
          :data-level="heading.level"
          @click="scrollToHeading(index)"
        >
          {{ heading.text }}
        </button>
      </nav>

      <div ref="progressRef" class="m-doc-progress" hidden aria-hidden="true" />

      <div
        ref="bodyRef"
        class="m-scroll m-document-body"
        @click.capture="onBodyClick"
        @scroll.passive="scheduleProgress"
        @touchstart.passive="onTouchStart"
        @touchmove.passive="onTouchMove"
        @touchend.passive="onTouchEnd"
        @touchcancel.passive="onTouchEnd"
        @wheel.passive="stopRestoring"
        @mousedown="stopRestoring"
      >
        <div ref="contentRef">
          <div v-if="error" class="m-notice" role="alert">{{ error }}</div>
          <div v-if="loading && !view" class="m-placeholder">{{ t("mobile.common.loading") }}</div>
          <div v-if="view && (facts.length || summary)" class="m-doc-head">
            <p v-if="facts.length" class="m-doc-facts">{{ facts.join(" · ") }}</p>
            <p v-if="summary" class="m-doc-summary">{{ summary }}</p>
          </div>
          <p v-if="truncatedLines !== null" class="m-doc-note">
            {{ truncatedLines ? t("mobile.document.truncatedLines", String(truncatedLines)) : t("mobile.document.truncated") }}
          </p>
          <template v-if="view">
            <div v-if="view.kind === 'markdown'" class="m-doc-markdown">
              <MarkdownRenderer
                :content="view.text"
                :workspace-ref="source.workspaceRef"
                :content-path="contentPath"
                @open-image="openMarkdownImage"
              />
            </div>
            <div v-else-if="view.kind === 'code'" class="m-doc-code">
              <MarkdownRenderer :content="codeMarkdown" :workspace-ref="source.workspaceRef" />
            </div>
            <pre v-else-if="view.kind === 'plain'" class="m-doc-plain">{{ view.text }}</pre>
            <div v-else-if="view.kind === 'csv'" class="m-doc-table-wrap">
              <table class="m-doc-table">
                <tbody>
                  <tr v-for="(row, rowIndex) in view.rows" :key="rowIndex" :class="{ 'is-head': rowIndex === 0 }">
                    <td v-for="(cell, cellIndex) in row" :key="cellIndex">{{ cell }}</td>
                  </tr>
                </tbody>
              </table>
              <p v-if="view.truncatedRows" class="m-doc-note">{{ t("mobile.document.rowsLimited", String(CSV_ROW_LIMIT)) }}</p>
            </div>
            <button
              v-else-if="view.kind === 'image'"
              class="m-doc-image-button"
              type="button"
              :aria-label="t('mobile.image.open')"
              @click="emit('openImage', view.src, title)"
            >
              <img class="m-doc-image" :src="view.src" :alt="title">
            </button>
            <video v-else-if="view.kind === 'media' && view.element === 'video'" class="m-doc-image" :src="view.src" controls />
            <audio v-else-if="view.kind === 'media'" :src="view.src" controls />
            <div v-else class="m-placeholder">{{ t("mobile.document.unsupported") }}</div>
          </template>
        </div>
      </div>
    </div>
    <div v-if="toast" class="m-toast" role="status">{{ toast }}</div>
  </section>
</template>
