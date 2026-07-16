import { ref } from "vue";
import { listSkills } from "../services/knowledge";
import type { SkillManifest } from "../types";

const skillItems = ref<SkillManifest[]>([]);
const skillsLoaded = ref(false);
let loadSkillsRequestId = 0;
let inflightLoad: Promise<void> | null = null;

export function useSkills() {
  // Stale-while-revalidate: the previous list stays rendered until the fresh
  // scan lands. Concurrent non-forced callers share the in-flight request so
  // rapid package switches don't fan out duplicate full manifest scans.
  function loadSkills(options?: { force?: boolean }): Promise<void> {
    if (!options?.force && inflightLoad) return inflightLoad;
    const requestId = ++loadSkillsRequestId;
    let request: Promise<void> | null = null;
    request = (async () => {
      try {
        const nextSkills = await listSkills();
        if (requestId === loadSkillsRequestId) {
          skillItems.value = nextSkills;
          skillsLoaded.value = true;
        }
      } catch {
        if (requestId === loadSkillsRequestId) {
          skillItems.value = [];
        }
      } finally {
        if (inflightLoad === request) {
          inflightLoad = null;
        }
      }
    })();
    inflightLoad = request;
    return request;
  }

  return { skillItems, skillsLoaded, loadSkills };
}
