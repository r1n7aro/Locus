import type { DevelopmentResourceRef } from "../../types/workbench";

export type WorkspaceSecondarySection = "knowledge" | "assets" | "views" | "archived" | "collab" | "agents";

export function workspaceSecondarySection(kind: string): WorkspaceSecondarySection | null {
  switch (kind) {
    case "agentsRoot": return "agents";
    case "knowledgeRoot": return "knowledge";
    case "assetsRoot": return "assets";
    case "viewsRoot": return "views";
    case "archivedRoot": return "archived";
    default: return null;
  }
}

export function workspaceSecondaryResourceSection(resource: DevelopmentResourceRef): WorkspaceSecondarySection | null {
  if (resource.kind === "knowledgeRoot") return "knowledge";
  if (resource.kind !== "section") return null;
  if (resource.section === "agents") return resource.agentId ? null : "agents";
  if (resource.section === "archived" && resource.sessionId) return null;
  if (resource.section === "knowledge" && resource.knowledgePage) return null;
  return ["knowledge", "assets", "views", "archived"].includes(resource.section)
    ? resource.section as WorkspaceSecondarySection
    : null;
}
