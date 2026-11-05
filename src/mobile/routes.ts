import type { WorkspaceRef } from "../services/project";
import type { KnowledgeDocumentType } from "../types";

/** What the document viewer shows. */
export type DocumentSource =
  | { kind: "file"; workspaceRef: WorkspaceRef; path: string }
  | { kind: "knowledge"; workspaceRef: WorkspaceRef; docType: KnowledgeDocumentType; path: string };

/** One screen on the navigation stack above the project list. */
export type MobileRoute =
  | { kind: "session"; key: string; sessionId: string | null; workspaceRef: WorkspaceRef | null; title: string }
  | { kind: "files"; key: string; workspaceRef: WorkspaceRef; projectName: string; path: string }
  | {
    kind: "knowledge";
    key: string;
    workspaceRef: WorkspaceRef;
    projectName: string;
    docType: KnowledgeDocumentType | null;
    path: string;
  }
  | { kind: "document"; key: string; source: DocumentSource; title: string }
  | { kind: "image"; key: string; src: string; title: string };

/** Last path segment, for titles. */
export function baseName(path: string): string {
  const parts = path.replace(/\\/g, "/").replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}
