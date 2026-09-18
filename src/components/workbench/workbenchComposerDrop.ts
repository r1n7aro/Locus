import type { AssetRefAttachment } from "../../types";
import type { LocusFileDropRef } from "../../services/unity";
import type {
  ProjectExplorerMountEntry,
  ProjectExplorerNode,
  ProjectKnowledgeDocument,
  WorkbenchEditorInput,
} from "../../types/workbench";

const UNITY_REFERENCE_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;
const KNOWLEDGE_REFERENCE_RE = /(?:^|\/)Locus\/knowledge\/(design|plan|memory|skill|reference)\/(.+\.md)$/i;
const BARE_KNOWLEDGE_REFERENCE_RE = /^(design|plan|memory|skill|reference)\/(.+\.md)$/i;

export interface WorkbenchComposerFileInput {
  absolutePath: string;
  workspaceRoot?: string | null;
  relativePath?: string | null;
  name?: string | null;
  typeLabel?: string | null;
  source?: string | null;
  knowledgeSource?: boolean;
  isDir?: boolean;
}

export type WorkbenchComposerFileAttachment =
  | { assetRef: AssetRefAttachment; localFile?: never }
  | { assetRef?: never; localFile: LocusFileDropRef };

function normalizePath(path: string | null | undefined): string {
  return path?.trim().replace(/\\/g, "/").replace(/\/+$/, "") ?? "";
}

function relativePathWithinRoot(absolutePath: string, workspaceRoot: string): string | null {
  const absolute = normalizePath(absolutePath);
  const root = normalizePath(workspaceRoot);
  if (!absolute || !root) return null;
  const prefix = `${root}/`;
  if (!absolute.toLocaleLowerCase().startsWith(prefix.toLocaleLowerCase())) return null;
  return absolute.slice(prefix.length);
}

function knowledgeReferencePath(
  candidates: readonly string[],
  allowBareReference: boolean,
): string | null {
  for (const candidate of candidates) {
    const locusMatch = candidate.match(KNOWLEDGE_REFERENCE_RE);
    if (locusMatch) return `${locusMatch[1]!.toLocaleLowerCase()}/${locusMatch[2]}`;
    if (!allowBareReference) continue;
    const bareMatch = candidate.match(BARE_KNOWLEDGE_REFERENCE_RE);
    if (bareMatch) return `${bareMatch[1]!.toLocaleLowerCase()}/${bareMatch[2]}`;
  }
  return null;
}

export function workbenchComposerFileAttachment(
  input: WorkbenchComposerFileInput,
): WorkbenchComposerFileAttachment | null {
  const absolutePath = normalizePath(input.absolutePath);
  if (!absolutePath) return null;
  const relativePath = normalizePath(input.relativePath);
  const workspaceRelativePath = relativePathWithinRoot(
    absolutePath,
    input.workspaceRoot ?? "",
  );
  const candidates = [relativePath, workspaceRelativePath, absolutePath]
    .filter((candidate): candidate is string => !!candidate);
  const name = input.name?.trim() || undefined;
  const typeLabel = input.typeLabel?.trim() || undefined;

  // Keep directory identity so the composer uses folder previews and list instructions.
  if (input.isDir) {
    return {
      localFile: {
        path: absolutePath,
        isDir: true,
        name,
        typeLabel,
        source: input.source?.trim() || "local",
      },
    };
  }

  const knowledgePath = knowledgeReferencePath(candidates, input.knowledgeSource === true);
  if (knowledgePath) {
    return {
      assetRef: {
        path: knowledgePath,
        kind: "knowledge",
        name,
        typeLabel,
        source: "manual",
      },
    };
  }

  const assetPath = candidates.find((candidate) => UNITY_REFERENCE_ROOT_RE.test(candidate));
  if (assetPath) {
    return {
      assetRef: {
        path: assetPath,
        kind: "asset",
        name,
        typeLabel,
        source: "manual",
      },
    };
  }

  return {
    localFile: {
      path: absolutePath,
      isDir: false,
      name,
      typeLabel,
      source: input.source?.trim() || "local",
    },
  };
}

