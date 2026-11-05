import { Prec } from "@codemirror/state";
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { isolateHistory } from "@codemirror/commands";
import type { WorkspaceRef } from "../../../services/project";
import { importMarkdownImage, MARKDOWN_IMAGE_DIRECTORY_UNSET } from "../../../services/markdownImage";
import { normalizeAppError } from "../../../services/errors";
import { hasTauriWindowRuntime } from "../../../services/tauriRuntime";
import type { FloatingAnchorRect } from "./markdownFloating";

/** Asks the user where this project stores pasted images. */
export interface MarkdownImageFolderRequest {
  workspaceRef: WorkspaceRef;
  contentPath: string;
  /** Caret box at the insertion point, for anchoring the prompt. */
  anchor: FloatingAnchorRect | null;
}

export function markdownImagePaste(options: {
  context: () => { workspaceRef: WorkspaceRef; contentPath: string; key: string } | null;
  onError: (error: unknown) => void;
  /**
   * Called when the project has no folder for the document yet. Resolves
   * `true` once a folder is saved; the import is then retried.
   */
  chooseFolder?: (request: MarkdownImageFolderRequest) => Promise<boolean>;
}) {
  return Prec.highest(ViewPlugin.fromClass(class {
    pending = new Set<{ from: number; to: number; cancelled: boolean }>();
    disposed = false;
    unlisten: (() => void) | null = null;
    constructor(readonly view: EditorView) {
      if (hasTauriWindowRuntime()) void import("@tauri-apps/api/webview").then(({ getCurrentWebview }) => getCurrentWebview().onDragDropEvent(event => {
        if (this.disposed || event.payload.type !== "drop" || this.view.state.readOnly) return;
        const x = event.payload.position.x / window.devicePixelRatio;
        const y = event.payload.position.y / window.devicePixelRatio;
        if (!this.view.dom.contains(document.elementFromPoint(x, y))) return;
        const files = event.payload.paths.filter(path => /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(path));
        if (files.length) this.importAt(files, this.view.posAtCoords({ x, y }));
      })).then(release => { if (this.disposed) release(); else this.unlisten = release; }).catch(options.onError);
    }
    update(update: ViewUpdate) {
      if (!update.docChanged) return;
      for (const range of this.pending) {
        if (range.from !== range.to && update.changes.touchesRange(range.from, range.to)) range.cancelled = true;
        range.from = update.changes.mapPos(range.from, 1);
        range.to = Math.max(range.from, update.changes.mapPos(range.to, -1));
      }
    }
    destroy() { this.disposed = true; this.unlisten?.(); }
    anchorAt(position: number): FloatingAnchorRect | null {
      try {
        const rect = this.view.coordsAtPos(Math.min(position, this.view.state.doc.length));
        return rect ? { left: rect.left, top: rect.top, bottom: rect.bottom } : null;
      } catch {
        return null; // Not laid out; the editor anchors the prompt itself.
      }
    }
    receive(event: ClipboardEvent | DragEvent) {
      const data = "clipboardData" in event ? event.clipboardData : event.dataTransfer;
      const files = Array.from(data?.files ?? []).filter(file => file.type.startsWith("image/") || /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(file.name));
      if (!options.context() || this.view.state.readOnly || !files.length) return false;
      event.preventDefault(); event.stopPropagation();
      const position = "clientX" in event ? this.view.posAtCoords({ x: event.clientX, y: event.clientY }) : null;
      this.importAt(files, position);
      return true;
    }
    importAt(files: (File | string)[], position: number | null) {
      const current = options.context();
      const context = current ? { ...current, workspaceRef: { ...current.workspaceRef } } : null;
      if (!context) return;
      const selection = this.view.state.selection.main;
      const range = { from: position ?? selection.from, to: position ?? selection.to, cancelled: false };
      this.pending.add(range);
      const importAll = () => Promise.all(files.map(file => importMarkdownImage(context.workspaceRef, context.contentPath, file)));
      void importAll()
        .catch(async (error: unknown) => {
          // The first paste of a project without an image folder asks for one.
          if (!options.chooseFolder || normalizeAppError(error).code !== MARKDOWN_IMAGE_DIRECTORY_UNSET) throw error;
          if (this.disposed) return null;
          const chosen = await options.chooseFolder({ workspaceRef: context.workspaceRef, contentPath: context.contentPath, anchor: this.anchorAt(range.from) });
          return chosen && !this.disposed && !range.cancelled ? importAll() : null;
        })
        .then(images => {
          if (!images || this.disposed || range.cancelled || this.view.state.readOnly || options.context()?.key !== context.key) return;
          // Images are standalone blocks. A bare insertion at the end of a
          // reference definition/list marker can otherwise change Markdown syntax.
          const doc = this.view.state.doc;
          const before = doc.sliceString(doc.lineAt(range.from).from, range.from).trim();
          const after = doc.sliceString(range.to, doc.lineAt(range.to).to).trim();
          const insert = `${before ? "\n\n" : ""}${images.map(image => `![图片](${image.relativePath})`).join("\n\n")}${after ? "\n\n" : "\n"}`;
          this.pending.delete(range);
          const atSelection = this.view.state.selection.main.from === range.from && this.view.state.selection.main.to === range.to;
          this.view.dispatch({ changes: { from: range.from, to: range.to, insert },
            ...(atSelection ? { selection: { anchor: range.from + insert.length } } : {}),
            annotations: isolateHistory.of("full"), userEvent: "input.paste", scrollIntoView: atSelection });
        }).catch(options.onError).finally(() => this.pending.delete(range));
    }
  }, { eventHandlers: { paste(event) { return this.receive(event); }, drop(event) { return this.receive(event); } } }));
}
