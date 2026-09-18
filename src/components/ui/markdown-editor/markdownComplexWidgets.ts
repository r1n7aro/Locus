import { EditorSelection } from "@codemirror/state";
import { EditorView, WidgetType } from "@codemirror/view";
import { renderMarkdownMathHtml } from "../../../composables/markdownMath";
import type { WorkspaceRef } from "../../../services/project";
import {
  markdownImageDirectSrc,
  shouldResolveMarkdownImageSource,
} from "../../../composables/markdownImages";
import type {
  MarkdownReferenceToken,
} from "./markdownComplexTokens";
import type { MarkdownEditTarget } from "./markdownEditTarget";
import { markdownNodeAt } from "./markdownVisualCommands";

export interface MarkdownImageResolution {
  url: string;
  displayPath?: string;
  mimeType?: string;
}

export interface MarkdownImageResolverContext {
  /** Include workspace generation/document scope so cached paths cannot leak
   * across checkouts. */
  cacheKey?: string;
  contentPath?: string;
  workspaceRef?: WorkspaceRef | null;
}

export type MarkdownImageResolver = (
  source: string,
  context: MarkdownImageResolverContext,
) => MarkdownImageResolution | string | null | Promise<MarkdownImageResolution | string | null>;

export interface MarkdownLivePreviewOptions {
  onEditTarget?: (target: MarkdownEditTarget) => void;
  imageResolver?: MarkdownImageResolver;
  imageContext?: MarkdownImageResolverContext;
  onReferenceOpen?: (reference: MarkdownReferenceToken) => void | Promise<void>;
  onReferencePointerDown?: (
    reference: MarkdownReferenceToken,
    event: PointerEvent,
    element: HTMLElement,
  ) => void;
}

function activateSourceRange(
  view: EditorView,
  from: number,
  to: number,
  anchor = from,
): void {
  const safeAnchor = Math.max(from, Math.min(to, anchor));
  view.focus();
  view.dispatch({
    selection: EditorSelection.cursor(safeAnchor),
    effects: EditorView.scrollIntoView(safeAnchor, { y: "nearest" }),
  });
}

function installSourceActivation(
  dom: HTMLElement,
  view: EditorView,
  from: number,
  to: number,
  anchor = from,
): void {
  dom.tabIndex = -1;
  dom.setAttribute("role", "button");
  dom.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    activateSourceRange(view, from, to, anchor);
  });
}

export class CollapsedSourceWidget extends WidgetType {
  eq(): boolean {
    return true;
  }

  toDOM(): HTMLElement {
    const span = document.createElement("span");
    span.className = "cm-live-collapsed-source";
    span.setAttribute("aria-hidden", "true");
    return span;
  }
}

type ResolvedImage = MarkdownImageResolution & { state: "ready" } | { state: "error" };
const IMAGE_CACHE_LIMIT = 256;
const imageResolverCaches = new WeakMap<MarkdownImageResolver, Map<string, Promise<ResolvedImage>>>();

