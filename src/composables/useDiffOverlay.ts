import { inject, provide, ref, type Ref } from "vue";
import type { FileDiffPayload } from "../types";

const DIFF_OVERLAY_KEY = "diffOverlay";

export interface DiffOverlayHeaderAction {
  label: string;
  onClick: () => void;
  danger?: boolean;
  confirmMessage?: string;
}

export interface DiffOverlayApi {
  open(payload: FileDiffPayload, headerActions?: DiffOverlayHeaderAction[]): void;
  close(): void;
  payload: Ref<FileDiffPayload | null>;
  visible: Ref<boolean>;
  headerActions: Ref<DiffOverlayHeaderAction[]>;
}

export function provideDiffOverlay(): DiffOverlayApi {
  const payload = ref<FileDiffPayload | null>(null);
  const visible = ref(false);
  const headerActions = ref<DiffOverlayHeaderAction[]>([]);

  const api: DiffOverlayApi = {
    open(p: FileDiffPayload, actions?: DiffOverlayHeaderAction[]) {
      payload.value = p;
      headerActions.value = actions ?? [];
      visible.value = true;
    },
    close() {
      visible.value = false;
      headerActions.value = [];
    },
    payload,
    visible,
    headerActions,
  };

  provide(DIFF_OVERLAY_KEY, api);
  return api;
}

export function useDiffOverlay(): DiffOverlayApi {
  const api = inject<DiffOverlayApi>(DIFF_OVERLAY_KEY);
  if (!api) {
    console.warn("[useDiffOverlay] provider not found — diff overlay will be disabled");
    const payload = ref<FileDiffPayload | null>(null);
    const visible = ref(false);
    const headerActions = ref<DiffOverlayHeaderAction[]>([]);
    return {
      open() {},
      close() {},
      payload,
      visible,
      headerActions,
    };
  }
  return api;
}
