<script setup lang="ts">
// Rendered view of a `.tex` file, backed by the LaTeX preview backend
// (`src-tauri/src/latex_preview`). Mounting opens a preview session and
// unmounting closes it, so the backend runs on demand like the other file
// previews. SVG pages are the normal output; when the document needs PDF
// output the cached PDF is shown instead, with the reason. Builds run only
// while this pane is visible: changes made while it is hidden build when it
// is shown again.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import {
  closeLatexPreview,
  latexEngineAvailable,
  openLatexPreview,
  rebuildLatexPreview,
  setLatexPreviewVisible,
  subscribeLatexPreviewUpdated,
  type LatexDiagnostic,
  type LatexPageRef,
  type LatexPreviewSnapshot,
} from "../../services/latexPreview";
import type { RuntimeUnsubscribe } from "../../services/locusRuntime";
import type { WorkspaceRef } from "../../services/project";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";

const props = defineProps<{
  /** Scopes dependency watching to the workspace; files outside one use their folder. */
  workspaceRef: WorkspaceRef | null;
  /** Absolute path of the `.tex` file. */
  path: string;
  active: boolean;
  /** Emit `unavailable` instead of opening when no TeX engine is installed. */
  autoFallback?: boolean;
}>();

const emit = defineEmits<{ unavailable: [] }>();

const subscriptionId = ref<string | null>(null);
const snapshot = ref<LatexPreviewSnapshot | null>(null);
const shownPages = ref<LatexPageRef[]>([]);
const openError = ref("");
const opening = ref(false);
const viewMode = ref<"auto" | "pdf">("auto");
const showDetails = ref(false);
const pageVisible = ref(typeof document === "undefined" || !document.hidden);

const visible = computed(() => props.active && pageVisible.value);

/** Pages preloaded before they replace the shown ones, so updates do not flash. */
const PRELOAD_PAGES = 8;
let preloadToken = 0;
let openVersion = 0;
let unsubscribe: RuntimeUnsubscribe | null = null;
let disposed = false;

function preload(url: string): Promise<void> {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve();
    image.onerror = () => resolve();
    image.src = url;
  });
}

function samePages(a: LatexPageRef[], b: LatexPageRef[]): boolean {
  return a.length === b.length && a.every((page, index) => page.url === b[index]?.url);
}

function applySnapshot(next: LatexPreviewSnapshot) {
  snapshot.value = next;
  if (next.mode !== "svg") {
    if (next.mode === "pdf") shownPages.value = [];
    return;
  }
  if (samePages(shownPages.value, next.pages)) return;
  const token = ++preloadToken;
  void Promise.all(next.pages.slice(0, PRELOAD_PAGES).map((page) => preload(page.url))).then(() => {
    if (token === preloadToken && !disposed) shownPages.value = next.pages;
  });
}

async function releaseSubscription() {
  const id = subscriptionId.value;
  subscriptionId.value = null;
  if (id) await closeLatexPreview(id).catch(() => undefined);
}

async function open() {
  const workspaceRef = props.workspaceRef;
  const version = ++openVersion;
  await releaseSubscription();
  openError.value = "";
  if (!props.path) return;
  opening.value = true;
  try {
    if (props.autoFallback && !(await latexEngineAvailable())) {
      if (version === openVersion && !disposed) emit("unavailable");
      return;
    }
    if (version !== openVersion || disposed) return;
    const result = await openLatexPreview(workspaceRef, props.path, visible.value);
    if (version !== openVersion || disposed) {
      void closeLatexPreview(result.subscriptionId).catch(() => undefined);
      return;
    }
    subscriptionId.value = result.subscriptionId;
    applySnapshot(result.snapshot);
  } catch (cause) {
    if (version === openVersion) openError.value = normalizeAppError(cause).message;
  } finally {
    if (version === openVersion) opening.value = false;
  }
}

