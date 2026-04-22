<script setup lang="ts">
import type { GitProbeResult } from "../../types";
import { t } from "../../i18n";
import GitMissingHelp from "../git/GitMissingHelp.vue";

defineProps<{
  isRepo: boolean;
  loading: boolean;
  initLoading: boolean;
  initError: string | null;
  gitAvailable: boolean;
  gitHelpText: string;
  gitProbeState: GitProbeResult | null;
  showGitConfigModal: boolean;
  gitConfigName: string;
  gitConfigEmail: string;
  gitConfigSaving: boolean;
  gitConfigError: string;
}>();

const emit = defineEmits<{
  (e: "init"): void;
  (e: "configSave"): void;
  (e: "configCancel"): void;
  (e: "refreshGitProbe"): void;
  (e: "update:gitConfigName", value: string): void;
  (e: "update:gitConfigEmail", value: string): void;
}>();
</script>

<template>
  <!-- Not a repo overlay -->
  <div v-if="!isRepo && !loading" class="vcs-init-overlay">
    <div class="empty-icon-sm">&#8709;</div>
    <div class="empty-text">{{ gitAvailable ? t("collab.notVcs") : t("onboarding.vcs.gitMissing") }}</div>
    <div class="empty-hint">{{ gitHelpText || (gitAvailable ? t("collab.initHint") : t("git.detect.missing")) }}</div>
    <div class="vcs-init-options">
      <button class="vcs-init-btn git-init-btn" @click="emit('init')" :disabled="initLoading || !gitAvailable">
        <span class="vcs-init-icon">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor"><path d="M21.62 11.108l-8.731-8.729a1.292 1.292 0 0 0-1.823 0L9.257 4.19l2.299 2.3a1.532 1.532 0 0 1 1.939 1.95l2.214 2.217a1.532 1.532 0 1 1-.918.863l-2.066-2.066v5.432a1.534 1.534 0 1 1-1.26-.045V9.306a1.532 1.532 0 0 1-.832-2.01L8.363 5.025l-5.99 5.985a1.292 1.292 0 0 0 0 1.824l8.731 8.729a1.292 1.292 0 0 0 1.823 0l8.691-8.691a1.292 1.292 0 0 0 .002-1.764"/></svg>
        </span>
        <span class="vcs-init-label">{{ t("collab.gitInit") }}</span>
        <span class="vcs-init-desc">{{ t("collab.gitInitDesc") }}</span>
      </button>
      <button class="vcs-init-btn p4-init-btn" disabled>
        <span class="vcs-init-icon">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor"><path d="M4.5 2v20h3.375V14.5H12c3.59 0 6.5-2.686 6.5-6.25S15.59 2 12 2H4.5zm3.375 3h4.125c1.726 0 3.125 1.455 3.125 3.25S13.726 11.5 12 11.5H7.875V5z"/></svg>
        </span>
        <span class="vcs-init-label">{{ t("collab.p4Init") }}</span>
        <span class="vcs-init-desc">{{ t("collab.p4InitDesc") }}</span>
      </button>
    </div>
    <GitMissingHelp
      v-if="!gitAvailable"
      :probe="gitProbeState"
      @resolved="emit('refreshGitProbe')"
    />
    <div v-if="initError" class="init-error">{{ initError }}</div>

    <Teleport to="body">
      <div v-if="showGitConfigModal" class="git-config-overlay" @click.self="emit('configCancel')">
        <div class="git-config-modal">
          <div class="git-config-header">
            <span>{{ t("git.config.title") }}</span>
            <button class="git-config-close" @click="emit('configCancel')">&times;</button>
          </div>
          <div class="git-config-body">
            <p class="git-config-desc">{{ t("git.config.desc") }}</p>
            <label class="git-config-label">{{ t("git.config.name") }}</label>
            <input
              :value="gitConfigName"
              @input="emit('update:gitConfigName', ($event.target as HTMLInputElement).value)"
              class="git-config-input"
              :placeholder="t('git.config.namePlaceholder')"
              @keydown.enter="emit('configSave')"
            />
            <label class="git-config-label">{{ t("git.config.email") }}</label>
            <input
              :value="gitConfigEmail"
              @input="emit('update:gitConfigEmail', ($event.target as HTMLInputElement).value)"
              type="email"
              class="git-config-input"
              :placeholder="t('git.config.emailPlaceholder')"
              @keydown.enter="emit('configSave')"
            />
            <div v-if="gitConfigError" class="msg error" style="margin-top:8px;">{{ t("git.config.saveFailed", gitConfigError) }}</div>
          </div>
          <div class="git-config-footer">
            <button class="collab-btn secondary" @click="emit('configCancel')">{{ t("common.cancel") }}</button>
            <button
              class="collab-btn primary"
              :disabled="gitConfigSaving || !gitConfigName.trim() || !gitConfigEmail.trim()"
              @click="emit('configSave')"
            >
              {{ gitConfigSaving ? t("git.config.saving") : t("git.config.save") }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>

  <!-- Loading state -->
  <div v-else-if="loading && !isRepo" class="vcs-init-overlay">
    <div class="empty-text">{{ t("common.loading") }}</div>
  </div>
</template>
