import type {
  AssetPreviewPayload,
  AssetRefAttachment,
  SemanticTargetInspector,
} from "../../types";
import type { UnitySerializedPropertySnapshot } from "../unity/unitySerializedValue";
import {
  unityAssetIconKindForPath,
  type UnityAssetIconKind,
} from "../icons/unityAssetIcons";

export type UnityObjectAssetRefAttachment = AssetRefAttachment & {
  kind: "asset" | "sceneObject";
};

export type UnityObjectPreviewLevel = "inline" | "row" | "thumbnail" | "inspector" | "editor";
export type UnityObjectPreviewSourceState = "disk" | "live" | "loading";

export type UnityObjectRefKind = "asset" | "sceneObject" | "subObject" | "importer";

export type UnityEditState =
  | "editable"
  | "readonly"
  | "unsupported"
  | "externalSource"
  | "disconnected";

export interface UnityObjectPreviewCapabilities {
  select: boolean;
  drag: boolean;
  preview: boolean;
  inspect: boolean;
  edit: boolean;
}

export interface UnityObjectPreviewRef {
  kind: UnityObjectRefKind;
  path: string;
  name?: string;
  typeLabel?: string;
  guid?: string;
  fileId?: number;
}

export type UnityObjectPropertyTreeInput =
  | UnitySerializedPropertySnapshot
  | UnitySerializedPropertySnapshot[];

export interface UnityObjectPreviewModel {
  ref: UnityObjectPreviewRef;
  title: string;
  subtitle?: string;
  iconKind: UnityAssetIconKind;
  capabilities: UnityObjectPreviewCapabilities;
  editState: UnityEditState;
  readonlyReason?: string;
  previewPayload?: AssetPreviewPayload;
  inspector?: SemanticTargetInspector | null;
  propertyTree?: UnityObjectPropertyTreeInput | null;
}

export interface UnityObjectPreviewInput {
  ref?: Partial<UnityObjectPreviewRef> | null;
  kind?: UnityObjectRefKind;
  path?: string;
  name?: string;
  title?: string;
  subtitle?: string;
  typeLabel?: string;
  guid?: string;
  fileId?: number;
  iconKind?: UnityAssetIconKind;
  capabilities?: Partial<UnityObjectPreviewCapabilities>;
  editState?: UnityEditState;
  readonlyReason?: string;
  writable?: boolean;
  unityConnected?: boolean;
  previewPayload?: AssetPreviewPayload;
  inspector?: SemanticTargetInspector | null;
  propertyTree?: UnityObjectPropertyTreeInput | null;
}

const MODEL_SOURCE_EXTENSIONS = new Set([
  ".fbx",
  ".obj",
  ".blend",
  ".dae",
  ".3ds",
  ".ma",
  ".mb",
  ".max",
]);

const CODE_SOURCE_EXTENSIONS = new Set([
  ".cs",
  ".shader",
  ".compute",
  ".cginc",
  ".hlsl",
  ".glsl",
]);

const UNITY_EDITABLE_ASSET_EXTENSIONS = new Set([
  ".prefab",
  ".unity",
  ".asset",
  ".mat",
  ".anim",
  ".controller",
  ".overridecontroller",
  ".physicmaterial",
  ".physicsmaterial2d",
  ".flare",
  ".mask",
  ".fontsettings",
]);

export function normalizeUnityObjectPreviewModel(
  input: UnityObjectPreviewInput | UnityObjectPreviewModel | UnityObjectAssetRefAttachment,
): UnityObjectPreviewModel {
  if (isNormalizedUnityObjectPreviewModel(input)) {
    return input;
  }

  const normalizedInput = normalizeInput(input);
  const ref = normalizeRef(normalizedInput);
  const propertyTree = normalizedInput.propertyTree ?? null;
  const title = normalizedInput.title?.trim()
    || ref.name?.trim()
    || basenameForTitle(ref.path)
    || shortGuid(ref.guid)
    || ref.path
    || "Unity Object";
  const subtitle = normalizedInput.subtitle?.trim()
    || normalizedInput.typeLabel?.trim()
    || ref.typeLabel?.trim()
    || defaultSubtitle(ref);
  const iconKind = normalizedInput.iconKind
    ?? unityAssetIconKindForPath(ref.path || title, {
      isSceneObject: ref.kind === "sceneObject" || ref.kind === "subObject",
      isFolder: false,
      fallbackKind: "asset",
    });
  const editState = resolveUnityObjectEditState(normalizedInput, ref, propertyTree);
  const capabilities = resolveUnityObjectCapabilities(normalizedInput, ref, editState, propertyTree);
  const readonlyReason = normalizedInput.readonlyReason?.trim()
    || defaultReadonlyReason(editState, ref, propertyTree);

  return {
    ref,
    title,
    subtitle,
    iconKind,
    capabilities,
    editState,
    readonlyReason,
    previewPayload: normalizedInput.previewPayload,
    inspector: normalizedInput.inspector ?? null,
    propertyTree,
  };
}

