import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { UnitySerializedPropertyTarget } from "./unitySerializedProperty";

export const UNITY_VALUE_EDITOR_WINDOW_LABEL = "locus-value-editor";
export const UNITY_VALUE_EDITOR_WINDOW_PATH = "/locus-value-editor";
export const UNITY_VALUE_EDITOR_WINDOW_FLAG = "locusValueEditor";
export const UNITY_VALUE_EDITOR_WINDOW_TITLE = "Locus Value Editor";
export const UNITY_VALUE_EDITOR_PAYLOAD_EVENT = "locus-value-editor:payload";
export const UNITY_VALUE_EDITOR_READY_EVENT = "locus-value-editor:ready";
/**
 * Broadcast by the editor window after a successful commit so source
 * surfaces (fence rows, inspector trees, fields) can re-read or update —
 * the editor owns its write-back and outlives the field that opened it.
 */
export const UNITY_VALUE_EDITOR_COMMITTED_EVENT = "locus-value-editor:committed";

export type UnityValueEditorKind = "curve" | "gradient";

export interface UnityValueEditorPayload {
  kind: UnityValueEditorKind;
  /** Full serialized-property target including propertyPath. */
  target: UnitySerializedPropertyTarget;
  label?: string;
}

export interface UnityValueEditorCommittedEvent {
  kind: UnityValueEditorKind;
  target: UnitySerializedPropertyTarget;
  propertyPath: string;
  value: unknown;
}

const EDITOR_WINDOW_READY_TIMEOUT_MS = 5000;

let pendingEditorWindowCreation: Promise<void> | null = null;
let editorWindowReady: Promise<void> | null = null;

export function isUnityValueEditorWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === UNITY_VALUE_EDITOR_WINDOW_PATH
    || locationLike.search.includes(`${UNITY_VALUE_EDITOR_WINDOW_FLAG}=1`);
}

export function getUnityValueEditorWindowPayload(
  search = window.location.search,
): UnityValueEditorPayload | null {
  const params = new URLSearchParams(search);
  const kind = params.get("kind");
  if (kind !== "curve" && kind !== "gradient") return null;
  let target: UnitySerializedPropertyTarget | null = null;
  try {
    target = JSON.parse(params.get("target") ?? "null") as UnitySerializedPropertyTarget | null;
  } catch {
    target = null;
  }
  if (!target || typeof target !== "object" || !target.kind) return null;
  return {
    kind,
    target,
    label: params.get("label") ?? undefined,
  };
}

export function buildUnityValueEditorWindowUrl(payload: UnityValueEditorPayload): string {
  const params = new URLSearchParams({
    [UNITY_VALUE_EDITOR_WINDOW_FLAG]: "1",
    kind: payload.kind,
    target: JSON.stringify(payload.target),
  });
  if (payload.label) params.set("label", payload.label);
  return `${UNITY_VALUE_EDITOR_WINDOW_PATH}?${params.toString()}`;
}

function hasValidEditorPayload(payload: UnityValueEditorPayload): boolean {
  return !!payload.target
    && typeof payload.target.kind === "string"
    && !!payload.target.kind.trim()
    && !!(payload.target.propertyPath ?? "").trim();
}

function waitForEditorWindowReady(): Promise<void> {
  return new Promise<void>((resolve) => {
    let settled = false;
    let unlisten: (() => void) | null = null;
    const settle = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      unlisten?.();
      unlisten = null;
      resolve();
    };
    const timer = setTimeout(settle, EDITOR_WINDOW_READY_TIMEOUT_MS);
    void listen(UNITY_VALUE_EDITOR_READY_EVENT, settle)
      .then((dispose) => {
        if (settled) {
          dispose();
        } else {
          unlisten = dispose;
        }
      })
      .catch(settle);
  });
}

function createUnityValueEditorWindow(payload: UnityValueEditorPayload): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    const editorWindow = new WebviewWindow(UNITY_VALUE_EDITOR_WINDOW_LABEL, {
      url: buildUnityValueEditorWindowUrl(payload),
      title: UNITY_VALUE_EDITOR_WINDOW_TITLE,
      width: 760,
      height: 560,
      minWidth: 540,
      minHeight: 400,
      decorations: false,
      resizable: true,
      closable: true,
      minimizable: false,
      maximizable: false,
      parent: getCurrentWebviewWindow(),
      center: true,
      shadow: true,
    });

    editorWindow.once("tauri://created", () => {
      resolve();
    });
    editorWindow.once("tauri://error", (event) => {
      reject(event);
    });
  });
}

export async function openUnityValueEditorWindow(
  payload: UnityValueEditorPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;
  if (!hasValidEditorPayload(payload)) return false;

  if (pendingEditorWindowCreation) {
    try {
      await pendingEditorWindowCreation;
    } catch {
      // Creation failed; retry from scratch below.
    }
  }

  const existingWindow = await WebviewWindow.getByLabel(UNITY_VALUE_EDITOR_WINDOW_LABEL);
  if (existingWindow) {
    if (editorWindowReady) await editorWindowReady;
    await existingWindow.emit(UNITY_VALUE_EDITOR_PAYLOAD_EVENT, payload);
    await existingWindow.setFocus();
    return true;
  }

  editorWindowReady = waitForEditorWindowReady();
  pendingEditorWindowCreation = createUnityValueEditorWindow(payload);
  try {
    await pendingEditorWindowCreation;
  } catch (error) {
    editorWindowReady = null;
    throw error;
  } finally {
    pendingEditorWindowCreation = null;
  }

  return true;
}

export function listenUnityValueEditorCommitted(
  handler: (event: UnityValueEditorCommittedEvent) => void,
): Promise<() => void> {
  if (!hasTauriWindowRuntime()) return Promise.resolve(() => {});
  return listen<UnityValueEditorCommittedEvent>(UNITY_VALUE_EDITOR_COMMITTED_EVENT, (event) => {
    if (event.payload && typeof event.payload === "object") handler(event.payload);
  });
}
