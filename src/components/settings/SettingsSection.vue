<script setup lang="ts">
import { t } from "../../i18n";

// One titled block of a settings page. `id` matches the section id in
// `nav.ts`, so settings search can scroll to it.
defineProps<{
  id?: string;
  title?: string;
  description?: string;
  experimental?: boolean;
}>();
</script>

<template>
  <section class="settings-section" :data-settings-section="id">
    <div v-if="title" class="section-label">
      <span>{{ title }}</span>
      <span v-if="experimental" class="settings-badge settings-badge-experimental">
        {{ t("settings.badge.experimental") }}
      </span>
    </div>
    <p v-if="description" class="section-desc">{{ description }}</p>
    <div class="settings-section-body">
      <slot />
    </div>
  </section>
</template>
