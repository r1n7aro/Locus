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

    expect(source).toContain('import packageJson from "../../../package.json"');
    expect(source).toContain('const APP_NAME = "Locus"');
    expect(source).toContain('const ORGANIZATION = "FarLocus"');
    expect(source).toContain('const CONTACT_EMAIL = "open@farlocus.com"');
    expect(source).toContain('const APP_VERSION = packageJson.version');
    expect(source).toContain("Unity Dev Agent");
    expect(source).toContain('<dd class="about-value mono">v{{ APP_VERSION }}</dd>');
    expect(source).toContain('t("settings.about.organization")');
    expect(source).toContain('t("settings.about.contact")');
  });

  it("defines localized about labels", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"settings.tab.about": "关于"');
    expect(zh).toContain('"settings.about.organization": "开发组织"');
    expect(zh).toContain('"settings.about.contact": "联络邮箱"');
    expect(en).toContain('"settings.tab.about": "About"');
    expect(en).toContain('"settings.about.organization": "Organization"');
    expect(en).toContain('"settings.about.contact": "Contact Email"');
  });
});
