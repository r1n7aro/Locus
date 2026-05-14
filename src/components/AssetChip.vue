
<script setup lang="ts">
import { computed } from "vue";
import {
  selectUnityAsset,
  selectUnitySceneObject,
  openUnitySceneObjectInspector,
  classifyUnitySceneObjectError,
} from "../services/unity";
import { knowledgeRevealTarget } from "../services/knowledge";
import { normalizeAppError } from "../services/errors";
import { t } from "../i18n";
import { useNotificationStore } from "../stores/notification";
import type { AssetRefKind, KnowledgeDocumentType } from "../types";
import LucideIcon from "./icons/LucideIcon.vue";
import {
  unityAssetIconClassForKind,
  unityAssetIconKindForPath,
  unityAssetIconNodeForKind,
} from "./icons/unityAssetIcons";

const props = defineProps<{
  path: string;
  kind?: AssetRefKind;
  removable?: boolean;
}>();

const emit = defineEmits<{
  remove: [];
}>();

const notificationStore = useNotificationStore();
const KNOWLEDGE_REF_ROOT_RE = /^(design|memory|skill|reference)\/.+\.md$/i;

const normalizedPath = computed(() =>
  props.path.trim().replace(/\\/g, "/").replace(/\/+$/, ""),
);

const knowledgeRef = computed(() => {
  const match = normalizedPath.value.match(KNOWLEDGE_REF_ROOT_RE);
  if (!match) return null;
  return {
    docType: match[1].toLowerCase() as KnowledgeDocumentType,
    path: normalizedPath.value,
  };
});

const sceneObjectRef = computed(() => {
  const normalized = normalizedPath.value;
  const match = normalized.match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
  if (!match) return null;
  const objectPath = match[2].replace(/^\/+|\/+$/g, "");
  if (!match[1] || !objectPath) return null;
  return {
    scenePath: match[1],
    objectPath,
  };
});

const effectiveKind = computed<AssetRefKind>(() =>
  props.kind ?? (knowledgeRef.value ? "knowledge" : sceneObjectRef.value ? "sceneObject" : "asset"),
);

const displayName = computed(() => {
  const parts = (sceneObjectRef.value?.objectPath ?? (normalizedPath.value || props.path)).split("/");
  const fileName = parts[parts.length - 1] || normalizedPath.value || props.path;
  const dotIdx = fileName.lastIndexOf(".");
  return dotIdx > 0 ? fileName.substring(0, dotIdx) : fileName;
});

const iconKind = computed(() =>
  effectiveKind.value === "knowledge"
    ? "text"
    : unityAssetIconKindForPath(normalizedPath.value || props.path, {
        isSceneObject: !!sceneObjectRef.value,
        fallbackKind: "asset",
      }),
);

const iconNode = computed(() => unityAssetIconNodeForKind(iconKind.value));

async function handleClick(e: MouseEvent) {
  try {
    if (knowledgeRef.value) {
      await knowledgeRevealTarget({
        kind: "document",
        docType: knowledgeRef.value.docType,
        path: knowledgeRef.value.path,
      });
      return;
    }
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
    if (knowledgeRef.value) {
      notifyKnowledgeRefError(error);
      return;
    }
    if (sceneObjectRef.value) {
      notifyUnitySceneObjectError(error, sceneObjectRef.value.scenePath, sceneObjectRef.value.objectPath);
    }
  }
}

function notifyKnowledgeRefError(error: unknown) {
  const err = normalizeAppError(error);
  notificationStore.addNotice("warning", t("chat.knowledgeRef.openFailed", err.message), {
    code: err.code,
    operation: "knowledgeRef",
    replaceOperation: true,
  });
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
  <span
    class="asset-chip"
    :title="path"
    :data-ref-kind="effectiveKind"
    :data-knowledge-type="knowledgeRef?.docType"
    :data-knowledge-path="knowledgeRef?.path"
    :data-file-path="effectiveKind === 'knowledge' ? undefined : normalizedPath"
    :data-asset-path="effectiveKind === 'asset' ? normalizedPath : undefined"
    :data-scene-path="sceneObjectRef?.scenePath"
    :data-scene-object-path="sceneObjectRef?.objectPath"
    @click.stop="handleClick"
  >
    <LucideIcon
      class="asset-chip-icon"
      :class="unityAssetIconClassForKind(iconKind)"
      :icon="iconNode"
    />
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
  width: 14px;
  min-width: 14px;
  height: 14px;
  opacity: 0.95;
  flex-shrink: 0;
  display: block;
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
  background: color-mix(in srgb, var(--status-error-bg, var(--hover-bg)) 76%, transparent);
  color: var(--status-error-fg, var(--text-color));
}
</style>
