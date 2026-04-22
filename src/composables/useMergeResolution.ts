import { ref, computed, type Ref, type ComputedRef } from "vue";
import type {
  MergeField,
  MergeSessionPayload,
  MergeSide,
  MergeTargetInspector,
  FieldResolution,
} from "../types";

export interface MergeResolutionState {
  fieldResolutions: Ref<Map<string, MergeSide>>;
  panelResolutions: Ref<Map<string, MergeSide>>;
  targetResolutions: Ref<Map<string, MergeSide>>;
  unresolvedCount: ComputedRef<number>;
  canApply: ComputedRef<boolean>;
  initializeSession: (session: MergeSessionPayload | null) => void;
  acceptField: (fieldId: string, side: MergeSide, field?: MergeField) => void;
  acceptPanel: (targetId: string, panelIndex: number, side: MergeSide, inspector?: MergeTargetInspector) => void;
  acceptTarget: (targetId: string, side: MergeSide, inspector?: MergeTargetInspector) => void;
  acceptAll: (side: MergeSide) => void;
  reset: () => void;
  registerConflictFields: (inspector: MergeTargetInspector) => void;
  buildResolutions: () => Record<string, FieldResolution>;
  isTargetLoaded: (targetId: string) => boolean;
  isTargetUnresolved: (targetId: string) => boolean;
  pendingMaterializationTargetIds: () => string[];
}

