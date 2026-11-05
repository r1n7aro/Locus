
<script setup lang="ts">
import { computed, getCurrentInstance, h, nextTick, onBeforeUnmount, onMounted, ref, render, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  markdownImageDirectSrc,
  prepareMarkdownImages,
  shouldResolveMarkdownImageSource,
} from "../composables/markdownImages";
import { escapeMarkdownHtml, markdownEngine } from "../composables/markdownEngine";
import { normalizeExternalMarkdownHref } from "../composables/markdownExternalLinks";
import { openSessionReference } from "../services/sessionReference";
import "../styles/markdown-links.css";
import {
  collectFileRefPathCandidates,
  collectInlineCodePathCandidates,
  injectAssetRefs,
  injectFileRefs,
  injectUnityObjectFenceRefs,
  injectUnityPropertyFenceRefs,
  injectViewRefs,
  injectWorkspaceMentions,
  normalizeMarkdownPathStatusKey,
  type MarkdownPathStatus,
} from "../composables/markdownInject";
import { resolveMathSentinels } from "../composables/markdownMath";
import { prepareMarkdownCitations } from "../composables/markdownCitations";
import { normalizeMarkdownForRender } from "../composables/markdownRender";
import { sanitizeRenderedMarkdownHtml } from "../composables/markdownSanitize";
import {
  loadCachedMarkdownPathStatuses,
  peekCachedMarkdownPathStatuses,
} from "../composables/markdownPathStatusCache";
import { markdownPathStatusWorkingDir } from "../composables/markdownPathStatusPrefetch";
import { useSurfaceContentReadiness } from "../composables/surfaceContentReadiness";
import { useSurfaceHidden } from "../composables/surfaceVisibility";
import {
  handleMarkdownReferenceSelectStart,
  installMarkdownReferenceCopy,
} from "../composables/markdownReferenceSelection";
import { useInternalDragController } from "../composables/useInternalDrag";
import { useTextViewerZoom } from "../composables/useTextViewerZoom";
import { workspaceFileChangeInScope } from "../composables/useFileChangeRevalidation";
import { collectMarkdownFileDependencies, MarkdownFileDependencies } from "../composables/markdownFileDependencies";
import { resolveMarkdownImage } from "../services/markdownImage";
import { subscribeWorkspaceFileChanges } from "../services/workspaceExplorer";
import type { RuntimeUnsubscribe } from "../services/locusRuntime";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";
import { normalizeViewError, viewTree, type ViewPackageSummary } from "../services/view";
import { useOpenView } from "../composables/useOpenView";
import { useNotificationStore } from "../stores/notification";
import { useProjectStore } from "../stores/project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { t } from "../i18n";
import { resolveLocusViewIcon } from "./icons/locusViewIcons";
import UnityObjectPreview from "./unity-preview/UnityObjectPreview.vue";
import UnityPropertyFenceBlock from "./unity/UnityPropertyFenceBlock.vue";
import { openImageLightbox } from "./ui/imageLightboxService";
import type { Citation } from "../types";
import type { WorkspaceRef } from "../services/project";
import { isRemoteClient } from "../services/remoteClient";
import type { UnityObjectPreviewInput, UnityObjectPreviewLevel } from "./unity-preview";
import { startWorkbenchReferenceInternalDrag } from "./workbench/workbenchReferenceDrag";

const props = withDefaults(defineProps<{
  content: string;
  citations?: Citation[];
  cursor?: boolean;
  enableFileRefs?: boolean;
  highlightTerms?: string[];
  unityPreviewStateScope?: string | null;
  workspaceRef?: WorkspaceRef | null;
  /** Workspace path of the document being shown: relative images resolve
   * against its folder instead of the workspace root. */
  contentPath?: string | null;
  textZoom?: boolean;
}>(), {
  textZoom: false,
});

const emit = defineEmits<{
  (e: "openImage", src: string): void;
}>();