export function unityObjectPreviewAssetRef(
  model: UnityObjectPreviewInput | UnityObjectPreviewModel | UnityObjectAssetRefAttachment,
): AssetRefAttachment | null {
  const normalized = normalizeUnityObjectPreviewModel(model);
  if (!normalized.capabilities.drag) return null;

  if (normalized.ref.kind === "sceneObject") {
    return {
      kind: "sceneObject",
      path: normalized.ref.path,
      name: normalized.title,
      typeLabel: normalized.ref.typeLabel,
      source: "manual",
    };
  }

  if (normalized.ref.kind === "asset" || normalized.ref.kind === "subObject") {
    return {
      kind: "asset",
      path: normalized.ref.path,
      name: normalized.title,
      typeLabel: normalized.ref.typeLabel,
      source: "manual",
    };
  }

  return null;
}

export function isUnityObjectEditable(model: UnityObjectPreviewInput | UnityObjectPreviewModel): boolean {
  return normalizeUnityObjectPreviewModel(model).capabilities.edit;
}

export function isUnityExternalSourceAssetPath(path: string): boolean {
  return MODEL_SOURCE_EXTENSIONS.has(extensionOf(path));
}

/**
 * Script/shader source files read better as highlighted source text than as
 * a serialized object panel: a MonoScript's serialized form is only importer
 * metadata (class name, execution order, ...), not the script itself.
 */
export function isUnityCodeSourceAssetPath(path: string): boolean {
  return CODE_SOURCE_EXTENSIONS.has(extensionOf(path));
}

export function hasEditableUnityPropertySnapshot(input: UnityObjectPropertyTreeInput | null | undefined): boolean {
  const snapshots = Array.isArray(input) ? input : input ? [input] : [];
  return snapshots.some(hasEditableSnapshot);
}

function normalizeInput(
  input: UnityObjectPreviewInput | UnityObjectAssetRefAttachment,
): UnityObjectPreviewInput {
  if (isAssetRefAttachment(input)) {
    return {
      kind: input.kind,
      path: input.path,
      name: input.name,
      typeLabel: input.typeLabel,
    };
  }
  return input;
}

function normalizeRef(input: UnityObjectPreviewInput): UnityObjectPreviewRef {
  const ref = input.ref ?? {};
  const kind = normalizeRefKind(ref.kind ?? input.kind);
  const path = normalizePath(ref.path ?? input.path ?? "");
  const name = ref.name ?? input.name;
  const typeLabel = ref.typeLabel ?? input.typeLabel;
  return {
    kind,
    path,
    name,
    typeLabel,
    guid: ref.guid ?? input.guid,
    fileId: ref.fileId ?? input.fileId,
  };
}

function normalizeRefKind(kind: string | undefined): UnityObjectRefKind {
  if (kind === "sceneObject" || kind === "subObject" || kind === "importer") return kind;
  return "asset";
}

function resolveUnityObjectEditState(
  input: UnityObjectPreviewInput,
  ref: UnityObjectPreviewRef,
  propertyTree: UnityObjectPropertyTreeInput | null,
): UnityEditState {
  if (input.editState) return input.editState;
  if (input.unityConnected === false) return "disconnected";
  if (input.capabilities?.edit === false || input.writable === false) return "readonly";
  if (ref.kind !== "importer" && isUnityExternalSourceAssetPath(ref.path)) return "externalSource";
  if (isReadOnlyPackagePath(ref.path) && input.writable !== true) return "readonly";
  if (input.writable === true || input.capabilities?.edit === true) return "editable";
  if (hasEditableUnityPropertySnapshot(propertyTree)) return "editable";
  if (propertyTree) return "readonly";
  if (ref.kind === "asset" && UNITY_EDITABLE_ASSET_EXTENSIONS.has(extensionOf(ref.path))) return "unsupported";
  if (ref.kind === "sceneObject" || ref.kind === "subObject" || ref.kind === "importer") return "unsupported";
  return "unsupported";
}

