import type { KnowledgeDocumentSummary } from "../../types";
import type { WorkspaceRef } from "../../services/project";
import { explorerFileKey, explorerKnowledgeKey } from "../../composables/useExplorerPathDisplay";
import { knowledgeResourcePath } from "../knowledge/knowledgeResourceActions";

export interface ExplorerResourceTarget {
  projectId: string;
  root: string;
  path: string;
  workspaceRef?: WorkspaceRef | null;
  mounted?: boolean;
  document?: KnowledgeDocumentSummary;
}
export type ExplorerResourceAction = "rename" | "delete" | "copy" | "reveal" | "export";
export const resourcePathKey = (target: ExplorerResourceTarget) => target.document
  ? explorerKnowledgeKey(target.root, target.document.id) : explorerFileKey(target.path);
export const resourceName = (target: ExplorerResourceTarget) => (target.document?.path ?? target.path).replace(/\\/g, "/").split("/").pop() ?? "";
export function resourceRelativePath(target: ExplorerResourceTarget): string {
  if (target.document) return knowledgeResourcePath(target.document);
  const path = target.path.replace(/\\/g, "/");
  const root = target.root.replace(/\\/g, "/").replace(/\/$/, "");
  const insensitive = /^(?:[a-z]:\/|\/\/)/i.test(root);
  return (insensitive ? path.toLowerCase().startsWith(`${root.toLowerCase()}/`) : path.startsWith(`${root}/`))
    ? path.slice(root.length + 1) : path;
}
export function validResourceName(name: string): boolean {
  return !!name.trim() && !/[\\/:*?"<>|\u0000-\u001f]/.test(name) && !/[. ]$/.test(name)
    && name !== "." && name !== ".." && !/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(name);
}
