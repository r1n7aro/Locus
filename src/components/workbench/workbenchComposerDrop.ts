import type { AssetRefAttachment } from "../../types";
import type { LocusFileDropRef } from "../../services/unity";

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
