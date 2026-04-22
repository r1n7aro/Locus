import type { ExplorerNode } from "../../composables/useKnowledgeState";
import type { KnowledgeFolderDisplayStats } from "../../types";

export function buildFolderDisplayStats(
  nodes: ExplorerNode[],
  cachedStats?: Record<string, KnowledgeFolderDisplayStats>,
): Map<string, KnowledgeFolderDisplayStats> {
  const stats = new Map<string, KnowledgeFolderDisplayStats>();

  const visit = (node: ExplorerNode): number => {
    if (node.kind === "document") return 1;

    let descendantDocumentCount = 0;
    for (const child of node.children) {
      descendantDocumentCount += visit(child);
    }

    stats.set(node.path, {
      directChildCount: node.children.length,
      descendantDocumentCount,
    });
    return descendantDocumentCount;
  };

  for (const node of nodes) {
    visit(node);
  }

  if (cachedStats) {
    for (const [path, value] of Object.entries(cachedStats)) {
      stats.set(path, value);
    }
  }

  return stats;
}
