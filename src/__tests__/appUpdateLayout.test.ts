import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("app update prompt", () => {
  it("checks the update manifest during startup and renders a root modal linked to the docs current-version page", () => {
    const app = read("src/App.vue");
    const modal = read("src/components/AppUpdateModal.vue");
    const store = read("src/stores/appUpdate.ts");
    const service = read("src/services/appUpdate.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(app).toContain('import AppUpdateModal from "./components/AppUpdateModal.vue"');
    expect(app).toContain('import { useAppUpdateStore } from "./stores/appUpdate"');
    expect(app).toContain("void appUpdateStore.checkForUpdates({ silent: true });");
    expect(app).toContain("<AppUpdateModal");
    expect(app).toContain('@view="openAppUpdateChangelog"');
    expect(app).toContain("await openUrl(updateInfo.changelogUrl);");
    expect(app).toContain('t("app.update.openFailed", err.message)');
    expect(modal).toContain('t("app.update.currentVersion")');
    expect(modal).toContain('Locus v${props.info.latestVersion} (${props.info.releasedAt})');
    expect(modal).toContain('Locus v${props.info.currentVersion}');
    expect(modal).toContain("font-size: 18px;");
    expect(modal).toContain("color: var(--text-color);");
    expect(modal).toContain("app-update-version-text-current");
    expect(modal).toContain("app-update-version-text-latest");
    expect(modal).toContain("grid-template-columns: minmax(0, 1fr) minmax(0, 0.92fr);");
    expect(modal).toContain("font-variant-numeric: tabular-nums;");
    expect(modal).toContain("font-weight: 700;");
    expect(modal).toContain('t("app.update.updateVersion")');
    expect(store).toContain("const LAST_CHECKED_AT_STORAGE_KEY = \"locus-app-update-last-checked-at\";");
    expect(store).toContain("const updateInfo = computed<AppUpdateInfo | null>(() => {");
    expect(store).toContain("const sourceLabel = computed(() => {");
    expect(store).toContain("notificationStore.addNotice(\"success\", t(\"app.update.upToDateNotice\")");
    expect(service).toContain('const DOCS_BASE_URL = "https://unity.farlocus.com";');
    expect(service).toContain("sourceBaseUrl = DOCS_BASE_URL");
    expect(service).toContain('"fetch_app_update_manifest"');
    expect(zh).toContain('"settings.about.versionSourceLocal": "本地服务器 ({0})"');
    expect(en).toContain('"settings.about.versionSourceLocal": "Local server ({0})"');
    expect(zh).toContain('"app.update.updateVersion": "更新版本"');
    expect(en).toContain('"app.update.updateVersion": "Update version"');
    expect(zh).toContain('"app.update.openFailed": "打开更新页面失败: {0}"');
    expect(en).toContain('"app.update.openFailed": "Failed to open the update page: {0}"');
  });
});
