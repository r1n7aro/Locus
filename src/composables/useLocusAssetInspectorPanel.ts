import { reactive, ref } from "vue";
import {
  isValidLocusAssetInspectorPayload,
  normalizeLocusAssetInspectorPayload,
  openLocusAssetInspectorWindow,
  type LocusAssetInspectorWindowPayload,
} from "../services/locusAssetInspectorWindow";

export type LocusAssetInspectorMode = "embedded" | "window" | "auto";

/**
 * Viewport thresholds for the adaptive ("auto") mode. The floating panel
 * opens at 520x640 (see LocusAssetInspectorPanel), so the embedded panel only
 * has room to fully expand when the window comfortably exceeds that
 * footprint; smaller windows route to a standalone inspector tab instead.
 */
const EMBEDDED_PANEL_FIT_MIN_WIDTH = 1040;
const EMBEDDED_PANEL_FIT_MIN_HEIGHT = 800;

export function canFitEmbeddedLocusAssetInspectorPanel(
  viewportWidth = typeof window !== "undefined" ? window.innerWidth : 0,
  viewportHeight = typeof window !== "undefined" ? window.innerHeight : 0,
): boolean {
  return viewportWidth >= EMBEDDED_PANEL_FIT_MIN_WIDTH
    && viewportHeight >= EMBEDDED_PANEL_FIT_MIN_HEIGHT;
}

export interface LocusAssetInspectorPanelState {
  open: boolean;
  payload: LocusAssetInspectorWindowPayload | null;
  /** Bumped on every open so the panel can re-focus / unfold for repeat targets. */
  revision: number;
}

const state = reactive<LocusAssetInspectorPanelState>({
  open: false,
  payload: null,
  revision: 0,
});

/**
 * Whether the current window hosts the embedded floating inspector panel.
 * Set by App.vue for the main Locus window; standalone windows (unity-embed,
 * progress windows, view-host windows) leave it false so embedded opens fall
 * back to an inspector tab in the View host window system.
 */
const panelHostAvailable = ref(false);

export function setLocusAssetInspectorPanelHostAvailable(available: boolean) {
  panelHostAvailable.value = available;
}

export function isLocusAssetInspectorPanelHostAvailable(): boolean {
  return panelHostAvailable.value;
}

export function openLocusAssetInspectorPanel(
  payload: LocusAssetInspectorWindowPayload,
): boolean {
  if (!panelHostAvailable.value) return false;
  const nextPayload = normalizeLocusAssetInspectorPayload(payload);
  if (!isValidLocusAssetInspectorPayload(nextPayload)) return false;
  state.payload = nextPayload;
  state.open = true;
  state.revision++;
  return true;
}

export function closeLocusAssetInspectorPanel() {
  state.open = false;
}

export function useLocusAssetInspectorPanel() {
  return {
    state,
    open: openLocusAssetInspectorPanel,
    close: closeLocusAssetInspectorPanel,
  };
}

/**
 * Open the Locus asset inspector in the requested mode. Embedded mode prefers
 * the floating panel inside the current window and falls back to a standalone
 * inspector tab when no panel host is available. Auto mode picks the embedded
 * panel only when the window is large enough to fully expand it.
 */
export async function openLocusAssetInspector(
  payload: LocusAssetInspectorWindowPayload,
  mode: LocusAssetInspectorMode = "auto",
): Promise<boolean> {
  const preferEmbedded = mode === "embedded"
    || (mode === "auto" && canFitEmbeddedLocusAssetInspectorPanel());
  if (preferEmbedded && openLocusAssetInspectorPanel(payload)) {
    return true;
  }
  return openLocusAssetInspectorWindow(payload);
}