const rootRef = ref<HTMLElement | null>(null);
const notificationStore = useNotificationStore();
const openViewPackage = useOpenView();
const projectStore = useProjectStore();
const workspaceContextStore = useWorkspaceContextStore();
const markdownWorkspaceRef = computed(() => props.workspaceRef ?? workspaceContextStore.focusedWorkspaceRef);
const markdownWorkingDir = computed(() => markdownPathStatusWorkingDir(
  markdownWorkspaceRef.value,
  workspaceContextStore.checkoutsById,
  projectStore.workingDir,
));
const internalDrag = useInternalDragController();
const { textViewerZoomStyle, handleTextViewerZoomWheel } = useTextViewerZoom();
const viewRefSummaries = ref<ViewPackageSummary[]>([]);
const inlinePathStatuses = ref<Map<string, MarkdownPathStatus>>(new Map());
const surfaceContentReadiness = useSurfaceContentReadiness();
const surfaceHidden = useSurfaceHidden();
const markdownPreviewRevisionEpoch = ref(0);
const instance = getCurrentInstance();
const appContext = instance?.appContext ?? null;
const markdownUnityObjectPreviewHosts = new Set<HTMLElement>();
const markdownUnityPropertyFenceHosts = new Set<HTMLElement>();
const markdownUnityPropertyFenceSources = new WeakMap<HTMLElement, string>();
let markdownViewRefLoadRun = 0;
let markdownInlinePathStatusLoadRun = 0;
let markdownInlinePathStatusKey = "";
let releaseWorkspaceFileChanges: RuntimeUnsubscribe | null = null;
let markdownRendererUnmounted = false;
let fileDependencies = new MarkdownFileDependencies();
let fileRefreshTimer: ReturnType<typeof setTimeout> | null = null;
let invalidateMarkdownImages = false;

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
      // A <mark> inserted inside a math sentinel would break its expansion.
      if (node.nodeValue.includes("")) return NodeFilter.FILTER_REJECT;
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

const parsedMarkdownHtml = computed(() => {
  if (!props.content) return "";
  const content = prepareMarkdownCitations(props.content, props.citations);
  return markdownEngine.parse(normalizeMarkdownForRender(content)) as string;
});

// Paths whose status decides how this markdown renders: inline code paths, and
// bare-text paths when file references are enabled.
const inlineFilePathCandidates = computed(() => {
  const html = parsedMarkdownHtml.value;
  const candidates = collectInlineCodePathCandidates(html);
  if (!props.enableFileRefs) return candidates;
  const seen = new Set(candidates);
  for (const candidate of collectFileRefPathCandidates(html)) {
    if (seen.has(candidate)) continue;
    seen.add(candidate);
    candidates.push(candidate);
  }
  return candidates;
});

function resolveInlinePathStatus(path: string): MarkdownPathStatus | null | undefined {
  return inlinePathStatuses.value.get(normalizeMarkdownPathStatusKey(path));
}

function sameInlinePathStatuses(
  current: Map<string, MarkdownPathStatus>,
  next: Map<string, MarkdownPathStatus>,
): boolean {
  if (current.size !== next.size) return false;
  for (const [path, status] of next) {
    const previous = current.get(path);
    if (
      !previous
      || previous.path !== status.path
      || previous.exists !== status.exists
      || previous.entryKind !== status.entryKind
    ) return false;
  }
  return true;
}

function applyInlinePathStatuses(statuses: Map<string, MarkdownPathStatus>) {
  const next = new Map<string, MarkdownPathStatus>();
  for (const [path, status] of statuses) {
    next.set(normalizeMarkdownPathStatusKey(path), status);
  }
  // An unchanged revalidation must not re-render the markdown.
  if (!sameInlinePathStatuses(inlinePathStatuses.value, next)) inlinePathStatuses.value = next;
}

async function loadInlinePathStatuses() {
  const candidates = inlineFilePathCandidates.value;
  const focusedWorkspaceRef = markdownWorkspaceRef.value;
  const nextKey = `${focusedWorkspaceRef?.checkoutId ?? ""}@${focusedWorkspaceRef?.expectedGeneration ?? ""}@${focusedWorkspaceRef?.expectedMaterializationEpoch ?? ""}\n${markdownWorkingDir.value}\n${candidates.join("\n")}`;
  if (nextKey === markdownInlinePathStatusKey) return;
  markdownInlinePathStatusKey = nextKey;
  const run = ++markdownInlinePathStatusLoadRun;

  if (!candidates.length) {
    applyInlinePathStatuses(new Map());
    return;
  }
  const workspaceRef = focusedWorkspaceRef;
  if (!workspaceRef) {
    applyInlinePathStatuses(new Map());
    return;
  }

  // Known statuses render with the text itself, so a remounted transcript
  // never swaps code spans for file references after its first frame.
  const cached = peekCachedMarkdownPathStatuses(markdownWorkingDir.value, candidates, workspaceRef);
  applyInlinePathStatuses(cached.statuses);
  if (!cached.missing.length && !cached.stale) return;

  // Unknown statuses change the layout once resolved; the owning surface stays
  // unpresented until then. Stale ones only revalidate in the background.
  const release = cached.missing.length ? surfaceContentReadiness?.hold() : undefined;
  try {
    const statuses = await loadCachedMarkdownPathStatuses(
      markdownWorkingDir.value,
      candidates,
      workspaceRef,
    );
    if (run !== markdownInlinePathStatusLoadRun) return;
    applyInlinePathStatuses(statuses);
  } catch {
    // The last known statuses stay rendered.
  } finally {
    release?.();
  }
}

