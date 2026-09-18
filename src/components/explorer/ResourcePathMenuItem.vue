<script setup lang="ts">
import { File, FolderTree } from "lucide";
import { t } from "../../i18n";
import { useExplorerPathDisplay } from "../../composables/useExplorerPathDisplay";
import LucideIcon from "../icons/LucideIcon.vue";
const props = defineProps<{ pathKey: string }>();
const emit = defineEmits<{ select: [] }>();
const { showsFullPath, togglePath } = useExplorerPathDisplay();
</script>

<template>
  <button type="button" @click="togglePath(props.pathKey); emit('select')">
    <LucideIcon :icon="showsFullPath(pathKey) ? File : FolderTree" :size="13" />
    {{ t(showsFullPath(pathKey) ? 'development.showFileName' : 'development.showFullPath') }}
  </button>
</template>
