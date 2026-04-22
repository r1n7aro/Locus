import type { GitFileChange } from "../../types";

type StagingTreeChild =
  | {
      kind: "folder";
      node: StagingTreeNode;
    }
  | {
      kind: "file";
      file: GitFileChange;
    };

interface StagingTreeNode {
  path: string;
  name: string;
  children: StagingTreeChild[];
  folderMap: Map<string, StagingTreeNode>;
}

export type StagingTreeRow =
  | {
      kind: "folder";
      key: string;
      path: string;
      name: string;
      chainPaths: string[];
      depth: number;
      expanded: boolean;
    }
  | {
      kind: "file";
      key: string;
      depth: number;
      file: GitFileChange;
    };

function createNode(path: string, name: string): StagingTreeNode {
  return {
    path,
    name,
    children: [],
    folderMap: new Map<string, StagingTreeNode>(),
  };
}

function splitPath(path: string): string[] {
  return path.split("/").filter(Boolean);
}

function buildTree(files: readonly GitFileChange[]): StagingTreeNode {
  const root = createNode("", "");

  for (const file of files) {
    const segments = splitPath(file.path);
    const leafName = segments.pop();
    if (!leafName) continue;

    let current = root;
    let currentPath = "";

    for (const segment of segments) {
      currentPath = currentPath ? `${currentPath}/${segment}` : segment;
      let childNode = current.folderMap.get(segment);
      if (!childNode) {
        childNode = createNode(currentPath, segment);
        current.folderMap.set(segment, childNode);
        current.children.push({
          kind: "folder",
          node: childNode,
        });
      }
      current = childNode;
    }

    current.children.push({
      kind: "file",
      file,
    });
  }

  return root;
}

function getFolderChildren(node: StagingTreeNode): StagingTreeNode[] {
  return node.children
    .filter((child): child is Extract<StagingTreeChild, { kind: "folder" }> => child.kind === "folder")
    .map((child) => child.node);
}

function hasFileChildren(node: StagingTreeNode): boolean {
  return node.children.some((child) => child.kind === "file");
}

export function collectStagingFolderPaths(
  files: readonly Pick<GitFileChange, "path">[],
): Set<string> {
  const paths = new Set<string>();

  for (const file of files) {
    const segments = splitPath(file.path);
    segments.pop();
    let currentPath = "";
    for (const segment of segments) {
      currentPath = currentPath ? `${currentPath}/${segment}` : segment;
      paths.add(currentPath);
    }
  }

  return paths;
}

export function buildStagingFolderFileMap(
  files: readonly Pick<GitFileChange, "path">[],
): Map<string, string[]> {
  const folderFiles = new Map<string, string[]>();

  for (const file of files) {
    const segments = splitPath(file.path);
    segments.pop();
    let currentPath = "";

    for (const segment of segments) {
      currentPath = currentPath ? `${currentPath}/${segment}` : segment;
      const existing = folderFiles.get(currentPath);
      if (existing) {
        existing.push(file.path);
      } else {
        folderFiles.set(currentPath, [file.path]);
      }
    }
  }

  return folderFiles;
}

export function buildStagingTreeRows(
  files: readonly GitFileChange[],
  collapsedPaths: ReadonlySet<string> = new Set<string>(),
): StagingTreeRow[] {
  const root = buildTree(files);
  const rows: StagingTreeRow[] = [];

  function walk(children: readonly StagingTreeChild[], depth: number) {
    for (const child of children) {
      if (child.kind === "folder") {
        let visibleNode = child.node;
        const chainNames = [child.node.name];
        const chainPaths = [child.node.path];

        while (true) {
          const folderChildren = getFolderChildren(visibleNode);
          if (hasFileChildren(visibleNode) || folderChildren.length !== 1) break;
          visibleNode = folderChildren[0];
          chainNames.push(visibleNode.name);
          chainPaths.push(visibleNode.path);
        }

        const expanded = !chainPaths.some((path) => collapsedPaths.has(path));
        rows.push({
          kind: "folder",
          key: `folder:${visibleNode.path}`,
          path: visibleNode.path,
          name: chainNames.join("/"),
          chainPaths,
          depth,
          expanded,
        });
        if (expanded) {
          walk(visibleNode.children, depth + 1);
        }
        continue;
      }

      rows.push({
        kind: "file",
        key: `file:${child.file.path}`,
        depth,
        file: child.file,
      });
    }
  }

  walk(root.children, 0);
  return rows;
}