function resolveUnityObjectCapabilities(
  input: UnityObjectPreviewInput,
  ref: UnityObjectPreviewRef,
  editState: UnityEditState,
  propertyTree: UnityObjectPropertyTreeInput | null,
): UnityObjectPreviewCapabilities {
  const explicit = input.capabilities ?? {};
  const hasPreviewData = !!input.previewPayload || !!input.inspector || !!propertyTree;
  const hasPathReference = isUnityReferencePath(ref.path) || ref.kind === "sceneObject" || ref.kind === "subObject";
  const isUnityReference = hasPathReference || !!ref.guid;
  const canEdit = editState === "editable";

  return {
    select: explicit.select ?? isUnityReference,
    drag: explicit.drag ?? (hasPathReference && ref.kind !== "importer"),
    preview: explicit.preview ?? (hasPreviewData || ref.kind !== "importer"),
    inspect: explicit.inspect ?? (!!input.inspector || input.previewPayload?.kind === "structured" || !!propertyTree),
    edit: canEdit && (explicit.edit ?? canEdit),
  };
}

function defaultReadonlyReason(
  editState: UnityEditState,
  ref: UnityObjectPreviewRef,
  propertyTree: UnityObjectPropertyTreeInput | null,
): string | undefined {
  if (editState === "editable") return undefined;
  if (editState === "disconnected") return "Unity disconnected";
  if (editState === "externalSource") return "External source asset";
  if (editState === "readonly" && isReadOnlyPackagePath(ref.path)) return "Package asset";
  if (editState === "readonly" && propertyTree) return "Read only";
  if (editState === "unsupported" && ref.kind === "importer") return "Importer properties unavailable";
  if (editState === "unsupported") return "No editable properties";
  return "Read only";
}

function defaultSubtitle(ref: UnityObjectPreviewRef): string | undefined {
  if (ref.kind === "sceneObject") return "Scene Object";
  if (ref.kind === "subObject") return "Sub-Object";
  if (ref.kind === "importer") return "Importer";
  const ext = extensionOf(ref.path).replace(/^\./, "");
  if (ext) return ext.toUpperCase();
  if (ref.guid) return "GUID";
  return undefined;
}

function hasEditableSnapshot(snapshot: UnitySerializedPropertySnapshot): boolean {
  if (snapshot.editable !== false && !snapshot.hasChildren && !snapshot.children?.length) {
    return true;
  }
  if (snapshot.editable !== false && (snapshot.isArray || snapshot.isManagedReference)) {
    return true;
  }
  return (snapshot.children ?? []).some(hasEditableSnapshot);
}

function isNormalizedUnityObjectPreviewModel(value: unknown): value is UnityObjectPreviewModel {
  return Boolean(
    value &&
    typeof value === "object" &&
    "ref" in value &&
    "capabilities" in value &&
    "editState" in value &&
    "iconKind" in value,
  );
}

function isAssetRefAttachment(value: unknown): value is UnityObjectAssetRefAttachment {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  if (
    "ref" in record ||
    "guid" in record ||
    "fileId" in record ||
    "title" in record ||
    "subtitle" in record ||
    "iconKind" in record ||
    "capabilities" in record ||
    "editState" in record ||
    "readonlyReason" in record ||
    "writable" in record ||
    "unityConnected" in record ||
    "previewPayload" in record ||
    "inspector" in record ||
    "propertyTree" in record
  ) {
    return false;
  }
  return Boolean(
    typeof record.path === "string" &&
    (record.kind === "asset" || record.kind === "sceneObject"),
  );
}

function isUnityReferencePath(path: string): boolean {
  return /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i.test(normalizePath(path));
}

function isReadOnlyPackagePath(path: string): boolean {
  return /^Packages\//i.test(normalizePath(path));
}

function basenameForTitle(path: string): string {
  return normalizePath(path).split("/").filter(Boolean).pop() ?? "";
}

function extensionOf(path: string): string {
  const leaf = normalizePath(path).split("/").pop() ?? "";
  const dot = leaf.lastIndexOf(".");
  return dot >= 0 ? leaf.slice(dot).toLowerCase() : "";
}

function normalizePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/g, "");
}

function shortGuid(guid: string | undefined): string {
  const normalized = (guid || "").trim();
  return normalized.length > 10 ? normalized.slice(0, 8) : normalized;
}
