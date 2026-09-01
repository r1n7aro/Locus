import { File, FileText, Folder, Package } from "lucide";
import type { KnowledgeDocumentType } from "../../types";
import type { WorkspaceRef } from "../../services/project";
import type {
  InternalDragController,
  InternalDragSource,
} from "../../composables/useInternalDrag";
import {
  externalizeLocusFileDrag,
  externalizeUnityReferenceDrag,
} from "../../composables/useUnityReferenceDragSource";
import type { LocusFileDropRef } from "../../services/unity";
import type { AssetRefAttachment } from "../../types";

export const WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE = "locus/workbench-reference";
export const WORKBENCH_REFERENCE_DRAG_SELECTOR = [
  ".unity-object-identity[data-unity-ref-kind][data-unity-ref-path]",
  ".cm-live-reference[data-reference-kind]",
  ".md-knowledge-ref[data-knowledge-path]",
  ".md-unity-scene-object-ref",
  ".md-unity-asset-ref",
  ".md-file-ref[data-asset-path]",
  ".md-asset-chip",
  ".md-workspace-ref[data-workspace-path]",
  ".md-file-ref[data-file-path]",
  ".asset-chip[data-ref-kind]",
].join(", ");

export interface WorkbenchReferenceDragOrigin {
  projectId: string;
  workspaceRef: WorkspaceRef;
  workspaceRoot: string;
}

export type WorkbenchReferenceDragEntry =
  | {
      kind: "file";
      path: string;
      isDir: boolean;
      name?: string;
      typeLabel?: string;
    }
  | {
      kind: "asset";
      path: string;
      name?: string;
      typeLabel?: string;
    }
  | {
      kind: "sceneObject";
      scenePath: string;
      objectPath: string;
      name?: string;
      typeLabel?: string;
    }
  | {
      kind: "knowledge";
      type: KnowledgeDocumentType;
      path: string;
      documentId?: string;
      name?: string;
    };

export interface WorkbenchReferenceDragData {
  version: 1;
  origin: WorkbenchReferenceDragOrigin;
  entries: WorkbenchReferenceDragEntry[];
}

const claimedPointerEvents = new WeakSet<PointerEvent>();
const UNITY_ASSET_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;
const KNOWLEDGE_REF_RE = /^(?:Locus\/knowledge\/)?(design|plan|memory|skill|reference)\/(.+\.md)$/i;

function normalizePath(value: string | null | undefined): string {
  return (value ?? "").trim().replace(/\\/g, "/").replace(/\/+$/g, "");
}

function basename(path: string): string {
  return normalizePath(path).split("/").filter(Boolean).pop() ?? path;
}

function knowledgeType(value: string | null | undefined): KnowledgeDocumentType | null {
  const normalized = (value ?? "").trim().toLocaleLowerCase();
  if (
    normalized === "design"
    || normalized === "plan"
    || normalized === "memory"
    || normalized === "skill"
    || normalized === "reference"
  ) return normalized;
  return null;
}

function knowledgeEntry(
  pathValue: string | null | undefined,
  typeValue?: string | null,
  name?: string,
): Extract<WorkbenchReferenceDragEntry, { kind: "knowledge" }> | null {
  const normalized = normalizePath(pathValue).replace(/^\/+/, "");
  const match = normalized.match(KNOWLEDGE_REF_RE);
  const type = knowledgeType(typeValue) ?? knowledgeType(match?.[1]);
  const relativePath = match?.[2] ? normalizePath(match[2]) : "";
  if (!type || !relativePath) return null;
  const path = `${type}/${relativePath}`;
  return { kind: "knowledge", type, path, name: name || basename(path) };
}

function sceneObjectEntry(
  scenePathValue: string | null | undefined,
  objectPathValue: string | null | undefined,
  name?: string,
): Extract<WorkbenchReferenceDragEntry, { kind: "sceneObject" }> | null {
  const scenePath = normalizePath(scenePathValue);
  const objectPath = normalizePath(objectPathValue).replace(/^\/+/, "");
  if (!scenePath || !objectPath || !/\.unity$/i.test(scenePath)) return null;
  return {
    kind: "sceneObject",
    scenePath,
    objectPath,
    name: name || basename(objectPath),
  };
}

function sceneObjectEntryFromPath(
  pathValue: string | null | undefined,
  name?: string,
): Extract<WorkbenchReferenceDragEntry, { kind: "sceneObject" }> | null {
  const path = normalizePath(pathValue);
  const match = path.match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
  return sceneObjectEntry(match?.[1], match?.[2], name);
}