function safeImageUrl(source: string): string | null {
  const normalized = source.startsWith("//") ? `https:${source}` : source.trim();
  if (/^https?:\/\//i.test(normalized)) return normalized;
  if (/^blob:/i.test(normalized)) return normalized;
  if (/^data:image\/[a-z0-9.+-]+(?:;[a-z0-9=+/-]+)*,/i.test(normalized)) return normalized;
  return null;
}

function normalizeImageResolution(
  value: MarkdownImageResolution | string | null,
): ResolvedImage {
  const resolution = typeof value === "string" ? { url: value } : value;
  const url = resolution ? safeImageUrl(resolution.url) : null;
  if (!resolution || !url) return { state: "error" };
  return {
    ...resolution,
    url,
    state: "ready",
  };
}

function resolveImageCached(
  source: string,
  resolver: MarkdownImageResolver,
  context: MarkdownImageResolverContext,
): Promise<ResolvedImage> {
  let cache = imageResolverCaches.get(resolver);
  if (!cache) {
    cache = new Map();
    imageResolverCaches.set(resolver, cache);
  }
  const key = `${context.cacheKey ?? ""}\u0000${context.contentPath ?? ""}\u0000${source}`;
  const cached = cache.get(key);
  if (cached) {
    cache.delete(key);
    cache.set(key, cached);
    return cached;
  }

  const pending = Promise.resolve()
    .then(() => resolver(source, context))
    .then(normalizeImageResolution)
    .catch((): ResolvedImage => ({ state: "error" }));
  cache.set(key, pending);
  while (cache.size > IMAGE_CACHE_LIMIT) {
    const oldest = cache.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
  return pending;
}

export class MarkdownImageWidget extends WidgetType {
  constructor(
    private readonly source: string,
    private readonly alt: string,
    private readonly sourceFrom: number,
    private readonly sourceTo: number,
    private readonly options: MarkdownLivePreviewOptions,
  ) {
    super();
  }

  eq(other: MarkdownImageWidget): boolean {
    return other.source === this.source
      && other.alt === this.alt
      && other.sourceFrom === this.sourceFrom
      && other.sourceTo === this.sourceTo
      && other.options.imageResolver === this.options.imageResolver
      && other.options.imageContext?.cacheKey === this.options.imageContext?.cacheKey
      && other.options.imageContext?.contentPath === this.options.imageContext?.contentPath;
  }

  toDOM(view: EditorView): HTMLElement {
    const frame = document.createElement("span");
    frame.className = "cm-live-image-frame";
    frame.dataset.state = "loading";
    frame.dataset.source = this.source;
    frame.title = this.source;

    const image = document.createElement("img");
    image.className = "cm-live-image";
    image.alt = this.alt;
    // CodeMirror already limits widgets to the viewport. A lazy image hidden
    // until its load event would never start loading in Chromium.
    image.loading = "eager";
    image.decoding = "async";
    image.draggable = false;

    const fallback = document.createElement("span");
    fallback.className = "cm-live-image-fallback";
    const sourceSegments = this.source.split(/[\\/]/);
    fallback.textContent = this.alt.trim() || sourceSegments[sourceSegments.length - 1] || "Image";
    frame.append(image, fallback);
    frame.tabIndex = view.state.readOnly ? -1 : 0;
    frame.setAttribute("role", "button");
    frame.setAttribute("aria-label", this.alt || "编辑图片");
    const editImage = () => {
      if (view.state.readOnly) return;
      const title = markdownNodeAt(view.state, this.sourceFrom, ["Image"])?.getChild("LinkTitle");
      this.options.onEditTarget?.({
        kind: "image", from: this.sourceFrom, to: this.sourceTo,
        source: view.state.sliceDoc(this.sourceFrom, this.sourceTo),
        label: this.alt, url: this.source,
        title: title ? view.state.sliceDoc(title.from + 1, title.to - 1) : "",
      });
    };
    frame.addEventListener("click", (event) => { event.preventDefault(); event.stopPropagation(); editImage(); });
    frame.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") { event.preventDefault(); editImage(); }
    });

    const applyReady = (resolved: MarkdownImageResolution) => {
      if (frame.dataset.disposed === "true") return;
      frame.dataset.state = "loading";
      frame.title = resolved.displayPath || this.source;
      image.src = resolved.url;
    };
    const applyError = () => {
      if (frame.dataset.disposed === "true") return;
      frame.dataset.state = "error";
      view.requestMeasure();
    };
    image.addEventListener("load", () => {
      if (frame.dataset.disposed === "true") return;
      frame.dataset.state = "ready";
      view.requestMeasure();
    });
    image.addEventListener("error", applyError);

    if (!shouldResolveMarkdownImageSource(this.source)) {
      const direct = safeImageUrl(markdownImageDirectSrc(this.source));
      if (direct) applyReady({ url: direct, displayPath: this.source });
      else applyError();
    } else if (this.options.imageResolver) {
      void resolveImageCached(
        this.source,
        this.options.imageResolver,
        this.options.imageContext ?? {},
      ).then((resolved) => {
        if (resolved.state === "ready") applyReady(resolved);
        else applyError();
      });
    } else {
      frame.dataset.state = "unresolved";
    }
    return frame;
  }

  destroy(dom: HTMLElement): void {
    dom.dataset.disposed = "true";
  }
}

export class MarkdownMathWidget extends WidgetType {
  constructor(
    private readonly latex: string,
    private readonly display: boolean,
    private readonly sourceFrom: number,
    private readonly sourceTo: number,
    private readonly selectionAnchor: number,
  ) {
    super();
  }

  eq(other: MarkdownMathWidget): boolean {
    return other.latex === this.latex
      && other.display === this.display
      && other.sourceFrom === this.sourceFrom
      && other.sourceTo === this.sourceTo;
  }

  toDOM(view: EditorView): HTMLElement {
    const math = document.createElement("span");
    math.className = this.display ? "cm-live-math cm-live-math-display" : "cm-live-math";
    math.dataset.state = "loading";
    math.textContent = this.latex;
    math.setAttribute("aria-label", `Math: ${this.latex}`);
    installSourceActivation(
      math,
      view,
      this.sourceFrom,
      this.sourceTo,
      this.selectionAnchor,
    );
    void renderMarkdownMathHtml(this.latex, this.display).then((html) => {
      if (math.dataset.disposed === "true") return;
      // renderMarkdownMathHtml returns formula-scoped sanitized KaTeX output.
      math.innerHTML = html;
      math.dataset.state = "ready";
      view.requestMeasure();
    }).catch(() => {
      if (math.dataset.disposed === "true") return;
      math.dataset.state = "error";
      math.textContent = this.latex;
    });
    return math;
  }

  destroy(dom: HTMLElement): void {
    dom.dataset.disposed = "true";
  }
}

const REFERENCE_KIND_LABELS: Record<MarkdownReferenceToken["kind"], string> = {
  knowledge: "Knowledge",
  "unity-asset": "Asset",
  "unity-scene-object": "Scene",
  workspace: "Workspace",
  file: "File",
  view: "View",
  "unity-property": "Property",
};

