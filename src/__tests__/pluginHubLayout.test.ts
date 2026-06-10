import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("plugin hub layout", () => {
  it("uses a left plugin list and a right detail pane", () => {
    const source = read("src/components/PluginView.vue");

    expect(source).toContain("pluginListMode");
    expect(source).toContain("const pluginListMode = ref<PluginListMode>(\"registry\")");
    expect(source).toContain("togglePluginListMode");
    expect(source).not.toContain("BaseSegmented");
    expect(source).not.toContain("plugin-list-mode");
    expect(source).toContain("plugin-list-pane");
    expect(source).toContain("plugin-detail-pane");
    expect(source).toContain("plugin-detail-view");
    expect(source).toContain("@click=\"selectRegistryPlugin(plugin)\"");
    expect(source).toContain("MarkdownRenderer");
    expect(source).toContain("pluginRegistryFetchDescription");
    expect(source).toContain("descriptionSource");
    expect(source).not.toContain("v-if=\"registryDetailOpen");
    expect(source).not.toContain("plugin-hub-list-meta");
    expect(source).not.toContain("plugin-hub-selected-detail");
  });

  it("keeps registry plugin metadata above the detail markdown", () => {
    const source = read("src/components/PluginView.vue");
    const headerStart = source.indexOf("<header class=\"plugin-detail-hero\">");
    const metaStart = source.indexOf("<section class=\"plugin-detail-meta\">", headerStart);
    const bodyStart = source.indexOf("<section class=\"plugin-detail-scroll\">", metaStart);
    const header = source.slice(headerStart, bodyStart);
    const bodyEnd = source.indexOf("</section>", bodyStart);
    const body = source.slice(bodyStart, bodyEnd);

    expect(headerStart).toBeGreaterThan(-1);
    expect(metaStart).toBeGreaterThan(headerStart);
    expect(bodyStart).toBeGreaterThan(metaStart);
    expect(header).toContain("plugin-detail-meta-item");
    expect(header).toContain("plugin.hub.tags");
    expect(body).toContain("MarkdownRenderer");
    expect(body).not.toContain("plugin-detail-grid");
    expect(body).not.toContain("plugin-detail-field");
  });

  it("shows a GitHub Star action for signed-in registry plugins with a GitHub repo", () => {
    const source = read("src/components/PluginView.vue");
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");
    const lib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");
    const registryDetailStart = source.indexOf("<div v-if=\"pluginListMode === 'registry' && selectedRegistryDisplay\"");
    const installedDetailStart = source.indexOf("<div v-else-if=\"pluginListMode === 'installed' && selectedInstalledPlugin\"", registryDetailStart);
    const registryDetail = source.slice(registryDetailStart, installedDetailStart);

    expect(source).toContain("pluginGithubRepoStarStatus");
    expect(source).toContain("pluginGithubRepoSetStarred");
    expect(source).toContain("const selectedRegistryGithubRepo = computed");
    expect(source).toContain("function registryGithubRepoKey");
    expect(source).toContain("function toggleSelectedRegistryGithubStar");
    expect(source).toContain("function syncGithubStarStatus");
    expect(source).toContain("function updateRegistryStarCountByDelta");
    expect(source).toContain("starCountDelta");
    expect(registryDetail).toContain("githubAuthStatus.authenticated && selectedRegistryGithubRepo");
    expect(registryDetail).toContain("class=\"plugin-star-button\"");
    expect(registryDetail).toContain("selectedRegistryStarStatus?.starred");
    expect(registryDetail).toContain("@click=\"toggleSelectedRegistryGithubStar\"");
    expect(source).toContain(".plugin-detail-actions :deep(.plugin-star-button.is-starred)");
    expect(service).toContain("export interface PluginGithubRepoStarStatus");
    expect(service).toContain('"plugin_github_repo_star_status"');
    expect(service).toContain('"plugin_github_repo_set_starred"');
    expect(backend).toContain("pub struct PluginGithubRepoStarStatus");
    expect(backend).toContain("fn github_user_star_api_url");
    expect(backend).toContain("fetch_github_repo_star_status_with_token");
    expect(backend).toContain("set_github_repo_star_status_with_token");
    expect(backend).not.toContain("fetch_github_repository_with_token");
    expect(backend).not.toContain("struct GithubRepository");
    expect(backend).toContain("pub async fn plugin_github_repo_star_status");
    expect(backend).toContain("pub async fn plugin_github_repo_set_starred");
    expect(lib).toContain("commands::plugin_github_repo_star_status");
    expect(lib).toContain("commands::plugin_github_repo_set_starred");
    expect(zh).toContain('"plugin.hub.star": "Star"');
    expect(zh).toContain('"plugin.hub.unstar": "取消 Star"');
    expect(zh).toContain('"plugin.notice.unstarred": "已取消 Star：{0}"');
    expect(en).toContain('"plugin.hub.star": "Star"');
    expect(en).toContain('"plugin.hub.unstar": "Unstar"');
    expect(en).toContain('"plugin.notice.unstarred": "Unstarred plugin: {0}"');
  });

  it("edits registry sources as table rows in the config window", () => {
    const source = read("src/components/PluginView.vue");
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");
    const lib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");
    const titlebarStart = source.indexOf("class=\"plugin-list-titlebar\"");
    const titlebarEnd = source.indexOf("</header>", titlebarStart);
    const titlebar = source.slice(titlebarStart, titlebarEnd);
    const toolsStart = source.indexOf("<div class=\"plugin-list-tools\">");
    const toolsEnd = source.indexOf("<div class=\"plugin-list-content\">", toolsStart);
    const tools = source.slice(toolsStart, toolsEnd);
    const configStart = source.indexOf("class=\"plugin-hub-config-modal\"");
    const configEnd = source.indexOf("</section>", configStart);
    const config = source.slice(configStart, configEnd);

    expect(titlebarStart).toBeGreaterThan(-1);
    expect(titlebar).toContain("toggleRegistryConfig");
    expect(titlebar).toContain("Settings2");
    expect(titlebar).toContain("openGithubOAuth");
    expect(titlebar).toContain("plugin-github-login-button");
    expect(titlebar).toContain("githubAuthStatus.authenticated");
    expect(titlebar).toContain("KeyRound");
    expect(titlebar).toContain("UserCheck");
    expect(toolsStart).toBeGreaterThan(-1);
    expect(tools).toContain("plugin-hub-search");
    expect(tools).toContain("pluginSearch");
    expect(tools).toContain("togglePluginListMode");
    expect(tools).toContain("pluginListModeToggleIcon");
    expect(tools).not.toContain("RefreshCw");
    expect(tools).not.toContain("plugin-registry-source-select");
    expect(source).not.toContain("v-if=\"pluginListMode === 'registry'\" class=\"plugin-list-tools\"");
    expect(configStart).toBeGreaterThan(-1);
    expect(config).toContain("plugin-hub-registry-table");
    expect(config).toContain("registrySourceDrafts");
    expect(config).toContain("source.repoInput");
    expect(config).toContain("plugin.hub.registryAddressPlaceholder");
    expect(config).toContain("saveRegistrySources");
    expect(config).toContain("removeRegistrySourceDraft(index)");
    expect(config).not.toContain("plugin-hub-github-auth");
    expect(config).not.toContain("githubTokenDraft");
    expect(config).not.toContain("saveGithubAuthToken");
    expect(source).toContain("refreshGithubAuthStatus");
    expect(service).toContain("export interface PluginGithubAuthStatus");
    expect(service).toContain("export interface PluginGithubOAuthStartResult");
    expect(service).toContain('"plugin_github_oauth_start"');
    expect(service).toContain('"plugin_github_oauth_poll"');
    expect(source).toContain("githubOAuthOpen");
    expect(source).toContain("pluginGithubOAuthStart");
    expect(source).toContain("pluginGithubOAuthPoll");
    expect(source).toContain("plugin-github-oauth-modal");
    expect(backend).toContain("pub async fn plugin_github_auth_save_token");
    expect(backend).toContain("pub async fn plugin_github_oauth_start");
    expect(backend).toContain("pub async fn plugin_github_oauth_poll");
    expect(backend).toContain("resolve_github_cli");
    expect(backend).toContain("run_github_cli_login");
    expect(backend).not.toContain("LOCUS_GITHUB_OAUTH_CLIENT_ID");
    expect(lib).toContain("commands::plugin_github_auth_save_token");
    expect(lib).toContain("commands::plugin_github_oauth_start");
    expect(lib).toContain("commands::plugin_github_oauth_poll");
    expect(zh).toContain('"plugin.hub.githubAuth": "GitHub 登录"');
    expect(zh).toContain('"plugin.hub.githubLogin": "登录 GitHub"');
    expect(zh).toContain('"plugin.hub.githubCliInstruction"');
    expect(zh).not.toContain("GitHub 访问令牌");
    expect(en).toContain('"plugin.hub.githubAuth": "GitHub Login"');
    expect(en).toContain('"plugin.hub.githubLogin": "Sign In to GitHub"');
    expect(en).toContain('"plugin.hub.githubCliInstruction"');
    expect(en).not.toContain("GitHub access token");
    expect(source).not.toContain("plugin-registry-source-select");
    expect(source).not.toContain("setSelectedRegistrySource");
    expect(source).not.toContain("selectedRegistrySourceId");
    expect(source).toContain("let shouldPersist = false");
    expect(source).toContain("shouldPersist = rawSources !== JSON.stringify(nextSources)");
    expect(source).toContain("if (shouldPersist) persistRegistrySourceSettings()");
  });

  it("adds a compact plugin help dialog from the plugin titlebar", () => {
    const source = read("src/components/PluginView.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");
    const titlebarStart = source.indexOf("class=\"plugin-list-titlebar\"");
    const titlebarEnd = source.indexOf("</header>", titlebarStart);
    const titlebar = source.slice(titlebarStart, titlebarEnd);
    const helpStart = source.indexOf("class=\"plugin-hub-config-modal plugin-help-modal\"");
    const helpEnd = source.indexOf("v-if=\"registryConfigOpen\"", helpStart);
    const help = source.slice(helpStart, helpEnd);

    expect(source).toContain("HelpCircle");
    expect(source).toContain("const pluginHelpOpen = ref(false)");
    expect(source).toContain("function openPluginHelp");
    expect(source).toContain("function closePluginHelp");
    expect(titlebar).toContain("plugin.help.label");
    expect(titlebar).toContain("@click=\"openPluginHelp\"");
    expect(titlebar).toContain(":icon=\"HelpCircle\"");
    expect(helpStart).toBeGreaterThan(-1);
    expect(help).toContain("plugin.help.commandTitle");
    expect(help).toContain("plugin.help.skillTitle");
    expect(help).toContain("plugin.help.publishTitle");
    expect(source).toContain(".plugin-help-section");
    expect(source).not.toContain("plugin-help-card");
    expect(zh).toContain('"plugin.help.commandTitle": "/plugin"');
    expect(zh).toContain('"plugin.help.skillTitle": "Skill"');
    expect(zh).toContain('"plugin.help.publishTitle": "如何发布自己的插件？"');
    expect(zh).toContain("要求 Agent 创建插件仓库并向注册表发起 Pull Request");
    expect(en).toContain('"plugin.help.commandTitle": "/plugin"');
    expect(en).toContain('"plugin.help.skillTitle": "Skill"');
    expect(en).toContain('"plugin.help.publishTitle": "How do I publish my own plugin?"');
    expect(en).toContain("ask the agent to create the plugin repository and open a pull request to the registry");
  });

  it("uses scope tags on plugin list entries instead of install scope controls", () => {
    const source = read("src/components/PluginView.vue");
    const registryListStart = source.indexOf("<template v-if=\"pluginListMode === 'registry'\">");
    const installedListStart = source.indexOf("<template v-else>", registryListStart);
    const detailStart = source.indexOf("<main class=\"plugin-pane plugin-detail-pane\">");
    const registryList = source.slice(registryListStart, installedListStart);
    const installedList = source.slice(installedListStart, detailStart);
    const detail = source.slice(detailStart);

    expect(source).not.toContain("function registryScopeTag");
    expect(source).toContain("function pluginScopeTag");
    expect(source).toContain("function installedRegistryScopeTag");
    expect(source).toContain("filteredInstalledPlugins");
    expect(registryList).toContain("installedRegistryScopeTag(plugin)");
    expect(registryList).toContain("v-if=\"installedRegistryScopeTag(plugin)\"");
    expect(registryList).not.toContain("registryScopeTag(plugin)");
    expect(registryList).not.toContain("plugin.hub.installed");
    expect(installedList).toContain("pluginScopeTag(plugin.scope)");
    expect(installedList).toContain("filteredInstalledPlugins");
    expect(source).toContain("plugin-scope-tag");
    expect(source).not.toContain("pluginInstallFromPath");
    expect(detail).not.toContain("plugin-install-scope");
    expect(detail).not.toContain("<LucideIcon :icon=\"Download\"");
  });

  it("places compact registry actions and stats inside registry list rows", () => {
    const source = read("src/components/PluginView.vue");
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");
    const installStart = source.indexOf("async function installRegistryPluginWithScopes");
    const installEnd = source.indexOf(
      "async function installRegistryPluginWithScope(",
      installStart + "async function installRegistryPluginWithScopes".length,
    );
    const installBlock = source.slice(installStart, installEnd);
    const registryListStart = source.indexOf("<template v-if=\"pluginListMode === 'registry'\">");
    const installedListStart = source.indexOf("<template v-else>", registryListStart);
    const registryList = source.slice(registryListStart, installedListStart);
    const stylesStart = source.indexOf(".plugin-registry-list-item");
    const stylesEnd = source.indexOf(".plugin-list-main", stylesStart);
    const listStyles = source.slice(stylesStart, stylesEnd);

    expect(source).toContain("const REGISTRY_INSTALL_SCOPE");
    expect(source).toContain("const registryInstallKey");
    expect(source).toContain("function installRegistryPlugin");
    expect(source).toContain("function uninstallRegistryPlugin");
    expect(source).toContain("pluginInstallFromRegistry");
    expect(installStart).toBeGreaterThan(-1);
    expect(installEnd).toBeGreaterThan(installStart);
    expect(installBlock).toContain("download: detail.download ?? {}");
    expect(installBlock).toContain("downloadSource: detail.downloadSource");
    expect(installBlock).toContain(": {},");
    expect(installBlock).not.toContain("download: detail.download ?? null");
    expect(installBlock).not.toContain(": null,");
    expect(source).toContain("function registryStatValue");
    expect(source).toContain("function registryStatIconNode");
    expect(registryList).toContain("class=\"plugin-list-side\"");
    expect(registryList).toContain("class=\"plugin-list-stats\"");
    expect(registryList).toContain("v-for=\"stat in plugin.stats\"");
    expect(registryList).toContain("plugin-registry-action-button is-install-action");
    expect(registryList).toContain("plugin-registry-action-button is-update-action");
    expect(registryList).toContain("plugin-registry-action-button is-uninstall-action");
    expect(registryList).toContain("@click.stop=\"installRegistryPlugin(plugin)\"");
    expect(registryList).toContain("@click.stop=\"updateRegistryPlugin(plugin)\"");
    expect(registryList).toContain("@click.stop=\"uninstallRegistryPlugin(plugin)\"");
    expect(registryList).not.toContain("variant=\"primary\"");
    expect(listStyles).toContain(":deep(.plugin-registry-action-button)");
    expect(listStyles).toContain(":deep(.plugin-registry-action-button.is-install-action)");
    expect(listStyles).toContain(":deep(.plugin-registry-action-button.is-update-action)");
    expect(listStyles).toContain("var(--accent-soft)");
    expect(listStyles).toContain("height: 20px");
    expect(listStyles).toContain("justify-content: space-between");
    expect(service).toContain("export interface PluginRegistryStat");
    expect(service).toContain("stats?: PluginRegistryStat[]");
    expect(backend).toContain("pub struct PluginRegistryStat");
    expect(backend).toContain("pub stats: Vec<PluginRegistryStat>");
    expect(backend).toContain("normalize_registry_stat");
  });

  it("supports right-click list actions and direct plugin import", () => {
    const source = read("src/components/PluginView.vue");
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");
    const lib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain('import { open } from "@tauri-apps/plugin-dialog"');
    expect(source).toContain("BaseContextMenu");
    expect(source).toContain("@contextmenu=\"openPluginListContextMenu\"");
    expect(source).toContain("plugin-list-context-menu-item");
    expect(source).toContain(":global(.plugin-list-context-menu .plugin-list-context-menu-item)");
    expect(source).toContain("refreshPluginListFromMenu");
    expect(source).toContain("openDirectImport('local')");
    expect(source).toContain("openDirectImport('link')");
    expect(source).toContain("pluginInstallFromSource");
    expect(source).toContain("chooseDirectImportFile");
    expect(source).toContain("chooseDirectImportFolder");
    expect(service).toContain("export interface PluginDownloadSource");
    expect(service).toContain('ipcInvoke<InstalledPluginSummary>("plugin_install_from_source"');
    expect(backend).toContain("pub struct PluginDownloadSource");
    expect(backend).toContain("pub async fn plugin_install_from_source");
    expect(lib).toContain("commands::plugin_install_from_source");
    expect(zh).toContain('"plugin.import.local": "导入本地插件"');
    expect(en).toContain('"plugin.import.link": "Import From Link"');
  });

  it("uses plugin enabled state as the installed plugin switch", () => {
    const source = read("src/components/PluginView.vue");
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");
    const pluginModel = read("src-tauri/src/plugin.rs");
    const lib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain("BaseSwitch");
    expect(source).toContain("pluginSetEnabled");
    expect(source).toContain("const pluginEnableKey = ref(\"\")");
    expect(source).toContain("function setPluginEnabledState");
    expect(source).toContain(":model-value=\"plugin.enabled\"");
    expect(source).toContain(":model-value=\"selectedInstalledPlugin.enabled\"");
    expect(source).toContain("plugin-state-tag");
    expect(source).toContain("plugin.notice.enabled");
    expect(source).toContain("plugin.notice.disabled");
    expect(service).toContain("enabled: boolean");
    expect(service).toContain('ipcInvoke<InstalledPluginSummary>("plugin_set_enabled"');
    expect(backend).toContain("pub async fn plugin_set_enabled");
    expect(backend).toContain("set_plugin_enabled_sync");
    expect(lib).toContain("commands::plugin_set_enabled");
    expect(pluginModel).toContain("PLUGIN_STATE_FILE_NAME");
    expect(pluginModel).toContain("plugin_enabled_for_scope");
    expect(pluginModel).toContain("component_sources_for_kind");
    expect(zh).toContain('"plugin.detail.status": "状态"');
    expect(zh).toContain('"plugin.hub.disable": "停用"');
    expect(en).toContain('"plugin.notice.disabled": "Disabled plugin: {0}"');
  });

  it("keeps plugin list descriptions single-line", () => {
    const source = read("src/components/PluginView.vue");
    const summaryStyleStart = source.indexOf(".plugin-list-summary");
    const summaryStyleEnd = source.indexOf(".plugin-list-meta", summaryStyleStart);
    const summaryStyle = source.slice(summaryStyleStart, summaryStyleEnd);

    expect(summaryStyle).toContain("white-space: nowrap");
    expect(summaryStyle).toContain("text-overflow: ellipsis");
    expect(summaryStyle).not.toContain("-webkit-line-clamp");
  });

  it("supports localized plugin summaries and detail descriptions", () => {
    const source = read("src/components/PluginView.vue");
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");

    expect(source).toContain('import { locale, t } from "../i18n"');
    expect(source).toContain("function localizedRegistrySummary");
    expect(source).toContain("function localizedRegistryDescription");
    expect(source).toContain("function localizedRegistryDescriptionSource");
    expect(source).toContain("registryDescriptionCacheKey");
    expect(source).toContain("${plugin.registryKey}:${locale.value}");
    expect(source).toContain("plugin.descriptionSourceI18n");
    expect(source).toContain("plugin.summaryI18n");
    expect(source).toContain("localizedRegistrySummary(plugin) || plugin.id");
    expect(source).toContain("localizedRegistrySummary(selectedRegistryDisplay) || selectedRegistryDisplay.id");
    expect(service).toContain("export type PluginRegistryLocalizedText");
    expect(service).toContain("summaryI18n?: PluginRegistryLocalizedText");
    expect(service).toContain("descriptionI18n?: PluginRegistryLocalizedText");
    expect(service).toContain("descriptionSourceI18n?: PluginRegistryLocalizedDescriptionSource");
    expect(backend).toContain("pub summary_i18n: BTreeMap<String, String>");
    expect(backend).toContain("pub description_i18n: BTreeMap<String, String>");
    expect(backend).toContain("pub description_source_i18n: BTreeMap<String, PluginRegistryDescriptionSource>");
    expect(backend).toContain("normalize_registry_localized_text");
    expect(backend).toContain("normalize_registry_localized_description_sources");
  });

  it("renders installed list entries with the same metadata shape as registry rows", () => {
    const source = read("src/components/PluginView.vue");
    const registryListStart = source.indexOf("<template v-if=\"pluginListMode === 'registry'\">");
    const installedListStart = source.indexOf("<template v-else>", registryListStart);
    const detailStart = source.indexOf("<main class=\"plugin-pane plugin-detail-pane\">");
    const installedList = source.slice(installedListStart, detailStart);

    expect(source).toContain("function installedRegistryDisplay");
    expect(source).toContain("function installedListDisplayName");
    expect(source).toContain("function installedListSummary");
    expect(source).toContain("function installedListAuthor");
    expect(source).toContain("function installedListIconNode");
    expect(installedList).toContain("plugin-registry-list-item plugin-installed-list-item");
    expect(installedList).toContain("class=\"plugin-list-select-button\"");
    expect(installedList).toContain("class=\"plugin-hub-icon-frame plugin-list-icon\"");
    expect(installedList).toContain("installedListDisplayName(plugin)");
    expect(installedList).toContain("installedListVersion(plugin)");
    expect(installedList).toContain("installedListSummary(plugin)");
    expect(installedList).toContain("installedListAuthor(plugin)");
    expect(installedList).toContain("class=\"plugin-list-side\"");
    expect(installedList).toContain("installedUpdateCandidate(plugin)");
    expect(installedList).toContain("@click.stop=\"updateInstalledPlugin(plugin)\"");
    expect(installedList).toContain("plugin-registry-action-button is-uninstall-action");
    expect(installedList).toContain("@click.stop=\"uninstallPlugin(plugin)\"");
    expect(installedList).not.toContain("plugin-list-id");
    expect(installedList).not.toContain("formatComponentSummary(plugin)");
    expect(installedList).not.toContain("plugin-local-icon");
  });

  it("separates installed and uninstalled registry entries outside list rows", () => {
    const source = read("src/components/PluginView.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");
    const registryListStart = source.indexOf("<template v-if=\"pluginListMode === 'registry'\">");
    const installedListStart = source.indexOf("<template v-else>", registryListStart);
    const registryList = source.slice(registryListStart, installedListStart);

    expect(source).toContain("function shouldShowRegistryInstallDivider");
    expect(source).toContain("function registryPluginIsInstalled");
    expect(source).not.toContain("installedPluginIds");
    expect(registryList).toContain("v-for=\"(plugin, index) in visibleRegistrySummaries\"");
    expect(registryList).toContain("shouldShowRegistryInstallDivider(plugin, index)");
    expect(registryList).toContain("plugin-registry-section-divider");
    expect(registryList).toContain("plugin.hub.uninstalled");
    expect(registryList).not.toContain("plugin.hub.installed");
    expect(source).toContain(".plugin-registry-section-divider::before");
    expect(zh).toContain("\"plugin.hub.uninstalled\": \"未安装\"");
    expect(en).toContain("\"plugin.hub.uninstalled\": \"Not Installed\"");
  });

  it("shows registry source only for duplicate registry plugin names", () => {
    const source = read("src/components/PluginView.vue");
    const registryListStart = source.indexOf("<template v-if=\"pluginListMode === 'registry'\">");
    const installedListStart = source.indexOf("<template v-else>", registryListStart);
    const registryList = source.slice(registryListStart, installedListStart);
    const registryDetailStart = source.indexOf("<div v-if=\"pluginListMode === 'registry' && selectedRegistryDisplay\"");
    const installedDetailStart = source.indexOf("<div v-else-if=\"pluginListMode === 'installed' && selectedInstalledPlugin\"", registryDetailStart);
    const registryDetail = source.slice(registryDetailStart, installedDetailStart);

    expect(source).toContain("const registryNameSourceCounts = computed");
    expect(source).toContain("function shouldShowRegistrySource");
    expect(registryList).toContain("v-if=\"shouldShowRegistrySource(plugin)\"");
    expect(registryList).toContain("plugin.registrySourceLabel");
    expect(registryDetail).toContain("v-if=\"shouldShowRegistrySource(selectedRegistryDisplay)\"");
    expect(registryDetail).toContain("selectedRegistryDisplay.registrySourceLabel");
  });

  it("streams registry list pages instead of rendering every loaded entry", () => {
    const source = read("src/components/PluginView.vue");
    const registryListStart = source.indexOf("<template v-if=\"pluginListMode === 'registry'\">");
    const installedListStart = source.indexOf("<template v-else>", registryListStart);
    const registryList = source.slice(registryListStart, installedListStart);

    expect(source).toContain("const REGISTRY_VISIBLE_PAGE_SIZE");
    expect(source).toContain("const registryVisibleCount");
    expect(source).toContain("const visibleRegistrySummaries");
    expect(source).toContain("hasMoreRegistryListItems");
    expect(source).toContain("function handleRegistryListScroll");
    expect(source).toContain("async function loadNextRegistryPage");
    expect(source).not.toContain("loadAllRegistryBuckets");
    expect(registryList).toContain("visibleRegistrySummaries");
    expect(registryList).toContain("@scroll.passive=\"handleRegistryListScroll\"");
    expect(registryList).toContain("@click=\"loadNextRegistryPage()\"");
    expect(registryList).not.toContain("v-for=\"plugin in filteredRegistrySummaries\"");
  });

  it("uses the generated registry search index when filtering registry plugins", () => {
    const source = read("src/components/PluginView.vue");
    const watcherStart = source.indexOf("watch(pluginSearch");
    const watcherEnd = source.indexOf("watch(locale", watcherStart);
    const watcherBlock = source.slice(watcherStart, watcherEnd);

    expect(source).toContain("pluginRegistryFetchSearchIndex");
    expect(source).toContain("const registrySearchIndexLoaded");
    expect(source).toContain("async function loadRegistrySearchIndex");
    expect(source).toContain("async function loadRegistrySearchIndexes");
    expect(watcherBlock).toContain("void loadRegistrySearchIndexes()");
  });

  it("keeps installed registry plugins sorted before uninstalled registry plugins", () => {
    const source = read("src/components/PluginView.vue");
    const sortedStart = source.indexOf("const sortedRegistrySummaries = computed");
    const sortedEnd = source.indexOf("const filteredRegistrySummaries", sortedStart);
    const sortedBlock = source.slice(sortedStart, sortedEnd);

    expect(source).toContain("const installedPluginById = computed");
    expect(source).toContain("async function loadInstalledRegistryEntries");
    expect(source).toContain("registryEntryToSummary");
    expect(source).toContain("registryItemKey");
    expect(source).toContain("withRegistrySource");
    expect(source).toContain("registryBaseUrls");
    expect(source).toContain("registryManifests");
    expect(sortedBlock).toContain("installedPluginById.value.has(right.id)");
    expect(sortedBlock).toContain("installedPluginById.value.has(left.id)");
  });

  it("loads registry shards before background installed detail enrichment", () => {
    const source = read("src/components/PluginView.vue");
    const refreshStart = source.indexOf("async function refreshRegistry(");
    const refreshEnd = source.indexOf("async function ensureRegistryDetail", refreshStart);
    const refreshBlock = source.slice(refreshStart, refreshEnd);
    const refreshAllStart = source.indexOf("async function refreshAll()");
    const refreshAllEnd = source.indexOf("function pluginScopeLabel", refreshAllStart);
    const refreshAllBlock = source.slice(refreshAllStart, refreshAllEnd);
    const selectStart = source.indexOf("async function selectRegistryPlugin");
    const selectEnd = source.indexOf("function registryCompatibilityLabel", selectStart);
    const selectBlock = source.slice(selectStart, selectEnd);

    expect(refreshBlock).toContain("await loadMoreRegistryBuckets(REGISTRY_BUCKET_BATCH_SIZE, { cacheMode: options.cacheMode }, generation)");
    expect(refreshBlock).toContain("void loadInstalledRegistryEntries()");
    expect(refreshBlock.indexOf("await loadMoreRegistryBuckets(REGISTRY_BUCKET_BATCH_SIZE, { cacheMode: options.cacheMode }, generation)")).toBeLessThan(
      refreshBlock.indexOf("void loadInstalledRegistryEntries()"),
    );
    expect(refreshBlock).not.toContain("await loadInstalledRegistryEntries()");
    expect(refreshAllBlock).toContain("void loadInstalledRegistryEntries()");
    expect(refreshAllBlock).not.toContain("await loadInstalledRegistryEntries()");
    expect(selectBlock).toContain("void ensureRegistryDescription(detail)");
    expect(selectBlock).not.toContain("await ensureRegistryDescription(detail)");
    expect(source).toContain("const registryInstalledEntriesLoading");
  });

  it("renders cached registry data before silent network refresh on startup", () => {
    const source = read("src/components/PluginView.vue");
    const service = read("src/services/plugin.ts");
    const backend = read("src-tauri/src/commands/plugin.rs");

    expect(service).toContain('export type PluginRegistryCacheMode = "default" | "cachePreferred" | "networkPreferred"');
    expect(service).toContain("cacheMode: cacheMode ?? null");
    expect(service).toContain("cacheMode: options.cacheMode ?? null");
    expect(source).toContain('await refreshRegistry({ cacheMode: "cachePreferred", silent: true })');
    expect(source).toContain('cacheMode: "networkPreferred"');
    expect(source).toContain("preserveExisting: true");
    expect(source).toContain("pluginRegistryFetchManifest(pluginRegistrySourceBaseUrl(source), options.cacheMode)");
    expect(source).toContain("cacheMode: options.cacheMode");
    expect(backend).toContain("pub enum PluginRegistryCacheMode");
    expect(backend).toContain("CachePreferred");
    expect(backend).toContain("NetworkPreferred");
    expect(backend).toContain("cache_mode.unwrap_or_default()");
  });

  it("prompts plugin updates and reinstalls with the installed scope", () => {
    const source = read("src/components/PluginView.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");
    const registryListStart = source.indexOf("<template v-if=\"pluginListMode === 'registry'\">");
    const installedListStart = source.indexOf("<template v-else>", registryListStart);
    const detailStart = source.indexOf("<main class=\"plugin-pane plugin-detail-pane\">");
    const registryDetailStart = source.indexOf("<div v-if=\"pluginListMode === 'registry' && selectedRegistryDisplay\"");
    const installedDetailStart = source.indexOf("<div v-else-if=\"pluginListMode === 'installed' && selectedInstalledPlugin\"", registryDetailStart);
    const registryList = source.slice(registryListStart, installedListStart);
    const installedList = source.slice(installedListStart, detailStart);
    const registryDetail = source.slice(registryDetailStart, installedDetailStart);
    const installedDetail = source.slice(installedDetailStart);
    const registryDetailStarAction = registryDetail.indexOf("@click=\"toggleSelectedRegistryGithubStar\"");
    const registryDetailOpenRepoAction = registryDetail.indexOf("@click=\"openRegistryRepo(selectedRegistryDetail)\"");
    const registryDetailUpdateAction = registryDetail.indexOf("@click=\"updateRegistryPlugin(selectedRegistryDisplay)\"");
    const registryDetailInstallAction = registryDetail.indexOf("@click=\"installRegistryPlugin(selectedRegistryDisplay)\"");
    const registryDetailUninstallAction = registryDetail.indexOf("@click=\"uninstallRegistryPlugin(selectedRegistryDisplay)\"");

    expect(source).toContain("function parsePluginVersion");
    expect(source).toContain("function comparePluginVersions");
    expect(source).toContain("function registryVersionIsNewer");
    expect(source).toContain("function registryPluginUpdateAvailable");
    expect(source).toContain("function installedUpdateCandidate");
    expect(source).toContain("function installedRegistryPlugins");
    expect(source).toContain("function installRegistryPluginWithScopes");
    expect(source).toContain("function installRegistryPluginWithScope");
    expect(source).toContain("outdated.map((installed) => installed.scope)");
    expect(source).toContain('await installRegistryPluginWithScopes(plugin, outdated.map((installed) => installed.scope), "update")');
    expect(source).toContain('await installRegistryPluginWithScope(candidate, plugin.scope, "update")');
    expect(registryList).toContain("registryPluginUpdateAvailable(plugin)");
    expect(registryList).toContain("plugin-registry-action-button is-update-action");
    expect(registryList).toContain("@click.stop=\"updateRegistryPlugin(plugin)\"");
    expect(installedList).toContain("installedUpdateCandidate(plugin)");
    expect(installedList).toContain("@click.stop=\"updateInstalledPlugin(plugin)\"");
    expect(registryDetail).toContain("registryPluginUpdateAvailable(selectedRegistryDisplay)");
    expect(registryDetail).toContain("@click=\"updateRegistryPlugin(selectedRegistryDisplay)\"");
    expect(registryDetail).toContain("installedRegistryPlugin(selectedRegistryDisplay)");
    expect(registryDetail).toContain("@click=\"uninstallRegistryPlugin(selectedRegistryDisplay)\"");
    expect(registryDetail).toContain("plugin.hub.uninstall");
    expect(registryDetailStarAction).toBeGreaterThan(-1);
    expect(registryDetailOpenRepoAction).toBeGreaterThan(registryDetailStarAction);
    expect(registryDetailUpdateAction).toBeGreaterThan(registryDetailOpenRepoAction);
    expect(registryDetailInstallAction).toBeGreaterThan(registryDetailUpdateAction);
    expect(registryDetailUninstallAction).toBeGreaterThan(registryDetailInstallAction);
    expect(installedDetail).toContain("installedUpdateCandidate(selectedInstalledPlugin)");
    expect(installedDetail).toContain("@click=\"updateInstalledPlugin(selectedInstalledPlugin)\"");
    expect(installedDetail).toContain("plugin.hub.uninstall");
    expect(installedDetail).toContain("plugin.hub.latestAvailable");
    expect(source).toContain("plugin.notice.updated");
    expect(zh).toContain('"plugin.hub.update": "更新"');
    expect(zh).toContain('"plugin.notice.updated": "已更新插件：{0}"');
    expect(en).toContain('"plugin.hub.update": "Update"');
    expect(en).toContain('"plugin.notice.updated": "Updated plugin: {0}"');
  });

  it("keeps base button text on the shared UI font metrics", () => {
    const source = read("src/components/ui/BaseButton.vue");

    expect(source).toContain("font-family: var(--font-ui)");
    expect(source).toContain("line-height: 1");
  });
});
