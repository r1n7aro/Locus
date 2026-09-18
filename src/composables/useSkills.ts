import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import { listSkills } from "../services/knowledge";
import type { WorkspaceRef } from "../services/project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type { SkillManifest } from "../types";

const skillItemsByWorkspace = ref<Record<string, SkillManifest[]>>({});
const loadedWorkspaces = ref<Record<string, boolean>>({});
const requestVersions = new Map<string, number>();
const inflightLoads = new Map<string, Promise<void>>();
const invalidations = ref<Record<string, number>>({});

function workspaceKey(workspaceRef: WorkspaceRef | null): string {
  if (!workspaceRef) return "";
  return `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? ""}:${workspaceRef.expectedMaterializationEpoch ?? "empty"}`;
}

/** Invalidate snapshots without loading unused/background checkout generations. */
export function invalidateSkills(workspaceRef?: WorkspaceRef): void {
  const prefix = workspaceRef ? `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? ""}:` : "";
  // Optional epoch handles and fully specified handles can address the same
  // runtime. Invalidate their aliases together; loads still validate each ref.
  const keys = [...requestVersions.keys()].filter((key) => !workspaceRef || key.startsWith(prefix));
  for (const key of keys) {
    loadedWorkspaces.value[key] = false;
    requestVersions.set(key, (requestVersions.get(key) ?? 0) + 1);
    inflightLoads.delete(key);
    invalidations.value[key] = (invalidations.value[key] ?? 0) + 1;
  }
}

export function useSkills(workspaceRef?: MaybeRefOrGetter<WorkspaceRef | null | undefined>) {
  const workspaceContextStore = useWorkspaceContextStore();
  const currentWorkspaceRef = computed(() => (
    workspaceRef === undefined ? workspaceContextStore.focusedWorkspaceRef : toValue(workspaceRef) ?? null
  ));
  const currentWorkspaceKey = computed(() => workspaceKey(currentWorkspaceRef.value));
  const skillItems = computed(() => (
    currentWorkspaceKey.value
      ? skillItemsByWorkspace.value[currentWorkspaceKey.value] ?? []
      : []
  ));
  const skillsLoaded = computed(() => (
    !!currentWorkspaceKey.value && !!loadedWorkspaces.value[currentWorkspaceKey.value]
  ));

  // Skill manifests are cached per checkout generation. Background windows
  // and reverse-completing requests can update only the scope they captured.
  function loadSkills(options?: {
    force?: boolean;
    workspaceRef?: WorkspaceRef | null;
  }): Promise<void> {
    const target = options && "workspaceRef" in options ? options.workspaceRef : currentWorkspaceRef.value;
    const scope = target ? { ...target } : null;
    const key = workspaceKey(scope);
    if (!scope || !key) return Promise.resolve();
    if (!options?.force && loadedWorkspaces.value[key]) return Promise.resolve();
    const existing = inflightLoads.get(key);
    if (!options?.force && existing) return existing;

    const requestVersion = (requestVersions.get(key) ?? 0) + 1;
    requestVersions.set(key, requestVersion);
    let request!: Promise<void>;
    request = (async () => {
      try {
        const nextSkills = await listSkills(scope);
        if (requestVersions.get(key) === requestVersion) {
          skillItemsByWorkspace.value[key] = nextSkills;
          loadedWorkspaces.value[key] = true;
        }
      } catch (error) {
        if (requestVersions.get(key) === requestVersion) {
          skillItemsByWorkspace.value[key] = [];
          loadedWorkspaces.value[key] = false;
          console.warn("[Skills] failed to load workspace skills:", error);
        }
      } finally {
        if (inflightLoads.get(key) === request) inflightLoads.delete(key);
      }
    })();
    inflightLoads.set(key, request);
    return request;
  }

  watch(
    () => [currentWorkspaceKey.value, invalidations.value[currentWorkspaceKey.value] ?? 0] as const,
    () => { void loadSkills(); },
    { immediate: true },
  );

  return { skillItems, skillsLoaded, loadSkills };
}