function splitUnitySceneObjectPath(path: string): {
  scenePath: string;
  objectPath: string;
} | null {
  const match = path.match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
  if (!match) return null;
  const scenePath = match[1]?.trim() ?? "";
  const objectPath = match[2]?.replace(/^\/+|\/+$/g, "") ?? "";
  return scenePath && objectPath ? { scenePath, objectPath } : null;
}

/**
 * Mirrors the semantic attributes emitted by MarkdownRenderer. The shared
 * workbench drag resolver can therefore treat Live Preview references exactly
 * like references in chat without reparsing editor source text.
 */
function applyReferenceDragSemantics(
  element: HTMLElement,
  reference: MarkdownReferenceToken,
): void {
  const path = reference.path.trim().replace(/\\/g, "/");
  if (!path || reference.kind === "view") return;

  element.draggable = true;
  element.dataset.entryKind = "file";
  if (reference.line) element.dataset.fileLine = String(reference.line);

  if (reference.kind === "knowledge") {
    const type = path.split("/", 1)[0]?.toLowerCase() ?? "";
    element.classList.add("md-file-ref", "md-knowledge-ref");
    element.dataset.knowledgeType = type;
    element.dataset.knowledgePath = path;
    element.dataset.entryKind = "knowledge";
    return;
  }

  const sceneObject = splitUnitySceneObjectPath(path);
  if (reference.kind === "unity-scene-object" || (reference.kind === "unity-property" && sceneObject)) {
    if (!sceneObject) return;
    element.classList.add("md-file-ref", "md-unity-scene-object-ref");
    element.dataset.filePath = path;
    element.dataset.scenePath = sceneObject.scenePath;
    element.dataset.sceneObjectPath = sceneObject.objectPath;
    return;
  }

  if (reference.kind === "unity-asset" || reference.kind === "unity-property") {
    if (!/^(?:Assets|Packages)\//i.test(path)) return;
    element.classList.add("md-file-ref", "md-unity-asset-ref");
    element.dataset.filePath = path;
    element.dataset.assetPath = path;
    return;
  }

  if (reference.kind === "workspace") {
    const folder = /\/$/.test(reference.raw.trim());
    element.classList.add("md-workspace-ref", folder ? "md-folder-ref" : "md-file-ref");
    element.dataset.workspacePath = path.replace(/\/+$/, "");
    element.dataset.entryKind = folder ? "folder" : "file";
    if (!folder) element.dataset.filePath = path;
    return;
  }

  element.classList.add("md-file-ref");
  element.dataset.filePath = path;
  if (/\/$/.test(reference.raw.trim())) {
    element.classList.add("md-folder-ref");
    element.dataset.entryKind = "folder";
  }
}

export class MarkdownReferenceWidget extends WidgetType {
  constructor(
    private readonly reference: MarkdownReferenceToken,
    private readonly sourceFrom: number,
    private readonly sourceTo: number,
    private readonly options: MarkdownLivePreviewOptions,
  ) {
    super();
  }

  eq(other: MarkdownReferenceWidget): boolean {
    return other.reference.kind === this.reference.kind
      && other.reference.path === this.reference.path
      && other.reference.label === this.reference.label
      && other.reference.line === this.reference.line
      && other.sourceFrom === this.sourceFrom
      && other.sourceTo === this.sourceTo
      && other.options.onReferenceOpen === this.options.onReferenceOpen
      && other.options.onReferencePointerDown === this.options.onReferencePointerDown;
  }

  toDOM(view: EditorView): HTMLElement {
    const ref = document.createElement("span");
    ref.className = `cm-live-reference cm-live-reference--${this.reference.kind}`;
    ref.dataset.referenceKind = this.reference.kind;
    ref.dataset.referencePath = this.reference.path;
    ref.title = this.reference.path;
    applyReferenceDragSemantics(ref, this.reference);

    const kind = document.createElement("span");
    kind.className = "cm-live-reference-kind";
    kind.textContent = REFERENCE_KIND_LABELS[this.reference.kind];
    const label = document.createElement("span");
    label.className = "cm-live-reference-label";
    label.textContent = `${this.reference.label}${this.reference.line ? `:${this.reference.line}` : ""}`;
    ref.append(kind, label);

    ref.tabIndex = view.state.readOnly ? -1 : 0;
    ref.setAttribute("role", "button");
    ref.addEventListener("pointerdown", (event) => {
      this.options.onReferencePointerDown?.(this.reference, event, ref);
    });
    ref.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if ((event.ctrlKey || event.metaKey || view.state.readOnly) && this.options.onReferenceOpen) {
        void Promise.resolve(this.options.onReferenceOpen(this.reference)).catch(() => undefined);
        return;
      }
      if (!view.state.readOnly) this.options.onEditTarget?.({
        kind: "reference", from: this.reference.from, to: this.reference.to,
        source: view.state.sliceDoc(this.reference.from, this.reference.to),
        label: this.reference.label, url: this.reference.path, reference: this.reference,
      });
    });
    ref.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        ref.click();
      }
    });
    return ref;
  }
}