export function useMergeResolution(): MergeResolutionState {
  const fieldResolutions = ref<Map<string, MergeSide>>(new Map());
  const panelResolutions = ref<Map<string, MergeSide>>(new Map());
  const targetResolutions = ref<Map<string, MergeSide>>(new Map());

  const targetConflictCounts = ref<Map<string, number>>(new Map());
  const loadedTargetConflictIds = ref<Map<string, Set<string>>>(new Map());

  const unresolvedCount = computed(() => {
    let unresolved = 0;
    for (const [targetId, totalConflicts] of targetConflictCounts.value) {
      if (totalConflicts <= 0) continue;
      if (targetResolutions.value.has(targetId)) continue;

      const loadedIds = loadedTargetConflictIds.value.get(targetId);
      if (!loadedIds) {
        unresolved += totalConflicts;
        continue;
      }

      let unresolvedLoaded = 0;
      for (const fieldId of loadedIds) {
        if (!fieldResolutions.value.has(fieldId)) {
          unresolvedLoaded++;
        }
      }
      unresolved += unresolvedLoaded + Math.max(0, totalConflicts - loadedIds.size);
    }
    return unresolved;
  });

  const canApply = computed(() => unresolvedCount.value === 0);

  function reset() {
    fieldResolutions.value = new Map();
    panelResolutions.value = new Map();
    targetResolutions.value = new Map();
    targetConflictCounts.value = new Map();
    loadedTargetConflictIds.value = new Map();
  }

  function initializeSession(session: MergeSessionPayload | null) {
    targetConflictCounts.value = new Map(
      (session?.targets ?? []).map((target) => [target.id, target.conflictCount]),
    );
    loadedTargetConflictIds.value = new Map();
    fieldResolutions.value = new Map();
    panelResolutions.value = new Map();
    targetResolutions.value = new Map();
  }

  function collectConflictFields(field: MergeField, output: Set<string>) {
    if (field.children.length === 0) {
      if (field.mergeState === "conflict") {
        output.add(field.id);
      }
      return;
    }
    for (const child of field.children) {
      collectConflictFields(child, output);
    }
  }

  function setFieldResolution(fieldId: string, side: MergeSide, field?: MergeField) {
    const next = new Map(fieldResolutions.value);
    next.set(fieldId, side);
    fieldResolutions.value = next;

    if (!field?.children?.length) return;
    for (const child of field.children) {
      setFieldResolution(child.id, side, child);
    }
  }

  function registerConflictFields(inspector: MergeTargetInspector) {
    const ids = new Set<string>();
    for (const panel of inspector.panels) {
      for (const field of panel.fields) {
        collectConflictFields(field, ids);
      }
    }

    const nextLoaded = new Map(loadedTargetConflictIds.value);
    nextLoaded.set(inspector.targetId, ids);
    loadedTargetConflictIds.value = nextLoaded;

    const bulkChoice = targetResolutions.value.get(inspector.targetId);
    if (bulkChoice) {
      const nextFields = new Map(fieldResolutions.value);
      for (const fieldId of ids) {
        nextFields.set(fieldId, bulkChoice);
      }
      fieldResolutions.value = nextFields;
    }
  }

  function acceptField(fieldId: string, side: MergeSide, field?: MergeField) {
    setFieldResolution(fieldId, side, field);
  }

  function acceptPanel(targetId: string, panelIndex: number, side: MergeSide, inspector?: MergeTargetInspector) {
    const key = `${targetId}:${panelIndex}`;
    const nextPanels = new Map(panelResolutions.value);
    nextPanels.set(key, side);
    panelResolutions.value = nextPanels;

    if (!inspector) return;
    const panel = inspector.panels[panelIndex];
    if (!panel) return;
    for (const field of panel.fields) {
      setFieldResolution(field.id, side, field);
    }
  }

  function acceptTarget(targetId: string, side: MergeSide, inspector?: MergeTargetInspector) {
    const nextTargets = new Map(targetResolutions.value);
    nextTargets.set(targetId, side);
    targetResolutions.value = nextTargets;

    if (!inspector) return;
    for (const panel of inspector.panels) {
      for (const field of panel.fields) {
        setFieldResolution(field.id, side, field);
      }
    }
  }

  function acceptAll(side: MergeSide) {
    const nextTargets = new Map(targetResolutions.value);
    for (const [targetId, conflictCount] of targetConflictCounts.value) {
      if (conflictCount > 0) {
        nextTargets.set(targetId, side);
      }
    }
    targetResolutions.value = nextTargets;

    const nextFields = new Map(fieldResolutions.value);
    for (const fieldIds of loadedTargetConflictIds.value.values()) {
      for (const fieldId of fieldIds) {
        nextFields.set(fieldId, side);
      }
    }
    fieldResolutions.value = nextFields;
  }

  function buildResolutions(): Record<string, FieldResolution> {
    const result: Record<string, FieldResolution> = {};
    for (const [fieldId, side] of fieldResolutions.value) {
      result[fieldId] = { side };
    }
    return result;
  }

  function isTargetLoaded(targetId: string): boolean {
    return loadedTargetConflictIds.value.has(targetId);
  }

  function isTargetUnresolved(targetId: string): boolean {
    const totalConflicts = targetConflictCounts.value.get(targetId) ?? 0;
    if (totalConflicts <= 0) return false;
    if (targetResolutions.value.has(targetId)) return false;
    const loadedIds = loadedTargetConflictIds.value.get(targetId);
    if (!loadedIds) return true;
    for (const fieldId of loadedIds) {
      if (!fieldResolutions.value.has(fieldId)) return true;
    }
    return totalConflicts > loadedIds.size;
  }

  function pendingMaterializationTargetIds(): string[] {
    const pending: string[] = [];
    for (const [targetId, conflictCount] of targetConflictCounts.value) {
      if (conflictCount <= 0) continue;
      if (!targetResolutions.value.has(targetId)) continue;
      const loadedIds = loadedTargetConflictIds.value.get(targetId);
      if (!loadedIds || loadedIds.size < conflictCount) {
        pending.push(targetId);
      }
    }
    return pending;
  }

  return {
    fieldResolutions,
    panelResolutions,
    targetResolutions,
    unresolvedCount,
    canApply,
    initializeSession,
    acceptField,
    acceptPanel,
    acceptTarget,
    acceptAll,
    reset,
    registerConflictFields,
    buildResolutions,
    isTargetLoaded,
    isTargetUnresolved,
    pendingMaterializationTargetIds,
  };
}
