import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";
import type { WorkspaceRef } from "./project";

/** LaTeX preview service (`src-tauri/src/latex_preview`). */

export type LatexEngine = "xelatex" | "pdflatex" | "lualatex";

export interface LatexDiagnostic {
  severity: "error" | "warning";
  file?: string;
  line?: number;
  message: string;
  /** Source excerpt TeX printed after the error. */
  context?: string;
}

export interface LatexPageRef {
  url: string;
  /** Page size in points. */
  width?: number;
  height?: number;
}

/**
 * `pending`: the document changed while no preview was visible; it builds
 * when one becomes visible.
 */
export type LatexPreviewStatus = "idle" | "pending" | "building" | "ready" | "error";

export interface LatexPreviewSnapshot {
  docId: string;
  checkoutId: string;
  path: string;
  /** Root document compiled for `path` (differs with `% !TEX root`). */
  rootPath: string;
  status: LatexPreviewStatus;
  /** Output shown: SVG pages, or the PDF when SVG is unsupported. */
  mode: "svg" | "pdf" | null;
  revision: number;
  pages: LatexPageRef[];
  /** Latest successful PDF (the cache), also while SVG is shown. */
  pdfUrl: string | null;
  pdfRevision: number;
  /** Stable code: why the PDF is shown instead of SVG. */
  fallbackReason: string | null;
  fallbackDetail: string | null;
  engine: LatexEngine | null;
  errors: LatexDiagnostic[];
  warnings: LatexDiagnostic[];
  svgMs: number | null;
  pdfMs: number | null;
  building: boolean;
  updatedAt: number;
}

export interface LatexPreviewOpenResult {
  subscriptionId: string;
  snapshot: LatexPreviewSnapshot;
}

export interface LatexToolchain {
  xelatex: string | null;
  pdflatex: string | null;
  lualatex: string | null;
  dvisvgm: string | null;
  distribution: "miktex" | "texlive" | "unknown" | null;
  version: string | null;
}

/**
 * Opens a preview session; the backend starts on demand (first open) and
 * releases everything when the last session closes. Without a workspace the
 * file's own folder bounds the dependency watching.
 */
export function openLatexPreview(
  workspaceRef: WorkspaceRef | null,
  path: string,
  visible: boolean,
): Promise<LatexPreviewOpenResult> {
  return ipcInvoke<LatexPreviewOpenResult>("latex_preview_open", { workspaceRef, path, visible });
}

export function setLatexPreviewVisible(subscriptionId: string, visible: boolean): Promise<LatexPreviewSnapshot> {
  return ipcInvoke<LatexPreviewSnapshot>("latex_preview_set_visible", { subscriptionId, visible });
}

export function rebuildLatexPreview(subscriptionId: string): Promise<void> {
  return ipcInvoke<void>("latex_preview_rebuild", { subscriptionId });
}

export function closeLatexPreview(subscriptionId: string): Promise<void> {
  return ipcInvoke<void>("latex_preview_close", { subscriptionId });
}

let toolchainProbe: Promise<LatexToolchain> | null = null;

/** Detected toolchain, cached per window; `refresh` detects again (after installing TeX). */
export function getLatexToolchain(refresh = false): Promise<LatexToolchain> {
  if (refresh || !toolchainProbe) {
    const request = ipcInvoke<LatexToolchain>("latex_toolchain_status", { refresh });
    toolchainProbe = request;
    request.catch(() => {
      if (toolchainProbe === request) toolchainProbe = null;
    });
  }
  return toolchainProbe;
}

/** Whether a TeX engine is installed. `.tex` files open as source when it is not. */
export async function latexEngineAvailable(): Promise<boolean> {
  try {
    const toolchain = await getLatexToolchain();
    return !!(toolchain.xelatex || toolchain.pdflatex || toolchain.lualatex);
  } catch {
    // Unknown: show the preview, which reports the problem itself.
    return true;
  }
}

/** Every preview update of every document; filter by `docId`. */
export function subscribeLatexPreviewUpdated(
  handler: (snapshot: LatexPreviewSnapshot) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<LatexPreviewSnapshot>("latex-preview-updated", handler);
}

export function isLatexSourcePath(path: string | null | undefined): boolean {
  return !!path && /\.tex$/i.test(path);
}
