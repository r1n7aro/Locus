import type { RoutedWorkspaceEvent, WorkspaceRef } from "./project";
import type { WindowEventInterests } from "./windowEventInterests";
import type { KnowledgeDocumentType } from "../types";

/** A file, folder or knowledge document mentioned in a conversation or
 * linked from a document. */
export type RemoteReference =
  | { kind: "file"; path: string; workspaceRef?: WorkspaceRef | null }
  | { kind: "folder"; path: string; workspaceRef?: WorkspaceRef | null }
  | { kind: "knowledge"; docType: KnowledgeDocumentType; path: string; workspaceRef?: WorkspaceRef | null };

/** Transport of the mobile remote client (the Locus Remote app). Installed by
 * the mobile entry before anything subscribes; absent in every desktop build.
 * Kept dependency-free so the event modules can consult it without cycles. */
/** A workspace path the client shows: a file, or a folder it lists. */
export interface RemoteFileWatch {
  checkoutId: string;
  /** Workspace-relative; "" for the root. */
  path: string;
}

export interface RemoteClientTransport {
  /** Replaces this client's merged workspace event interests on the desktop. */
  setInterests(interests: WindowEventInterests): Promise<void>;
  /** Replaces the paths whose file changes the desktop forwards (it forwards
   * none otherwise). */
  setFileWatches?(watches: RemoteFileWatch[]): Promise<void>;
  /** Routed workspace events the desktop forwards for those interests. */
  subscribeWorkspaceEvents(handler: (event: RoutedWorkspaceEvent<unknown>) => void): () => void;
  /** Shows a referenced file or knowledge document in the client's own
   * viewer (instead of the desktop workbench). */
  openReference?(reference: RemoteReference): void;
}

let transport: RemoteClientTransport | null = null;

export function installRemoteClientTransport(next: RemoteClientTransport): () => void {
  if (transport) throw new Error("A remote client transport is already installed.");
  transport = next;
  return () => {
    if (transport === next) transport = null;
  };
}

export function remoteClientTransport(): RemoteClientTransport | null {
  return transport;
}

/** True inside the Locus Remote app: desktop-only actions (workbench, Unity,
 * native windows) must not run. */
export function isRemoteClient(): boolean {
  return transport !== null;
}

/** Whether the remote client can show references itself. */
export function canOpenRemoteReferences(): boolean {
  return typeof transport?.openReference === "function";
}

/** Opens `reference` in the remote client's viewer; false when unsupported. */
export function openRemoteReference(reference: RemoteReference): boolean {
  if (!transport?.openReference) return false;
  transport.openReference(reference);
  return true;
}
