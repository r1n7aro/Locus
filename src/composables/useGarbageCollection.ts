import { ref } from "vue";
import { t } from "../i18n";
import { garbageCollection } from "../services/garbageCollection";
import { normalizeAppError } from "../services/errors";
import type { WorkspaceRef } from "../services/project";
import { useNotificationStore } from "../stores/notification";

const pending = ref(false);
const operation = "garbageCollection";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(2)} ${units[unit]}`;
}

export function useGarbageCollection() {
  const notifications = useNotificationStore();

  async function collect(workspaceRef: WorkspaceRef | null) {
    if (pending.value) return;
    pending.value = true;
    const reference = workspaceRef ? { ...workspaceRef } : null;
    const notice = notifications.addNotice("info", t("chat.command.gcRunning"), {
      operation, sticky: true, spinner: true,
    });
    try {
      const results = await garbageCollection(reference);
      console.info("[garbage-collection]", results);
      for (const entry of results) {
        const name = t(entry.kind === "session" ? "chat.command.gcSession" : "chat.command.gcProject");
        if (entry.error) {
          notifications.addNotice("error", t("chat.command.gcFailed", name, entry.error), { operation });
        } else if (entry.skipped) {
          notifications.addNotice("info", t("chat.command.gcSkipped", name), { operation });
        } else if (entry.result) {
          const { before, after, reclaimedBytes, warning } = entry.result;
          const summary = t("chat.command.gcDone", name, formatSize(reclaimedBytes),
            formatSize(before.databaseBytes + before.walBytes),
            formatSize(after.databaseBytes + after.walBytes));
          notifications.addNotice(warning ? "warning" : "success",
            warning ? `${summary}\n${warning}` : summary, { operation, ttl: 12000 });
        }
      }
    } catch (error) {
      notifications.addNotice("error", t("chat.command.gcFailed",
        t("chat.command.gcTitle"), normalizeAppError(error).message), { operation });
    } finally {
      notifications.removeNotice(notice);
      pending.value = false;
    }
  }

  return { collect };
}
