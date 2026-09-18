import type { KnowledgeDocumentSummary } from "../../types";
import { isSkillPackageRootDocument } from "../../composables/useKnowledgeState";

export function knowledgeResourceManagedHint(document: KnowledgeDocumentSummary): string | undefined {
  const source = document.externalSource;
  if (source?.locator?.startsWith("plugin://")) return "knowledge.explorer.pluginManagedHint";
  if (source?.locator?.startsWith("external://")) return "knowledge.explorer.externalManagedHint";
  if (source?.provider === "package" && !isSkillPackageRootDocument(document)) return "knowledge.explorer.packageManagedHint";
  return undefined;
}

export function knowledgeResourcePath(document: Pick<KnowledgeDocumentSummary, "type" | "path">): string {
  const path = document.path.replace(/\\/g, "/").replace(/^\/+/, "");
  return path.startsWith(`${document.type}/`) ? path : `${document.type}/${path}`;
}