const renderedHtml = computed(() => {
  if (!props.content) return "";
  try {
    let html = parsedMarkdownHtml.value;
    html = prepareMarkdownImages(html);
    html = injectAssetRefs(html, {
      inlinePathStatus: resolveInlinePathStatus,
      requireInlinePathStatus: true,
    });
    html = injectUnityPropertyFenceRefs(html);
    html = injectUnityObjectFenceRefs(html);
    html = injectWorkspaceMentions(html);
    html = injectViewRefs(html, {
      openLabel: t("tool.view.open"),
      resolveViewRef(viewRef) {
        const view = resolveViewRefSummary(viewRef);
        if (!view) return null;
        return {
          id: view.id,
          icon: resolveLocusViewIcon(view.icon),
          iconName: view.icon,
        };
      },
    });
    if (props.enableFileRefs) {
      // Bare text is only a file reference when the file exists.
      html = injectFileRefs(html, {
        inlinePathStatus: resolveInlinePathStatus,
        requireInlinePathStatus: true,
      });
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
    // Math sentinels expand after sanitize: formula HTML is sanitized by its
    // own formula-scoped pass inside the resolver (see markdownMath.ts).
    return resolveMathSentinels(sanitizeRenderedMarkdownHtml(html));
  } catch {
    return sanitizeRenderedMarkdownHtml(escapeMarkdownHtml(props.content));
  }
});

function normalizeMarkdownUnityLevel(value?: string): UnityObjectPreviewLevel {
  switch (value) {
    case "row":
    case "thumbnail":
    case "inspector":
    case "editor":
      return value;
    default:
      return "inline";
  }
}

function markdownUnityLevelForHost(host: HTMLElement): UnityObjectPreviewLevel {
  const level = normalizeMarkdownUnityLevel(host.dataset.mdUnityLevel);
  if (host.dataset.mdUnityEditable === "true" && level === "inline") return "row";
  return level;
}

function isPassiveMarkdownUnityPreviewLevel(level: UnityObjectPreviewLevel): boolean {
  return level !== "inline";
}

function markdownUnityObjectModelFromHost(host: HTMLElement): UnityObjectPreviewInput | null {
  if (host.dataset.entryKind === "folder") return null;

  const refKind = host.dataset.mdUnityRefKind;
  if (refKind === "sceneObject" || host.classList.contains("md-unity-scene-object-ref")) {
    const scenePath = normalizeUnityRefDatasetPath(host.dataset.scenePath);
    const objectPath = normalizeUnityRefDatasetPath(host.dataset.sceneObjectPath);
    if (!scenePath || !objectPath) return null;
    return {
      kind: "sceneObject",
      path: `${scenePath}/${objectPath}`,
      writable: host.dataset.mdUnityEditable === "true" || undefined,
    };
  }

  const assetPath = normalizeUnityRefDatasetPath(host.dataset.assetPath || host.dataset.filePath);
  if (!assetPath) return null;
  return {
    kind: "asset",
    path: assetPath,
    writable: host.dataset.mdUnityEditable === "true" || undefined,
  };
}

function markdownUnityPreviewStateKeyPart(value: string): string {
  return encodeURIComponent(value.trim().replace(/\\/g, "/"));
}

function markdownUnityObjectPreviewStateKey(
  host: HTMLElement,
  index: number,
  model: UnityObjectPreviewInput,
  level: UnityObjectPreviewLevel,
): string | undefined {
  const scope = props.unityPreviewStateScope?.trim();
  if (!scope) return undefined;

  const refKind = model.kind ?? model.ref?.kind ?? host.dataset.mdUnityRefKind ?? "asset";
  const path = normalizeUnityRefDatasetPath(model.path ?? model.ref?.path);
  return [
    "unity-object-preview",
    scope,
    level,
    String(index),
    refKind,
    path.toLowerCase(),
  ].map(markdownUnityPreviewStateKeyPart).join("|");
}

function unmountMarkdownUnityObjectPreviews() {
  for (const host of markdownUnityObjectPreviewHosts) {
    render(null, host);
  }
  markdownUnityObjectPreviewHosts.clear();
}

function unmountMarkdownUnityPropertyFences() {
  for (const host of markdownUnityPropertyFenceHosts) {
    render(null, host);
  }
  markdownUnityPropertyFenceHosts.clear();
}

function mountMarkdownUnityObjectPreviews() {
  const root = rootRef.value;
  if (!root) {
    unmountMarkdownUnityObjectPreviews();
    return;
  }

  unmountMarkdownUnityObjectPreviews();
  const hosts = Array.from(root.querySelectorAll<HTMLElement>("[data-md-unity-object-preview='true']"));
  for (const [index, host] of hosts.entries()) {
    const model = markdownUnityObjectModelFromHost(host);
    if (!model) continue;
    const level = markdownUnityLevelForHost(host);
    if (isPassiveMarkdownUnityPreviewLevel(level)) {
      host.dataset.mdUnityPassive = "true";
      host.removeAttribute("draggable");
    }

    const vnode = h(UnityObjectPreview, {
      model,
      workspaceRef: props.workspaceRef ?? undefined,
      level,
      draggable: false,
      autoLoadPreview: true,
      previewRevision: String(markdownPreviewRevisionEpoch.value),
      previewStateKey: markdownUnityObjectPreviewStateKey(host, index, model, level),
    });
    if (appContext) {
      vnode.appContext = appContext;
    }
    host.replaceChildren();
    render(vnode, host);
    markdownUnityObjectPreviewHosts.add(host);
  }
}

function markdownUnityPropertySourceFromHost(host: HTMLElement): string {
  return host.querySelector("[data-md-unity-property-source='true']")?.textContent
    ?? markdownUnityPropertyFenceSources.get(host) ?? "";
}

function mountMarkdownUnityPropertyFences() {
  const root = rootRef.value;
  if (!root) {
    unmountMarkdownUnityPropertyFences();
    return;
  }

  unmountMarkdownUnityPropertyFences();
  const hosts = Array.from(root.querySelectorAll<HTMLElement>("[data-md-unity-property-fence='true']"));
  for (const host of hosts) {
    const source = markdownUnityPropertySourceFromHost(host);
    if (!source.trim()) continue;
    markdownUnityPropertyFenceSources.set(host, source);

    host.replaceChildren();
    renderMarkdownUnityPropertyFence(host, source);
    markdownUnityPropertyFenceHosts.add(host);
  }
}

// Fence blocks are mounted with `render()`, outside this component's tree, so
// they cannot inject the surface visibility; it is passed as a prop instead.
// Hidden workbench tabs then defer their Unity reads until they are shown.
function renderMarkdownUnityPropertyFence(host: HTMLElement, source: string) {
  const vnode = h(UnityPropertyFenceBlock, { source, surfaceHidden: surfaceHidden.value });
  if (appContext) {
    vnode.appContext = appContext;
  }
  render(vnode, host);
}

watch(surfaceHidden, () => {
  for (const host of markdownUnityPropertyFenceHosts) {
    const source = markdownUnityPropertyFenceSources.get(host);
    if (source !== undefined) renderMarkdownUnityPropertyFence(host, source);
  }
});

function refreshMarkdownEnhancements() {
  if (markdownRendererUnmounted) return;
  fileDependencies = collectMarkdownFileDependencies(
    rootRef.value, markdownWorkingDir.value, inlineFilePathCandidates.value,
    markdownUnityPropertySourceFromHost,
  );
  mountMarkdownUnityPropertyFences();
  mountMarkdownUnityObjectPreviews();
  void resolveMarkdownImages();
}

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
  const workspaceRef = markdownWorkspaceRef.value;
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

    const contentPath = props.contentPath || undefined;
    const cacheKey = `${workspaceRef?.checkoutId ?? ""}@${workspaceRef?.expectedGeneration ?? ""}@${workspaceRef?.expectedMaterializationEpoch ?? ""}:${contentPath ?? ""}:${source}`;
    const cached = markdownImageCache.get(cacheKey);
    if (cached) {
      applyResolvedMarkdownImage(image, source, cached);
      continue;
    }

    setMarkdownImageState(image, "loading");
    try {
      if (!workspaceRef) throw new Error("Workspace scope is required for relative Markdown images");
      const preview = await resolveMarkdownImage(workspaceRef, source, contentPath);
      const resolved = {
        url: preview.url,
        displayPath: preview.displayPath || source,
      };
      if (
        run !== markdownImageResolveRun
        || !image.isConnected
        || image.dataset.mdImageSource?.trim() !== source
      ) {
        continue;
      }
      markdownImageCache.set(cacheKey, resolved);
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
  inlineFilePathCandidates,
  () => {
    void loadInlinePathStatuses();
  },
  { immediate: true },
);

watch(
  () => [
    markdownWorkingDir.value,
    markdownWorkspaceRef.value?.checkoutId ?? "",
    markdownWorkspaceRef.value?.expectedGeneration ?? null,
    markdownWorkspaceRef.value?.expectedMaterializationEpoch ?? null,
    props.contentPath ?? null,
  ],
  () => {
    markdownImageResolveRun++;
    markdownImageCache.clear();
    if (fileRefreshTimer !== null) clearTimeout(fileRefreshTimer);
    fileRefreshTimer = null;
    invalidateMarkdownImages = false;
    fileDependencies = new MarkdownFileDependencies();
    for (const image of rootRef.value?.querySelectorAll<HTMLImageElement>("img[data-md-image-source]") ?? []) {
      delete image.dataset.mdImageResolvedFor;
    }
    void loadInlinePathStatuses();
    void nextTick(refreshMarkdownEnhancements);
  },
);

watch(
  renderedHtml,
  () => {
    void nextTick(refreshMarkdownEnhancements);
  },
  { immediate: true, flush: "post" },
);

void subscribeWorkspaceFileChanges((event) => {
  if (fileDependencies.empty || !workspaceFileChangeInScope(event, markdownWorkspaceRef.value)) return;
  const { path } = event.payload;
  if (!fileDependencies.matches(path)) return;
  invalidateMarkdownImages ||= fileDependencies.matches(path, true);
  if (fileRefreshTimer !== null) return;
  // Fixed window: a multi-path batch refreshes a referenced preview only once,
  // while a continuous stream cannot postpone its refresh indefinitely.
  fileRefreshTimer = setTimeout(() => {
    fileRefreshTimer = null;
    if (markdownRendererUnmounted) return;
    markdownPreviewRevisionEpoch.value += 1;
    if (invalidateMarkdownImages) {
      markdownImageResolveRun++;
      markdownImageCache.clear();
      for (const image of rootRef.value?.querySelectorAll<HTMLImageElement>("img[data-md-image-source]") ?? []) {
        delete image.dataset.mdImageResolvedFor;
      }
      invalidateMarkdownImages = false;
    }
    void nextTick(refreshMarkdownEnhancements);
  }, 100);
}, () => markdownWorkspaceRef.value).then((release) => {
  if (markdownRendererUnmounted) release();
  else releaseWorkspaceFileChanges = release;
});

// References join text selections; copying one writes its full path.
onMounted(() => {
  if (rootRef.value) installMarkdownReferenceCopy(rootRef.value.ownerDocument);
});

onBeforeUnmount(() => {
  markdownRendererUnmounted = true;
  if (fileRefreshTimer !== null) clearTimeout(fileRefreshTimer);
  fileRefreshTimer = null;
  releaseWorkspaceFileChanges?.();
  releaseWorkspaceFileChanges = null;
  markdownImageResolveRun++;
  unmountMarkdownUnityPropertyFences();
  unmountMarkdownUnityObjectPreviews();
});

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

function normalizeViewRefKey(value: string): string {
  return value.trim().replace(/\\/g, "/").replace(/\/+/g, "/").replace(/^\/+|\/+$/g, "").toLowerCase();
}

function viewMatchesRef(view: ViewPackageSummary, refKey: string): boolean {
  const candidates = [
    view.id,
    view.displayPath,
    view.packageRelPath,
    view.name,
  ];
  return candidates.some((candidate) => candidate && normalizeViewRefKey(candidate) === refKey);
}

function contentMayContainViewRef(content: string): boolean {
  return /(?:^|\n)\s*`?view:/i.test(content);
}

function resolveViewRefSummary(rawViewId: string): ViewPackageSummary | null {
  const refKey = normalizeViewRefKey(rawViewId);
  if (!refKey) return null;
  return viewRefSummaries.value.find((view) => viewMatchesRef(view, refKey)) ?? null;
}

async function loadMarkdownViewRefs() {
  const run = ++markdownViewRefLoadRun;
  try {
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!workspaceRef) return;
    const snapshot = await viewTree(workspaceRef);
    if (run === markdownViewRefLoadRun) {
      viewRefSummaries.value = snapshot.views;
    }
  } catch {
    if (run === markdownViewRefLoadRun) {
      viewRefSummaries.value = [];
    }
  }
}

watch(
  () => props.content,
  (content) => {
    if (!contentMayContainViewRef(content)) {
      markdownViewRefLoadRun++;
      viewRefSummaries.value = [];
      return;
    }
    void loadMarkdownViewRefs();
  },
  { immediate: true },
);

async function resolveMarkdownViewId(rawViewId: string): Promise<string> {
  const refKey = normalizeViewRefKey(rawViewId);
  if (!refKey) return rawViewId.trim();

  const cachedView = resolveViewRefSummary(rawViewId);
  if (cachedView) return cachedView.id;

  try {
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!workspaceRef) return rawViewId.trim();
    const snapshot = await viewTree(workspaceRef);
    return snapshot.views.find((view) => viewMatchesRef(view, refKey))?.id ?? rawViewId.trim();
  } catch {
    return rawViewId.trim();
  }
}

async function openMarkdownViewRef(button: HTMLButtonElement) {
  // Views run on the desktop; the phone client cannot host them.
  if (button.disabled || isRemoteClient()) return;
  const rawViewId = button.dataset.viewId?.trim();
  if (!rawViewId) return;

  const block = button.closest<HTMLElement>(".md-view-ref-block");
  button.disabled = true;
  if (block) block.dataset.mdViewOpening = "true";
  try {
    const viewId = await resolveMarkdownViewId(rawViewId);
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!workspaceRef) return;
    await openViewPackage(workspaceRef, viewId);
  } catch (error) {
    const err = normalizeViewError(error);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openViewFromMarkdownRef",
      replaceOperation: true,
    });
  } finally {
    button.disabled = false;
    if (block) delete block.dataset.mdViewOpening;
  }
}

function handleMarkdownContentActivation(event: MouseEvent) {
  if (event.defaultPrevented || !isHandledMarkdownMouseButton(event)) return;
  if (!(event.target instanceof Element)) return;

  const viewOpenButton = event.target.closest(".md-view-open-button[data-view-id]") as HTMLButtonElement | null;
  if (viewOpenButton) {
    if (event.type !== "click" || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    void openMarkdownViewRef(viewOpenButton);
    return;
  }

  const anchor = event.target.closest("a[href]") as HTMLAnchorElement | null;
  if (anchor) {
    event.preventDefault();
    event.stopPropagation();

    const rawHref = anchor.getAttribute("href");
    const sessionId = /^#locus-session\/([A-Za-z0-9_-]+)$/.exec(rawHref ?? "")?.[1];
    if (sessionId) {
      if (!isRemoteClient()) void openSessionReference(sessionId, anchor.textContent || sessionId);
      return;
    }
    const href = normalizeExternalMarkdownHref(rawHref);
    if (!href) return;

    void openMarkdownHref(href);
    return;
  }

  if (event.type !== "click" || event.button !== 0) return;
  const image = event.target.closest("img.md-image-preview[src]") as HTMLImageElement | null;
  if (!image || image.dataset.mdImageState !== "ready") return;

  event.preventDefault();
  event.stopPropagation();
  const src = image.currentSrc || image.src;
  // Hosts with their own viewer (Chat, mobile) take the image; documents float it in place.
  if (instance?.vnode.props?.onOpenImage) emit("openImage", src);
  else openImageLightbox({ src, alt: image.alt, originRect: image.getBoundingClientRect() });
}

function normalizeUnityRefDatasetPath(value?: string): string {
  return (value ?? "").trim().replace(/\\/g, "/").replace(/\/+$/g, "");
}

function handleMarkdownPointerDown(event: PointerEvent) {
  if (event.button !== 0 || event.isPrimary === false || isRemoteClient()) return;
  const workspaceRef = props.workspaceRef;
  if (!workspaceRef) return;
  const checkout = workspaceContextStore.checkoutsById[workspaceRef.checkoutId];
  if (!checkout?.projectId || !checkout.root) return;
  startWorkbenchReferenceInternalDrag(internalDrag, event, {
    projectId: checkout.projectId,
    workspaceRef: { ...workspaceRef },
    workspaceRoot: checkout.root,
  });
}

function handleMarkdownWheel(event: WheelEvent) {
  if (!props.textZoom) return;
  handleTextViewerZoomWheel(event);
}

// Bind wheel only for text zoom: a wheel listener is non-passive, so scrolling
// over every transcript message would wait for the main thread.
const markdownWheelListeners = computed(() => (props.textZoom ? { wheel: handleMarkdownWheel } : {}));
</script>

<template>
  <div
    ref="rootRef"
    class="markdown-body ui-select-text"
    :style="textZoom ? textViewerZoomStyle : undefined"
    @click="handleMarkdownContentActivation"
    @auxclick="handleMarkdownContentActivation"
    @pointerdown="handleMarkdownPointerDown"
    @selectstart="handleMarkdownReferenceSelectStart"
    v-on="markdownWheelListeners"
    v-html="renderedHtml"
  />
</template>

<style>
.markdown-body {
  font-family: var(--font-prose);
  font-size: calc(14px * var(--text-viewer-font-scale, 1));
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

/* Trim the document's outer edges. Consecutive `.markdown-segment` bodies
 * render one document together (streaming blocks), so only the first
 * segment's top and the last segment's bottom are trimmed; at inner
 * boundaries the margins collapse across the wrappers exactly as inside one
 * body. `:where` keeps the specificity of a plain `.markdown-body > :x`. */
.markdown-body:where(:not(.markdown-segment), .markdown-segment:first-child) > :first-child {
  margin-top: 0;
}

.markdown-body:where(:not(.markdown-segment), .markdown-segment:last-child) > :last-child {
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

.markdown-body hr {
  border: none;
  border-top: 1px solid var(--border-color);
  opacity: 0.8;
}

/* The frame draws the border and clips the rounded corners and edge shadows;
   .md-table-scroll inside it is the horizontal scroller. */
.markdown-body .md-table-wrap {
  --md-table-edge-shadow: rgba(17, 19, 24, 0.12);
  position: relative;
  isolation: isolate;
  width: fit-content;
  max-width: 100%;
  box-sizing: border-box;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

:root[data-theme="dark"] .markdown-body .md-table-wrap {
  --md-table-edge-shadow: rgba(0, 0, 0, 0.48);
}

.markdown-body .md-table-scroll {
  overflow-x: auto;
  overflow-y: hidden;
}

/* Auto width wraps long cells inside the available width; the table scrolls
   only once the per-column minimum widths no longer fit. */
.markdown-body table {
  width: auto;
  min-width: 100%;
  margin: 0;
  border-collapse: separate;
  border-spacing: 0;
  table-layout: auto;
  font-size: 0.9286em;
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

/* The first column stays pinned while wider tables scroll; the opaque cell
   backgrounds above keep scrolled columns from showing through. */
.markdown-body .md-table-scroll :is(th, td):first-child {
  position: sticky;
  left: 0;
  z-index: 1;
}

/* Edge shadows follow the scroll position without script: the right one shows
   while columns remain past the visible edge, the left one once columns have
   scrolled under the pinned first column. A table that fits has an inactive
   scroll timeline, so both keep their base opacity of 0. */
@supports (animation-timeline: scroll()) {
  .markdown-body .md-table-wrap {
    timeline-scope: --md-table-scroll;
  }

  .markdown-body .md-table-scroll {
    scroll-timeline: --md-table-scroll x;
  }

  .markdown-body .md-table-wrap::after,
  .markdown-body .md-table-scroll :is(th, td):first-child::after {
    content: "";
    position: absolute;
    top: 0;
    pointer-events: none;
    opacity: 0;
  }

  .markdown-body .md-table-wrap::after {
    right: 0;
    bottom: 0;
    z-index: 2;
    width: 20px;
    background: linear-gradient(to left, var(--md-table-edge-shadow), transparent);
    animation: md-table-end-shadow linear both;
    animation-timeline: --md-table-scroll;
  }

  .markdown-body .md-table-scroll :is(th, td):first-child::after {
    bottom: -1px;
    left: calc(100% + 1px);
    width: 14px;
    background: linear-gradient(to right, var(--md-table-edge-shadow), transparent);
    animation: md-table-start-shadow linear both;
    animation-timeline: --md-table-scroll;
  }
}

@keyframes md-table-end-shadow {
  0%,
  92% {
    opacity: 1;
  }

  100% {
    opacity: 0;
  }
}

@keyframes md-table-start-shadow {
  0% {
    opacity: 0;
  }

  8%,
  100% {
    opacity: 1;
  }
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
  font-size: 0.9286em;
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
  font-size: 0.8462em;
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

.markdown-body .md-image-preview[data-md-image-state="ready"] {
  cursor: zoom-in;
}

.markdown-body .md-image-preview[data-md-image-state="pending"],
.markdown-body .md-image-preview[data-md-image-state="loading"],
.markdown-body .md-image-preview[data-md-image-state="error"] {
  width: 100%;
  min-height: 110px;
  cursor: default;
}

.markdown-body .katex {
  font-size: 1.16em;
}

/* KaTeX displayMode roots are span.katex-display (block via CSS), so they
 * stay legal inside <p> while still needing the paragraph rhythm. */
.markdown-body .katex-display {
  margin: 4px 0 12px;
  padding: 2px 0;
  overflow-x: auto;
  overflow-y: hidden;
}

.markdown-body .md-math-pending,
.markdown-body .md-math-error {
  font-family: var(--font-mono-inline);
  font-size: 0.92em;
  color: var(--text-secondary);
}

.markdown-body .md-math-error {
  color: var(--danger, #d73a49);
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

.markdown-body .md-view-ref-block {
  width: fit-content;
  max-width: 100%;
  min-width: min(340px, 100%);
  min-height: 38px;
  margin: 4px 0 12px;
  padding: 6px 7px 6px 9px;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
  color: var(--text-color);
}

.markdown-body .md-view-ref-block[data-md-view-opening="true"] {
  border-color: color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  background: color-mix(in srgb, var(--accent-color) 5%, var(--panel-bg) 95%);
}

.markdown-body .md-view-ref-main {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
}

.markdown-body .md-view-ref-kind-icon {
  color: color-mix(in srgb, var(--accent-color) 74%, var(--text-color) 26%);
}

.markdown-body .md-view-ref-type {
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  line-height: 1;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  user-select: none;
}

.markdown-body .md-view-ref-id {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-inline);
  font-size: 13px;
  line-height: 1.35;
  color: var(--text-color);
}

.markdown-body .md-view-open-button {
  appearance: none;
  min-width: max-content;
  min-height: 24px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 0 8px;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 12px;
  line-height: 1;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease, opacity 0.12s ease;
}

.markdown-body .md-view-open-button:hover:not(:disabled),
.markdown-body .md-view-open-button:focus-visible {
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.markdown-body .md-view-open-button:disabled {
  cursor: wait;
  opacity: 0.58;
}

.markdown-body .md-view-open-icon {
  width: 13px;
  min-width: 13px;
  height: 13px;
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
}

.md-unity-asset-ref,
.md-unity-scene-object-ref {
  background: color-mix(in srgb, var(--sidebar-bg, var(--hover-bg)) 54%, transparent);
  border-color: color-mix(in srgb, var(--border-color) 78%, transparent);
  color: color-mix(in srgb, var(--text-color) 90%, var(--text-secondary) 10%);
}

.md-unity-object-ref {
  max-width: 100%;
}

.md-unity-object-ref[data-md-unity-object-preview="true"] {
  padding: 1px 6px 1px 5px;
}

.md-unity-object-ref[data-md-unity-level="row"],
.md-unity-object-ref[data-md-unity-level="thumbnail"],
.md-unity-object-ref[data-md-unity-level="inspector"],
.md-unity-object-ref[data-md-unity-level="editor"] {
  display: block;
  width: min(560px, 100%);
  min-height: 0;
  margin: 4px 0 12px;
  padding: 0;
  line-height: normal;
  vertical-align: baseline;
}

.md-unity-object-ref[data-md-unity-level="thumbnail"] {
  width: min(420px, 100%);
}

.md-unity-object-ref[data-md-unity-level="inspector"],
.md-unity-object-ref[data-md-unity-level="editor"] {
  width: min(720px, 100%);
}

.md-unity-object-ref[data-md-unity-level="row"] .unity-object-identity {
  min-height: 30px;
}

.md-unity-object-ref[data-md-unity-level="thumbnail"] .unity-object-preview,
.md-unity-object-ref[data-md-unity-level="inspector"] .unity-object-preview,
.md-unity-object-ref[data-md-unity-level="editor"] .unity-object-editor-panel {
  width: 100%;
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
}

/* Inline references join text selections that pass over them; a press on one
 * starts a reference drag instead (handleMarkdownReferenceSelectStart), and
 * copying writes its full path (markdownReferenceSelection). */
.markdown-body :is(.md-asset-chip, .md-file-ref, .md-workspace-ref, .md-unity-object-ref) {
  user-select: text;
  -webkit-user-select: text;
}

.markdown-body .md-unity-object-ref:not([data-md-unity-level="inline"]) {
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

.md-unity-object-ref[data-md-unity-level="thumbnail"],
.md-unity-object-ref[data-md-unity-level="inspector"],
.md-unity-object-ref[data-md-unity-level="editor"],
.md-unity-object-ref[data-md-unity-level="thumbnail"]:hover,
.md-unity-object-ref[data-md-unity-level="inspector"]:hover,
.md-unity-object-ref[data-md-unity-level="editor"]:hover {
  border: none;
  background: transparent;
  color: var(--text-color);
}

.md-unity-object-ref[data-md-unity-passive="true"] {
  cursor: default;
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
  display: inline-block;
  width: 0;
  margin-left: 0;
  overflow: visible;
  color: var(--accent-color);
  font-weight: 400;
  transform: translateX(1px);
  animation: streaming-cursor-blink 0.8s step-end infinite;
}

.markdown-body .md-citation {
  display: inline;
  margin-left: 2px;
  color: var(--accent-color);
  font-size: 0.78em;
  font-weight: 500;
  line-height: 1;
  text-decoration: none;
  white-space: nowrap;
}

.markdown-body a.md-citation:hover {
  color: color-mix(in srgb, var(--accent-color) 82%, var(--text-color));
  text-decoration: underline;
  text-underline-offset: 2px;
}

.markdown-body .md-citation sup {
  position: relative;
  top: -0.08em;
  vertical-align: baseline;
}

@keyframes streaming-cursor-blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0; }
}
</style>