async function reportVisibility(value: boolean) {
  const id = subscriptionId.value;
  if (!id) {
    // The earlier open failed: retry when shown.
    if (value && !opening.value) await open();
    return;
  }
  try {
    applySnapshot(await setLatexPreviewVisible(id, value));
  } catch (cause) {
    const error = normalizeAppError(cause);
    if (error.code === "latex.preview_closed") {
      subscriptionId.value = null;
      if (value) await open();
    } else {
      openError.value = error.message;
    }
  }
}

async function rebuild() {
  const id = subscriptionId.value;
  if (!id) {
    await open();
    return;
  }
  try {
    await rebuildLatexPreview(id);
  } catch (cause) {
    openError.value = normalizeAppError(cause).message;
  }
}

function onVisibilityChange() {
  pageVisible.value = !document.hidden;
}

watch(visible, (value) => { void reportVisibility(value); });
watch(
  () => `${props.workspaceRef?.checkoutId ?? ""}:${props.workspaceRef?.expectedGeneration ?? ""}:${props.path}`,
  () => {
    snapshot.value = null;
    shownPages.value = [];
    void open();
  },
  { immediate: true },
);

onMounted(() => {
  document.addEventListener("visibilitychange", onVisibilityChange);
  void subscribeLatexPreviewUpdated((next) => {
    if (next.docId === snapshot.value?.docId) applySnapshot(next);
  }).then((release) => {
    if (disposed) release();
    else unsubscribe = release;
  });
});

onBeforeUnmount(() => {
  disposed = true;
  openVersion += 1;
  document.removeEventListener("visibilitychange", onVisibilityChange);
  unsubscribe?.();
  unsubscribe = null;
  void releaseSubscription();
});

const showPdf = computed(() => {
  const current = snapshot.value;
  if (!current?.pdfUrl) return false;
  return current.mode === "pdf" || viewMode.value === "pdf" || shownPages.value.length === 0;
});
const showSvg = computed(() => !showPdf.value && shownPages.value.length > 0);
const pdfSrc = computed(() => snapshot.value?.pdfUrl ?? "");

function seconds(ms: number | null | undefined): string {
  return ms == null ? "" : (ms / 1000).toFixed(1);
}

function clock(ms: number): string {
  return ms ? new Date(ms).toLocaleTimeString() : "";
}

const statusText = computed(() => {
  const current = snapshot.value;
  if (opening.value && !current) return t("latexPreview.status.opening");
  if (!current) return "";
  switch (current.status) {
    case "building":
      return current.mode === "svg" && current.revision > 0 && shownPages.value.length
        ? t("latexPreview.status.buildingWithPreview")
        : t("latexPreview.status.building");
    case "pending":
      return t("latexPreview.status.pending");
    case "error":
      return current.revision > 0 ? t("latexPreview.status.errorKeepsLast") : t("latexPreview.status.error");
    case "ready": {
      const parts = [t("latexPreview.status.updatedAt", clock(current.updatedAt))];
      if (current.mode === "svg" && current.svgMs != null) parts.push(`SVG ${seconds(current.svgMs)} s`);
      if (current.pdfMs != null) parts.push(`PDF ${seconds(current.pdfMs)} s`);
      return parts.join(" · ");
    }
    default:
      return t("latexPreview.status.idle");
  }
});

const modeLabel = computed(() => {
  const current = snapshot.value;
  if (!current?.mode) return "";
  if (current.mode === "pdf") return t("latexPreview.mode.pdfFallback");
  return showPdf.value ? "PDF" : "SVG";
});

