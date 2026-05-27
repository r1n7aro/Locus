import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AboutSettings layout", () => {
  it("adds an about category to settings navigation", () => {
    const source = read("src/components/SettingsView.vue");

    expect(source).toContain('import AboutSettings from "./settings/AboutSettings.vue"');
    expect(source).toContain(`:class="{ active: activeCategory === 'about' }"`);
    expect(source).toContain(`@click="activeCategory = 'about'"`);
    expect(source).toContain('{{ t("settings.tab.about") }}');
    expect(source).toContain(`<template v-if="activeCategory === 'about'">`);
    expect(source).toContain("<AboutSettings />");
  });

  it("renders app identity, organization, and contact details", () => {
    const source = read("src/components/settings/AboutSettings.vue");
    const appUpdateStore = read("src/stores/appUpdate.ts");

    expect(source).toContain('import { useAppUpdateStore } from "../../stores/appUpdate"');
    expect(source).toContain('import BaseButton from "../ui/BaseButton.vue"');
    expect(source).toContain('import BaseSegmented, { type SegmentedOption } from "../ui/BaseSegmented.vue"');
    expect(source).toContain('const APP_NAME = "Locus"');
    expect(source).toContain('const ORGANIZATION = "FarLocus"');
    expect(source).toContain('const CONTACT_EMAIL = "open@farlocus.com"');
    expect(source).toContain("await appUpdateStore.ensureCurrentVersion();");
    expect(source).toContain("Unity Dev Agent");
    expect(source).toContain('<dd class="about-value about-version-value">');
    expect(source).toContain("{{ currentVersionChannelLabel }}");
    expect(source).toContain('t("settings.about.versionSource")');
    expect(source).toContain("{{ appUpdateStore.sourceLabel }}");
    expect(source).toContain('t("settings.about.updateChannel")');
    expect(source).toContain(':model-value="appUpdateStore.updateChannel"');
    expect(source).toContain('t("settings.about.lastChecked")');
    expect(source).toContain('t("settings.about.checkUpdates")');
    expect(source).toContain("await appUpdateStore.checkForUpdates();");
    expect(source).toContain('t("settings.about.organization")');
    expect(source).toContain('t("settings.about.contact")');
    expect(appUpdateStore).toContain("export const useAppUpdateStore = defineStore(\"appUpdate\", () => {");
    expect(appUpdateStore).toContain("const lastCheckedAt = ref<number | null>(loadLastCheckedAt());");
    expect(appUpdateStore).toContain("async function checkForUpdates(options?: { silent?: boolean }): Promise<AppUpdateInfo | null> {");
  });

  it("defines localized about labels", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"settings.tab.about": "关于"');
    expect(zh).toContain('"settings.about.organization": "开发组织"');
    expect(zh).toContain('"settings.about.contact": "联络邮箱"');
    expect(zh).toContain('"settings.about.versionSource": "版本来源"');
    expect(zh).toContain('"settings.about.versionSourceLocal": "本地服务器 ({0})"');
    expect(zh).toContain('"settings.about.versionSourceRemote": "{0}"');
    expect(zh).toContain('"settings.about.updateChannel": "更新通道"');
    expect(zh).toContain('"settings.about.lastChecked": "上次检查"');
    expect(zh).toContain('"settings.about.checkUpdates": "检查更新"');
    expect(en).toContain('"settings.tab.about": "About"');
    expect(en).toContain('"settings.about.organization": "Organization"');
    expect(en).toContain('"settings.about.contact": "Contact Email"');
    expect(en).toContain('"settings.about.versionSource": "Version source"');
    expect(en).toContain('"settings.about.versionSourceLocal": "Local server ({0})"');
    expect(en).toContain('"settings.about.versionSourceRemote": "{0}"');
    expect(en).toContain('"settings.about.updateChannel": "Update channel"');
    expect(en).toContain('"settings.about.lastChecked": "Last checked"');
    expect(en).toContain('"settings.about.checkUpdates": "Check for updates"');
  });
});
