import { ref } from "vue";
import { listSkills } from "../services/knowledge";
import type { SkillManifest } from "../types";

const skillItems = ref<SkillManifest[]>([]);
let loadSkillsRequestId = 0;

export function useSkills() {
  async function loadSkills() {
    const requestId = ++loadSkillsRequestId;
    try {
      const nextSkills = await listSkills();
      if (requestId === loadSkillsRequestId) {
        skillItems.value = nextSkills;
      }
    } catch {
      if (requestId === loadSkillsRequestId) {
        skillItems.value = [];
      }
    }
  }

  return { skillItems, loadSkills };
}
