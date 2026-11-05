import { inject, onBeforeUnmount, shallowRef, type InjectionKey, type ShallowRef } from "vue";
import type { MarkdownEditorSelection } from "../components/ui/markdown-editor/markdownEditorSelection";
import type { WorkspaceRef } from "../services/project";
import type { UserMessageDraftKnowledgeQuote } from "./chatMessageDraft";

/**
 * Opens a new conversation beside the document's pane with the quote attached.
 * Provided by the Workbench; `workspaceRef` is null for files outside a checkout.
 */
export const DOCUMENT_QUOTE_IN_NEW_SESSION_KEY: InjectionKey<
  (workspaceRef: WorkspaceRef | null, quote: UserMessageDraftKnowledgeQuote) => Promise<void>
> = Symbol("document-quote-in-new-session");

/** The latest non-empty text selection in a document editor of one window. */
export interface DocumentSelectionSnapshot {
  /** Changes with every reported selection, so a dismissal applies to one snapshot only. */
  id: string;
  ownerId: string;
  /** Checkout that owns the document; null for files outside a workspace. */
  checkoutId: string | null;
  /** Type-prefixed knowledge path (`design/a.md`) when the document is project knowledge. */
  knowledgePath?: string;
  /** Absolute physical file path with forward slashes. */
  filePath: string;
  name: string;
  /** Short location shown in the composer, e.g. `notes.md:12-14`. */
  location: string;
  /** Prompt excerpt: physical location line followed by the fenced selected text. */
  content: string;
}

export type DocumentSelectionInput = Omit<DocumentSelectionSnapshot, "id" | "ownerId">;

export interface DocumentSelectionHub {
  readonly current: Readonly<ShallowRef<DocumentSelectionSnapshot | null>>;
  report(snapshot: DocumentSelectionSnapshot): void;
  clear(ownerId: string): void;
}

export const DOCUMENT_SELECTION_HUB_KEY: InjectionKey<DocumentSelectionHub> = Symbol("document-selection-hub");

export function createDocumentSelectionHub(): DocumentSelectionHub {
  const current = shallowRef<DocumentSelectionSnapshot | null>(null);
  return {
    current,
    report(snapshot) {
      current.value = snapshot;
    },
    clear(ownerId) {
      if (current.value?.ownerId === ownerId) current.value = null;
    },
  };
}

/** A composer attachment that follows the live document selection. */
export interface LiveSelectionQuote extends UserMessageDraftKnowledgeQuote {
  id: string;
  location: string;
}

/**
 * Knowledge paths resolve only inside the checkout that owns the document;
 * another conversation workspace receives the physical file path instead.
 */
export function liveSelectionQuoteForCheckout(
  selection: DocumentSelectionSnapshot,
  checkoutId: string | null | undefined,
): LiveSelectionQuote {
  const knowledgePath = selection.knowledgePath && checkoutId && selection.checkoutId === checkoutId
    ? selection.knowledgePath
    : "";
  return {
    id: selection.id,
    path: knowledgePath || selection.filePath,
    source: knowledgePath ? "knowledge" : "file",
    name: selection.name,
    location: selection.location,
    content: selection.content,
  };
}

export function normalizeSelectionFilePath(path: string): string {
  return path.trim().replace(/\\/g, "/");
}

function fencedExcerpt(location: string, text: string, language: string): string {
  let fenceLength = 3;
  for (const match of text.matchAll(/`+/g)) fenceLength = Math.max(fenceLength, match[0].length + 1);
  const fence = "`".repeat(fenceLength);
  return `${location}\n${fence}${language}\n${text}\n${fence}`;
}

export function selectionLineRange(startLine: number, endLine: number): string {
  return endLine > startLine ? `${startLine}-${endLine}` : `${startLine}`;
}

/** Excerpt for an editor whose document is the whole physical file. */
export function fileSelectionReference(
  filePath: string,
  selection: MarkdownEditorSelection,
  options: { language?: string; draftLabel?: string } = {},
): string {
  const path = normalizeSelectionFilePath(filePath);
  const draft = options.draftLabel ? ` (${options.draftLabel})` : "";
  return selection.ranges
    .map((range) => fencedExcerpt(
      `${path}:${selectionLineRange(range.startLine, range.endLine)}${draft}`,
      range.text,
      options.language ?? "markdown",
    ))
    .join("\n\n");
}

/** Composer label: file name plus the line span of every selected range. */
export function selectionLocationLabel(filePath: string, content: string): string {
  const name = normalizeSelectionFilePath(filePath).split("/").filter(Boolean).pop() || filePath;
  const prefix = `${normalizeSelectionFilePath(filePath)}:`;
  const ranges: string[] = [];
  for (const line of content.split("\n")) {
    if (!line.startsWith(prefix)) continue;
    const range = line.slice(prefix.length).match(/^\d+(?:-\d+)?/)?.[0];
    if (range) ranges.push(range);
  }
  return ranges.length ? `${name}:${ranges.join(",")}` : name;
}

let reporterSequence = 0;

/**
 * Reports selections of one document surface to the window hub. Results of
 * slower selection builds are dropped once a newer selection was reported.
 */
export function useDocumentSelectionReporter(prefix: string) {
  const hub = inject(DOCUMENT_SELECTION_HUB_KEY, null);
  const ownerId = `${prefix}:${++reporterSequence}`;
  let revision = 0;
  let disposed = false;

  function clear(): void {
    revision += 1;
    hub?.clear(ownerId);
  }

  async function update(
    build: (() => DocumentSelectionInput | null | Promise<DocumentSelectionInput | null>) | null,
  ): Promise<void> {
    if (!hub || disposed) return;
    const current = ++revision;
    if (!build) {
      hub.clear(ownerId);
      return;
    }
    let input: DocumentSelectionInput | null = null;
    try {
      input = await build();
    } catch {
      input = null;
    }
    if (disposed || current !== revision) return;
    if (input?.content.trim()) hub.report({ ...input, id: `${ownerId}:${current}`, ownerId });
    else hub.clear(ownerId);
  }

  onBeforeUnmount(() => {
    clear();
    disposed = true;
  });

  return { enabled: !!hub, update, clear };
}
