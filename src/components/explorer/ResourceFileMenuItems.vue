<script setup lang="ts">
import { Copy, Download, FolderOpen, PencilLine, Trash2 } from "lucide";
import { t } from "../../i18n";
import LucideIcon from "../icons/LucideIcon.vue";
import ResourcePathMenuItem from "./ResourcePathMenuItem.vue";
import { isSkillPackageRootDocument } from "../../composables/useKnowledgeState";
import { knowledgeResourceManagedHint } from "../knowledge/knowledgeResourceActions";
import { resourcePathKey, type ExplorerResourceAction, type ExplorerResourceTarget } from "./explorerResourceActions";
import { computed } from "vue";
const props = defineProps<{ target: ExplorerResourceTarget }>();
const emit = defineEmits<{ action: [action: ExplorerResourceAction]; close: [] }>();
const hint = computed(() => props.target.document && knowledgeResourceManagedHint(props.target.document));
const isPackage = computed(() => !!props.target.document && isSkillPackageRootDocument(props.target.document));
</script>

<template>
  <button v-if="!isPackage" type="button" :disabled="!!hint" :title="hint ? t(hint) : undefined" @click="emit('action', 'rename')">
    <LucideIcon :icon="PencilLine" :size="13" />{{ t('knowledge.explorer.rename') }}
  </button>
  <button v-else-if="!target.document?.externalSource?.locator?.startsWith('external://')" type="button" @click="emit('action', 'export')">
    <LucideIcon :icon="Download" :size="13" />{{ t('knowledge.explorer.exportSkillPackage') }}
  </button>
  <div class="base-context-menu-separator" role="separator" />
  <button type="button" @click="emit('action', 'reveal')"><LucideIcon :icon="FolderOpen" :size="13" />{{ t('knowledge.explorer.openInFileSystem') }}</button>
  <button type="button" @click="emit('action', 'copy')"><LucideIcon :icon="Copy" :size="13" />{{ t('knowledge.explorer.copyRelativePath') }}</button>
  <ResourcePathMenuItem :path-key="resourcePathKey(target)" @select="emit('close')" />
  <div class="base-context-menu-separator" role="separator" />
  <button type="button" class="danger" :disabled="!!hint" :title="hint ? t(hint) : undefined" @click="emit('action', 'delete')">
    <LucideIcon :icon="Trash2" :size="13" />{{ t(isPackage ? 'knowledge.explorer.deletePackage' : 'knowledge.explorer.delete') }}
  </button>
</template>