function entryName(element: HTMLElement, path: string): string {
  return element.querySelector<HTMLElement>(
    ".asset-chip-name, .md-ref-label, .cm-live-reference-label, .unity-object-identity-title",
  )?.textContent?.trim()
    || basename(path);
}

export function workbenchReferenceDragElementFromElement(target: Element): HTMLElement | null {
  return target.closest<HTMLElement>(WORKBENCH_REFERENCE_DRAG_SELECTOR);
}

/**
 * Resolve every chat reference surface to one canonical semantic entry. The
 * selector order mirrors ChatView activation/context-menu precedence.
 */
export function workbenchReferenceFromElement(
  target: Element,
): WorkbenchReferenceDragEntry | null {
  const livePreviewReference = target.closest(".cm-live-reference[data-reference-kind]");
  const referenceSurface = workbenchReferenceDragElementFromElement(target);
  const interactiveControl = target.closest("button, input, textarea, select");
  if (
    target.closest(".asset-chip-remove")
    || (interactiveControl && interactiveControl !== referenceSurface)
    || (!livePreviewReference && target.closest("[contenteditable='true']"))
  ) {
    return null;
  }

  const unityIdentity = target.closest<HTMLElement>(
    ".unity-object-identity[data-unity-ref-kind][data-unity-ref-path]",
  );
  if (unityIdentity) {
    const kind = unityIdentity.dataset.unityRefKind;
    const path = normalizePath(unityIdentity.dataset.unityRefPath);
    if (kind === "sceneObject") return sceneObjectEntryFromPath(path);
    if ((kind === "asset" || kind === "subObject") && path) {
      return { kind: "asset", path, name: entryName(unityIdentity, path) };
    }
  }

  const knowledgeRef = target.closest<HTMLElement>(
    ".md-knowledge-ref[data-knowledge-path], .asset-chip[data-ref-kind='knowledge']",
  );
  if (knowledgeRef) {
    const path = normalizePath(knowledgeRef.dataset.knowledgePath);
    return knowledgeEntry(path, knowledgeRef.dataset.knowledgeType, entryName(knowledgeRef, path));
  }

  const sceneRef = target.closest<HTMLElement>(
    ".md-unity-scene-object-ref, .asset-chip[data-ref-kind='sceneObject']",
  );
  if (sceneRef) {
    return sceneObjectEntry(
      sceneRef.dataset.scenePath,
      sceneRef.dataset.sceneObjectPath,
      entryName(sceneRef, sceneRef.dataset.sceneObjectPath ?? ""),
    );
  }

  const assetRef = target.closest<HTMLElement>(
    ".md-unity-asset-ref, .md-file-ref[data-asset-path], .md-asset-chip, .asset-chip[data-ref-kind='asset']",
  );
  if (assetRef) {
    const path = normalizePath(assetRef.dataset.assetPath || assetRef.dataset.filePath);
    if (path) {
      const sceneObject = sceneObjectEntryFromPath(path, entryName(assetRef, path));
      if (sceneObject) return sceneObject;
      if (UNITY_ASSET_ROOT_RE.test(path)) {
        return {
          kind: "asset",
          path,
          name: entryName(assetRef, path),
          typeLabel: assetRef.dataset.assetKind,
        };
      }
    }
  }

  const workspaceRef = target.closest<HTMLElement>(
    ".md-workspace-ref[data-workspace-path]",
  );
  if (workspaceRef) {
    const path = normalizePath(workspaceRef.dataset.workspacePath);
    const knowledge = knowledgeEntry(path, workspaceRef.dataset.knowledgeType, entryName(workspaceRef, path));
    if (knowledge) return knowledge;
    if (UNITY_ASSET_ROOT_RE.test(path)) {
      return { kind: "asset", path, name: entryName(workspaceRef, path) };
    }
    if (path) {
      return {
        kind: "file",
        path,
        isDir: workspaceRef.dataset.entryKind === "folder",
        name: entryName(workspaceRef, path),
      };
    }
  }

  const fileRef = target.closest<HTMLElement>(".md-file-ref[data-file-path]");
  if (!fileRef) return null;
  const path = normalizePath(fileRef.dataset.filePath);
  const knowledge = knowledgeEntry(path, fileRef.dataset.knowledgeType, entryName(fileRef, path));
  if (knowledge) return knowledge;
  const sceneObject = sceneObjectEntryFromPath(path, entryName(fileRef, path));
  if (sceneObject) return sceneObject;
  if (UNITY_ASSET_ROOT_RE.test(path)) {
    return { kind: "asset", path, name: entryName(fileRef, path) };
  }
  return path ? {
    kind: "file",
    path,
    isDir: fileRef.dataset.entryKind === "folder",
    name: entryName(fileRef, path),
  } : null;
}