/** User-facing text for the backend's stable fallback codes. */
function fallbackText(code: string | null, detail: string | null): string {
  if (!code) return "";
  const [kind, name] = code.includes(":") ? code.split(/:(.*)/, 2) : [code, ""];
  switch (kind) {
    case "package":
      return t("latexPreview.fallback.package", name);
    case "class":
      return t("latexPreview.fallback.class", name);
    case "engine":
      return t("latexPreview.fallback.engine", name);
    case "pdf_specials":
      return t("latexPreview.fallback.pdfSpecials", detail ?? "");
    case "svg_build_failed":
      return t("latexPreview.fallback.svgBuildFailed", detail ?? "");
    case "svg_disabled":
    case "dvisvgm_missing":
    case "pdfoutput":
    case "sticky":
    case "dvisvgm_failed":
    case "dvisvgm_dropped_content":
    case "no_pages":
      return t(`latexPreview.fallback.${kind}`);
    default:
      return t("latexPreview.fallback.unknown", code);
  }
}

const fallbackLabel = computed(() => (
  snapshot.value?.mode === "pdf"
    ? fallbackText(snapshot.value.fallbackReason, snapshot.value.fallbackDetail)
    : ""
));
const errors = computed<LatexDiagnostic[]>(() => snapshot.value?.errors ?? []);
const warnings = computed<LatexDiagnostic[]>(() => snapshot.value?.warnings ?? []);

function location(diagnostic: LatexDiagnostic): string {
  const file = diagnostic.file ? diagnostic.file.replace(/\\/g, "/").split("/").pop() : "";
  if (file && diagnostic.line) return `${file}:${diagnostic.line}`;
  if (diagnostic.line) return t("latexPreview.diagnostic.line", diagnostic.line);
  return file ?? "";
}

const emptyText = computed(() => {
  if (openError.value) return "";
  const current = snapshot.value;
  if (!current || current.status === "idle" || (current.status === "building" && current.revision === 0)) {
    return t("latexPreview.empty.building");
  }
  if (current.status === "pending") return t("latexPreview.status.pending");
  if (current.status === "error" && current.revision === 0) return t("latexPreview.empty.failed");
  return "";
});

const viewOptions = computed(() => [
  { value: "auto", label: "SVG" },
  { value: "pdf", label: "PDF" },
]);

function pageStyle(page: LatexPageRef): Record<string, string> {
  if (!page.width || !page.height) return {};
  // Points to CSS pixels at 100 %; narrower panes scale the page down.
  return {
    aspectRatio: `${page.width} / ${page.height}`,
    width: `min(100%, ${Math.round(page.width * (4 / 3))}px)`,
  };
}
</script>

<template>
  <section class="latex-preview" :aria-busy="snapshot?.building || opening">
    <header class="latex-preview-toolbar">
      <span v-if="modeLabel" class="latex-preview-mode" :class="{ fallback: snapshot?.mode === 'pdf' }">{{ modeLabel }}</span>
      <span class="latex-preview-status" :class="{ error: snapshot?.status === 'error' }">
        <span v-if="snapshot?.building" class="latex-preview-spinner" aria-hidden="true" />
        {{ statusText }}
      </span>
      <span v-if="fallbackLabel" class="latex-preview-fallback" :title="snapshot?.fallbackDetail || fallbackLabel">
        {{ fallbackLabel }}
      </span>
      <span class="latex-preview-spacer" />
      <BaseSegmented
        v-if="snapshot?.mode === 'svg' && snapshot?.pdfUrl"
        v-model="viewMode"
        size="sm"
        :options="viewOptions"
      />
      <BaseButton
        v-if="warnings.length"
        size="sm"
        :aria-expanded="showDetails"
        @click="showDetails = !showDetails"
      >
        {{ t("latexPreview.action.warnings", warnings.length) }}
      </BaseButton>
      <BaseButton size="sm" :disabled="opening" @click="rebuild">
        {{ t("latexPreview.action.rebuild") }}
      </BaseButton>
    </header>

    <div v-if="openError" class="latex-preview-notice error" role="alert">{{ openError }}</div>

    <div v-if="errors.length" class="latex-preview-diagnostics error" role="alert">
      <div v-for="(diagnostic, index) in errors" :key="`e${index}`" class="latex-preview-diagnostic">
        <span v-if="location(diagnostic)" class="latex-preview-location">{{ location(diagnostic) }}</span>
        <span class="latex-preview-message">{{ diagnostic.message }}</span>
        <code v-if="diagnostic.context" class="latex-preview-context">{{ diagnostic.context }}</code>
      </div>
    </div>
    <div v-if="showDetails && warnings.length" class="latex-preview-diagnostics warning">
      <div v-for="(diagnostic, index) in warnings" :key="`w${index}`" class="latex-preview-diagnostic">
        <span v-if="location(diagnostic)" class="latex-preview-location">{{ location(diagnostic) }}</span>
        <span class="latex-preview-message">{{ diagnostic.message }}</span>
      </div>
    </div>

    <div class="latex-preview-body">
      <div v-if="showSvg" class="latex-preview-pages">
        <img
          v-for="(page, index) in shownPages"
          :key="index"
          class="latex-preview-page"
          :src="page.url"
          :style="pageStyle(page)"
          :alt="t('latexPreview.pageAlt', index + 1)"
          draggable="false"
          loading="lazy"
        />
      </div>
      <iframe
        v-else-if="showPdf"
        class="latex-preview-pdf"
        :src="pdfSrc"
        :title="t('latexPreview.pdfTitle')"
      />
      <div v-else-if="emptyText" class="latex-preview-empty">{{ emptyText }}</div>
    </div>
  </section>
