
<script setup lang="ts">
import { computed } from "vue";
import {
  selectUnityAsset,
  selectUnitySceneObject,
  openUnitySceneObjectInspector,
  classifyUnitySceneObjectError,
} from "../services/unity";
import { t } from "../i18n";
import { useNotificationStore } from "../stores/notification";

const props = defineProps<{
  path: string;
  removable?: boolean;
}>();

const emit = defineEmits<{
  remove: [];
}>();

const notificationStore = useNotificationStore();

const sceneObjectRef = computed(() => {
  const normalized = props.path.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  const match = normalized.match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
  if (!match) return null;
  const objectPath = match[2].replace(/^\/+|\/+$/g, "");
  if (!match[1] || !objectPath) return null;
  return {
    scenePath: match[1],
    objectPath,
  };
});

const displayName = computed(() => {
  const parts = (sceneObjectRef.value?.objectPath ?? props.path).split("/");
  const fileName = parts[parts.length - 1] || props.path;
  const dotIdx = fileName.lastIndexOf(".");
  return dotIdx > 0 ? fileName.substring(0, dotIdx) : fileName;
});

const ext = computed(() => {
  if (sceneObjectRef.value) return "unity-scene-object";
  const dotIdx = props.path.lastIndexOf(".");
  return dotIdx > 0 ? props.path.substring(dotIdx + 1).toLowerCase() : "";
});

const typeIcon = computed(() => {
  switch (ext.value) {
    case "unity-scene-object": return "□";
    case "prefab": return "◆";
    case "unity": return "◈";
    case "asset": return "◇";
    case "mat": return "●";
    case "cs": return "#";
    case "shader": case "shadergraph": return "◎";
    case "png": case "jpg": case "jpeg": case "tga": case "psd": return "▣";
    case "fbx": case "obj": case "blend": return "△";
    case "anim": case "controller": return "▶";
    case "mp3": case "wav": case "ogg": return "♪";
    default: return "◇";
  }
});

async function handleClick(e: MouseEvent) {
  try {
    if (sceneObjectRef.value) {
      const { scenePath, objectPath } = sceneObjectRef.value;
      if (e.ctrlKey || e.metaKey) {
        await openUnitySceneObjectInspector(scenePath, objectPath);
        return;
      }
      await selectUnitySceneObject(scenePath, objectPath);
      return;
    }
    await selectUnityAsset(props.path);
  } catch (error) {
    if (sceneObjectRef.value) {
      notifyUnitySceneObjectError(error, sceneObjectRef.value.scenePath, sceneObjectRef.value.objectPath);
    }
  }
}

function notifyUnitySceneObjectError(error: unknown, scenePath: string, objectPath: string) {
  const kind = classifyUnitySceneObjectError(error);
  const message = kind === "sceneNotLoaded"
    ? t("chat.sceneObject.sceneNotLoaded", scenePath)
    : kind === "objectMissing"
      ? t("chat.sceneObject.objectMissing", objectPath)
      : t("chat.sceneObject.openFailed", `${scenePath}/${objectPath}`);
  notificationStore.addNotice("warning", message, {
    operation: "unitySceneObjectRef",
    code: `unity.sceneObject.${kind}`,
    replaceOperation: true,
  });
}
</script>

<template>
  <span class="asset-chip" :title="path" @click.stop="handleClick">
    <span class="asset-chip-icon">{{ typeIcon }}</span>
    <span class="asset-chip-name">{{ displayName }}</span>
    <button v-if="removable" class="asset-chip-remove" @click.stop="emit('remove')">&times;</button>
  </span>
</template>

<style scoped>
.asset-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 8px;
  border-radius: 4px;
  background: var(--hover-bg, rgba(255,255,255,0.08));
  border: 1px solid var(--border-color, rgba(255,255,255,0.12));
  cursor: pointer;
  font-size: 13px;
  line-height: 1.5;
  vertical-align: baseline;
  transition: background 0.15s, border-color 0.15s;
  max-width: 300px;
  white-space: nowrap;
}

.asset-chip:hover {
  background: var(--active-bg, rgba(255,255,255,0.14));
  border-color: var(--accent-color, #4a9eff);
}

.asset-chip-icon {
  font-size: 11px;
  opacity: 0.7;
  flex-shrink: 0;
}

.asset-chip-name {
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 500;
}

.asset-chip-remove {
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 14px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border-radius: 3px;
  margin-left: 2px;
  box-shadow: none;
}

.asset-chip-remove:hover {
  background: rgba(255, 80, 80, 0.2);
  color: #e55;
}
</style>