export function claimWorkbenchReferencePointerEvent(event: PointerEvent): void {
  claimedPointerEvents.add(event);
}

export function isWorkbenchReferencePointerEventClaimed(event: PointerEvent): boolean {
  return claimedPointerEvents.has(event);
}

function pointerEventElement(
  event: PointerEvent,
  target?: Element,
): Element | null {
  if (target) return target;
  const eventTarget = event.target as Partial<Element> | null;
  return eventTarget && typeof eventTarget.closest === "function"
    ? eventTarget as Element
    : null;
}

export function startWorkbenchReferenceInternalDrag(
  controller: Pick<InternalDragController, "start">,
  event: PointerEvent,
  origin: WorkbenchReferenceDragOrigin,
  target?: Element,
): boolean {
  if (event.button !== 0 || event.isPrimary === false) return false;
  if (isWorkbenchReferencePointerEventClaimed(event)) return false;
  const element = pointerEventElement(event, target);
  if (!element) return false;
  const entry = workbenchReferenceFromElement(element);
  const captureElement = workbenchReferenceDragElementFromElement(element);
  if (!entry || !captureElement) return false;

  const data: WorkbenchReferenceDragData = {
    version: 1,
    origin,
    entries: [entry],
  };
  if (!controller.start(event, workbenchReferenceInternalDragSource(data, captureElement))) {
    return false;
  }
  claimWorkbenchReferencePointerEvent(event);
  return true;
}

function entryLabel(entry: WorkbenchReferenceDragEntry): string {
  if (entry.name?.trim()) return entry.name.trim();
  if (entry.kind === "sceneObject") return basename(entry.objectPath);
  return basename(entry.path);
}

function fileRefsForExternalization(data: WorkbenchReferenceDragData): LocusFileDropRef[] {
  return data.entries.flatMap((entry): LocusFileDropRef[] => {
    if (entry.kind === "asset" || entry.kind === "sceneObject") return [];
    const path = entry.kind === "knowledge"
      ? `Locus/knowledge/${entry.path}`
      : entry.path;
    return [{
      path,
      isDir: entry.kind === "file" && entry.isDir,
      name: entryLabel(entry),
      source: entry.kind === "knowledge" ? "knowledge" : "locus",
    }];
  });
}

function assetRefsForExternalization(data: WorkbenchReferenceDragData): AssetRefAttachment[] {
  return data.entries.flatMap((entry): AssetRefAttachment[] => {
    if (entry.kind === "asset") {
      return [{
        kind: "asset",
        path: entry.path,
        name: entry.name,
        typeLabel: entry.typeLabel,
        source: "manual",
      }];
    }
    if (entry.kind === "sceneObject") {
      return [{
        kind: "sceneObject",
        path: `${entry.scenePath}/${entry.objectPath}`,
        name: entry.name,
        typeLabel: entry.typeLabel,
        source: "manual",
      }];
    }
    return [];
  });
}

async function externalizeWorkbenchReferenceDrag(data: WorkbenchReferenceDragData): Promise<void> {
  const assets = assetRefsForExternalization(data);
  if (assets.length > 0) {
    await externalizeUnityReferenceDrag(assets, data.origin.workspaceRef);
    return;
  }
  const files = fileRefsForExternalization(data);
  if (files.length > 0) {
    await externalizeLocusFileDrag(files, data.origin.workspaceRef);
  }
}

export function workbenchReferenceInternalDragSource(
  data: WorkbenchReferenceDragData,
  captureElement?: HTMLElement,
): InternalDragSource<WorkbenchReferenceDragData> {
  const first = data.entries[0];
  const kind = first?.kind === "knowledge"
    ? "file"
    : first?.kind === "file" && first.isDir
      ? "folder"
      : "file";
  return {
    id: `workbench-reference:${data.origin.projectId}:${data.entries.map(entryLabel).join("|")}`,
    captureElement,
    payload: { type: WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE, data },
    preview: {
      label: first ? entryLabel(first) : "",
      count: data.entries.length,
      kind,
      icon: first?.kind === "knowledge"
        ? FileText
        : first?.kind === "asset" || first?.kind === "sceneObject"
          ? Package
          : first?.kind === "file" && first.isDir
            ? Folder
          : File,
    },
    allowedOperations: ["copy"],
    externalize: () => externalizeWorkbenchReferenceDrag(data),
  };
}
