import type { GitFileChange } from "../../types";

type StagingTreeFile = Pick<GitFileChange, "path">;

type StagingTreeChild<TFile extends StagingTreeFile = GitFileChange> =
  | {
      kind: "folder";
      node: StagingTreeNode<TFile>;
    }
  | {
      kind: "file";
      file: TFile;
    };

interface StagingTreeNode<TFile extends StagingTreeFile = GitFileChange> {
  path: string;
  name: string;
  children: StagingTreeChild<TFile>[];
  folderMap: Map<string, StagingTreeNode<TFile>>;
}

export type StagingTreeRow<TFile extends StagingTreeFile = GitFileChange> =
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
      file: TFile;
    };

function createNode<TFile extends StagingTreeFile>(path: string, name: string): StagingTreeNode<TFile> {
  return {
    path,
    name,
    children: [],
    folderMap: new Map<string, StagingTreeNode<TFile>>(),
  };
}

function splitPath(path: string): string[] {
  return path.split("/").filter(Boolean);
}

function buildTree<TFile extends StagingTreeFile>(files: readonly TFile[]): StagingTreeNode<TFile> {
  const root = createNode<TFile>("", "");

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
        childNode = createNode<TFile>(currentPath, segment);
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

function getFolderChildren<TFile extends StagingTreeFile>(node: StagingTreeNode<TFile>): StagingTreeNode<TFile>[] {
  return node.children
    .filter((child): child is Extract<StagingTreeChild<TFile>, { kind: "folder" }> => child.kind === "folder")
    .map((child) => child.node);
}

function hasFileChildren<TFile extends StagingTreeFile>(node: StagingTreeNode<TFile>): boolean {
  return node.children.some((child) => child.kind === "file");
}

function compareTreeChildKind<TFile extends StagingTreeFile>(
  left: StagingTreeChild<TFile>,
  right: StagingTreeChild<TFile>,
): number {
  if (left.kind === right.kind) return 0;
  return left.kind === "folder" ? -1 : 1;
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
  collapsedPaths?: ReadonlySet<string>,
): StagingTreeRow[];
export function buildStagingTreeRows<TFile extends StagingTreeFile>(
  files: readonly TFile[],
  collapsedPaths: ReadonlySet<string>,
  getFileKey: (file: TFile) => string,
): StagingTreeRow<TFile>[];
export function buildStagingTreeRows<TFile extends StagingTreeFile = GitFileChange>(
  files: readonly TFile[],
  collapsedPaths: ReadonlySet<string> = new Set<string>(),
  getFileKey: (file: TFile) => string = (file) => file.path,
): StagingTreeRow<TFile>[] {
  const root = buildTree(files);
  const rows: StagingTreeRow<TFile>[] = [];

  function walk(children: readonly StagingTreeChild<TFile>[], depth: number) {
    for (const child of [...children].sort(compareTreeChildKind)) {
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
        key: `file:${getFileKey(child.file)}`,
        depth,
        file: child.file,
      });
    }
  }

  walk(root.children, 0);
  return rows;
}
