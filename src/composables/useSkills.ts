import { computed, ref } from "vue";
import { listSkills } from "../services/knowledge";
import type { WorkspaceRef } from "../services/project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type { SkillManifest } from "../types";

const skillItemsByWorkspace = ref<Record<string, SkillManifest[]>>({});
const loadedWorkspaces = ref<Record<string, boolean>>({});
const requestVersions = new Map<string, number>();
const inflightLoads = new Map<string, Promise<void>>();

function workspaceKey(workspaceRef: WorkspaceRef | null): string {
  if (!workspaceRef) return "";
  return `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? ""}`;
}

export function useSkills() {
  const workspaceContextStore = useWorkspaceContextStore();
  const currentWorkspaceRef = computed(() => workspaceContextStore.focusedWorkspaceRef);
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
    const scope = options?.workspaceRef ?? currentWorkspaceRef.value;
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
      } catch {
        if (requestVersions.get(key) === requestVersion) {
          skillItemsByWorkspace.value[key] = [];
        }
      } finally {
        if (inflightLoads.get(key) === request) inflightLoads.delete(key);
      }
    })();
    inflightLoads.set(key, request);
    return request;
  }

  return { skillItems, skillsLoaded, loadSkills };
}
