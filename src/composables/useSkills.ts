import { ref } from "vue";
import { listSkills } from "../services/knowledge";
import type { SkillManifest } from "../types";

const skillItems = ref<SkillManifest[]>([]);

export function useSkills() {
  async function loadSkills() {
    try {
      skillItems.value = await listSkills();
    } catch {
      skillItems.value = [];
    }
  }

  return { skillItems, loadSkills };
}