</template>

<style scoped>
.latex-preview {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
}
.latex-preview-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 34px;
  padding: 4px 10px;
  border-bottom: 1px solid var(--border-color);
  font-size: 12px;
  color: var(--text-secondary);
}
.latex-preview-mode {
  padding: 1px 6px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  color: var(--text-color);
  font-weight: 500;
}
.latex-preview-mode.fallback {
  border-color: color-mix(in srgb, var(--warning-color, #d29922) 60%, transparent);
  color: var(--warning-color, #d29922);
}
.latex-preview-status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}
.latex-preview-status.error {
  color: var(--danger-color, #e5484d);
}
.latex-preview-fallback {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.latex-preview-spacer {
  flex: 1;
}
.latex-preview-spinner {
  width: 10px;
  height: 10px;
  border: 1.5px solid var(--border-strong, var(--border-color));
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: latex-preview-spin 0.9s linear infinite;
}
@keyframes latex-preview-spin {
  to {
    transform: rotate(360deg);
  }
}
.latex-preview-notice {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  font-size: 12px;
  color: var(--text-secondary);
}
.latex-preview-notice.error {
  color: var(--danger-color, #e5484d);
}
.latex-preview-diagnostics {
  max-height: 30%;
  overflow: auto;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border-color);
  font-size: 12px;
}
.latex-preview-diagnostics.error {
  background: color-mix(in srgb, var(--danger-color, #e5484d) 8%, transparent);
}
.latex-preview-diagnostic {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 4px 8px;
  padding: 2px 0;
}
.latex-preview-location {
  font-family: var(--font-mono, monospace);
  color: var(--text-secondary);
}
.latex-preview-message {
  color: var(--text-color);
}
.latex-preview-context {
  flex-basis: 100%;
  font-family: var(--font-mono, monospace);
  color: var(--text-secondary);
  white-space: pre-wrap;
}
.latex-preview-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--sidebar-bg) 30%);
}
.latex-preview-pages {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  padding: 16px;
}
.latex-preview-page {
  display: block;
  max-width: 100%;
  height: auto;
  background: #fff;
  box-shadow: 0 1px 4px rgb(0 0 0 / 18%);
  user-select: none;
}
.latex-preview-pdf {
  flex: 1;
  width: 100%;
  min-height: 0;
  border: 0;
  background: var(--panel-bg);
}
.latex-preview-empty {
  margin: auto;
  padding: 24px;
  font-size: 13px;
  color: var(--text-secondary);
}
</style>