export function workbenchComposerTreeFileAttachment(
  input: {
    kind: string;
    explorerNode?: Pick<ProjectExplorerNode, "sourcePath" | "sourceKind">;
    mountEntry?: Pick<ProjectExplorerMountEntry, "absolutePath" | "relativePath" | "name" | "isDir">;
    name?: string;
  },
  workspaceRoot: string,
): WorkbenchComposerFileAttachment | null {
  const mountEntry = input.kind === "mountedFile" || input.kind === "mountedFolder"
    ? input.mountEntry
    : undefined;
  const absolutePath = mountEntry?.absolutePath
    ?? (input.kind === "localFile" || input.kind === "folder" ? input.explorerNode?.sourcePath : null);
  if (!absolutePath) return null;
  return workbenchComposerFileAttachment({
    absolutePath,
    workspaceRoot,
    relativePath: mountEntry?.relativePath,
    name: input.name ?? mountEntry?.name,
    source: input.explorerNode?.sourceKind,
    knowledgeSource: input.explorerNode?.sourceKind === "knowledge",
    isDir: mountEntry?.isDir ?? input.kind === "folder",
  });
}

export function workbenchComposerEditorAttachment(
  editor: WorkbenchEditorInput,
  context: {
    workspaceRoot: string;
    targetWorkspaceRoot: string;
    knowledgeDocument?: Pick<ProjectKnowledgeDocument, "type" | "path" | "sourceRoot"> | null;
  },
): WorkbenchComposerFileAttachment | null {
  if (editor.availability !== "available") return null;
  const resource = editor.resource;
  const workspaceRoot = normalizePath(context.workspaceRoot);
  const targetWorkspaceRoot = normalizePath(context.targetWorkspaceRoot);
  const sameWorkspace = !!workspaceRoot
    && workspaceRoot.toLocaleLowerCase() === targetWorkspaceRoot.toLocaleLowerCase();
  const name = editor.title;
  let path: string;
  let isDir = false;

  switch (resource.kind) {
    case "knowledge": {
      const document = context.knowledgeDocument;
      if (!document) return null;
      const documentPath = normalizePath(document.path).replace(/^\/+/, "");
      if (!documentPath) return null;
      path = `${document.type}/${documentPath}`;
      if (sameWorkspace) {
        return { assetRef: { kind: "knowledge", path, name, source: "manual" } };
      }
      const root = normalizePath(document.sourceRoot) || workspaceRoot;
      if (!root) return null;
      path = `${root}/Locus/knowledge/${path}`;
      break;
    }
    case "asset":
      if (sameWorkspace) {
        return { assetRef: { kind: "asset", path: resource.path, name, source: "manual" } };
      }
      path = resource.path;
      break;
    case "sceneObject":
      // Scene objects need their owning Unity workspace; a file cannot identify them.
      return sameWorkspace ? {
        assetRef: {
          kind: "sceneObject",
          path: `${resource.scenePath}/${resource.objectPath}`,
          name,
          source: "manual",
        },
      } : null;
    case "workspaceFile":
      path = resource.path;
      break;
    case "folder":
    case "localDirectory":
    case "localFile":
      path = editor.sourcePath ?? "";
      isDir = resource.kind !== "localFile";
      break;
    default:
      return null;
  }

  path = normalizePath(path);
  if (!path) return null;
  if (!/^(?:[A-Za-z]:\/|\/)/.test(path)) {
    if (!workspaceRoot) return null;
    path = `${workspaceRoot}/${path.replace(/^\.\//, "")}`;
  }
  // Preserve the source file when the receiving conversation uses another checkout.
  if (!sameWorkspace) {
    return { localFile: { path, isDir, name, source: "local" } };
  }
  return workbenchComposerFileAttachment({
    absolutePath: path,
    workspaceRoot,
    name,
    isDir,
  });
}
