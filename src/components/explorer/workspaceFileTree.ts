import type { AssetExplorerNode } from "../../composables/useAssetState";
import { isMetaFile } from "../../composables/useHideMeta";
import type { WorkspaceTreeItem } from "./WorkspaceTree.vue";
import { workspaceFileParent } from "./workspaceFileMove";
import type { WorkspaceFileOrder } from "../../services/workspaceFileOrder";

// The shared 18px text/icon line and 14px SVG stay on whole CSS pixels.
// Special nodes, directories and files use the same uninterrupted hit area.
export const WORKSPACE_FILE_ROW_HEIGHT = 28;

export interface WorkspaceFileEntry extends WorkspaceTreeItem {
  node: AssetExplorerNode;
  loadMore?: boolean;
}

/** The workspace header already identifies the root; project its children at depth zero. */
export function workspaceFileEntries(
  tree: AssetExplorerNode[],
  isExpanded: (path: string) => boolean,
  loadMoreLabel: string,
  isHidden: (path: string) => boolean = () => false,
): WorkspaceFileEntry[] {
  const entries: WorkspaceFileEntry[] = [];
  function walk(nodes: AssetExplorerNode[], depth: number) {
    for (const node of nodes) {
      if (isHidden(node.path)) continue;
      if (node.kind === "file" && (isMetaFile(node.name) || node.name.toLowerCase().endsWith(".csv.view"))) continue;
      const root = node.kind === "folder" && node.isRoot;
      const expanded = root || (node.kind === "folder" && isExpanded(node.path));
      if (!root) entries.push({
        key: node.path, node,
        treeRow: { key: node.path, name: node.name, depth, kind: node.kind,
          monospace: true, fileDropDirectory: node.kind === "folder" ? node.path : workspaceFileParent(node.path),
          expandable: node.kind === "folder", expanded },
      });
      if (node.kind !== "folder" || !expanded) continue;
      const childDepth = root ? depth : depth + 1;
      walk(node.children, childDepth);
      // Refreshes keep loaded rows in place; only initial reads and real paging need a row.
      if ((!node.loaded && node.loading) || node.hasMore) entries.push({
        key: `${node.path}::load-more`, node, loadMore: true,
        treeRow: { key: `${node.path}::load-more`, name: loadMoreLabel, depth: childDepth,
          kind: "file", disabled: node.loading },
      });
    }
  }
  walk(tree, 0);
  return entries;
}

export interface WorkspaceFileTreeState {
  entries: WorkspaceFileEntry[];
  toggle: (path: string) => void;
  /** Expands and pages a path's ancestors; resolves the listed relative path, or null. */
  reveal: (path: string) => Promise<string | null>;
  loadMore: (path: string) => void;
  refresh: (path: string, kind: "delete" | "upsert") => Promise<void>;
  order: WorkspaceFileOrder;
  siblings: (directory: string) => Promise<WorkspaceFileEntry[]>;
  move: (directory: string, item: string, anchor: string | null, side: "before" | "after", base: string[]) => Promise<void>;
}
