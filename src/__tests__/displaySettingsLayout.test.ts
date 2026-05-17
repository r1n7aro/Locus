import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("display settings transcript alignment", () => {
  it("keeps main and Unity embed color styles separately configurable", () => {
    const theme = read("src/composables/useTheme.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const settingsState = read("src/composables/useSettingsState.ts");
    const app = read("src/App.vue");
    const html = read("index.html");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(theme).toContain('export type ThemeScope = "main" | "unityEmbed";');
    expect(theme).toContain('unityEmbed: "locus-unity-embed-theme-preference"');
    expect(theme).toContain('main: "dark"');
    expect(theme).toContain('unityEmbed: "dark"');
    expect(theme).toContain("unityEmbedPreference");
    expect(theme).toContain("setThemePreference(scope: ThemeScope, pref: ThemePreference)");

    expect(app).toContain('initTheme(isUnityEmbedWindow ? "unityEmbed" : "main")');
    expect(html).toContain("locus-unity-embed-theme-preference");
    expect(html).toContain("var fallback='dark';");

    expect(displayPanel).toContain("mainPreference");
    expect(displayPanel).toContain("unityEmbedPreference");
    expect(displayPanel).toContain("settings.display.themeMainWindow");
    expect(displayPanel).toContain("settings.display.themeUnityEmbedWindow");
    expect(displayPanel).toContain("setThemePreference('main', $event as ThemePreference)");
    expect(displayPanel).toContain("setThemePreference('unityEmbed', $event as ThemePreference)");
    expect(settingsState).toContain('setThemePreference("main", "dark");');
    expect(settingsState).toContain('setThemePreference("unityEmbed", "dark");');

    expect(zh).toContain('"settings.display.themeMainWindow": "主窗口"');
    expect(zh).toContain('"settings.display.themeUnityEmbedWindow": "Unity 嵌入窗口"');
    expect(en).toContain('"settings.display.themeMainWindow": "Main Window"');
    expect(en).toContain('"settings.display.themeUnityEmbedWindow": "Unity Embedded Window"');
  });

  it("adds a session user message right-align toggle that defaults to on", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("rightAlignUserMessages: boolean;");
    expect(displaySettings).toContain("rightAlignUserMessages: true,");

    expect(displayPanel).toContain(":model-value=\"display.rightAlignUserMessages\"");
    expect(displayPanel).toContain(":aria-label=\"t('settings.display.rightAlignUserMessages')\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('rightAlignUserMessages', $event)\"");
    expect(displayPanel).toContain("{{ t(\"settings.display.rightAlignUserMessages\") }}");

    expect(transcript).toContain("const { state: displaySettings } = useDisplaySettings();");
    expect(transcript).toContain("function shouldRightAlignUserMessageGroup(group: Pick<MessageGroup, \"role\">) {");
    expect(transcript).toContain("'user-align-right': shouldRightAlignUserMessageGroup(group),");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-role.is-session {");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-content.is-session {");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-item-stack.is-session {");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-plain-text {");

    expect(zh).toContain('"settings.display.rightAlignUserMessages": "会话窗口中将用户消息右对齐"');
    expect(en).toContain('"settings.display.rightAlignUserMessages": "Right-align user messages in the session view"');
  });

  it("adds a Git tree status icon merge toggle", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const stagingArea = read("src/components/collab/StagingArea.vue");
    const commitDetail = read("src/components/collab/CommitDetail.vue");
    const collabStyles = read("src/components/collab/collabPreview.css");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("mergeGitTreeStatusIcon: boolean;");
    expect(displaySettings).toContain("mergeGitTreeStatusIcon: true,");

    expect(displayPanel).toContain("settings.display.gitViewTitle");
    expect(displayPanel).toContain(":model-value=\"display.mergeGitTreeStatusIcon\"");
    expect(displayPanel).toContain(":aria-label=\"t('settings.display.mergeGitTreeStatusIcon')\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('mergeGitTreeStatusIcon', $event)\"");

    for (const component of [stagingArea, commitDetail]) {
      expect(component).toContain("const { state: displaySettings } = useDisplaySettings();");
      expect(component).toContain("displaySettings.mergeGitTreeStatusIcon");
      expect(component).toContain("fileTreeIconClasses(row.file)");
      expect(component).toContain("staging-tree-status-spacer");
    }

    expect(collabStyles).toContain(".staging-tree-file-icon.is-git-status-icon.status-modified");
    expect(collabStyles).toContain("color: var(--git-status-modified);");
    expect(collabStyles).toContain("color: var(--git-status-added);");
    expect(collabStyles).toContain("color: var(--git-status-deleted);");

    expect(zh).toContain('"settings.display.mergeGitTreeStatusIcon": "层级视图用彩色图标显示修改状态"');
    expect(en).toContain('"settings.display.mergeGitTreeStatusIcon": "Use colored icons for Git tree status"');
  });

  it("adds a Git terminal suggestion visibility toggle that defaults to visible", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const gitTerminal = read("src/components/GitTerminal.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("hideGitCommandSuggestions: boolean;");
    expect(displaySettings).toContain("hideGitCommandSuggestions: false,");

    expect(displayPanel).toContain(":model-value=\"display.hideGitCommandSuggestions\"");
    expect(displayPanel).toContain(":aria-label=\"t('settings.display.hideGitCommandSuggestions')\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('hideGitCommandSuggestions', $event)\"");
    expect(displayPanel).toContain("{{ t(\"settings.display.hideGitCommandSuggestions\") }}");

    expect(gitTerminal).toContain('import { useDisplaySettings } from "../composables/useDisplaySettings";');
    expect(gitTerminal).toContain("const { state: displaySettings } = useDisplaySettings();");
    expect(gitTerminal).toContain("!displaySettings.hideGitCommandSuggestions && lines.length === 0");

    expect(zh).toContain('"settings.display.hideGitCommandSuggestions": "隐藏 Git 候选项"');
    expect(en).toContain('"settings.display.hideGitCommandSuggestions": "Hide Git command suggestions"');
  });

  it("adds a subagent completion notification toggle that defaults to off", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const notifications = read("src/services/systemNotifications.ts");
    const bootstrap = read("src/composables/useAppBootstrap.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("notifyOnSubagentDone: boolean;");
    expect(displaySettings).toContain("notifyOnSubagentDone: false,");

    expect(displayPanel).toContain(":model-value=\"display.notifyOnSubagentDone\"");
    expect(displayPanel).toContain(":aria-label=\"t('settings.display.notifyOnSubagentDone')\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('notifyOnSubagentDone', $event)\"");
    expect(displayPanel).toContain("{{ t(\"settings.display.notifyOnSubagentDone\") }}");

    expect(notifications).toContain("context.isSubagent ? state.notifyOnSubagentDone : state.notifyOnChatDone");
    expect(notifications).toContain('context.isSubagent ? "notifications.subagentDoneTitle" : "notifications.chatDoneTitle"');
    expect(bootstrap).toContain("...(session?.parentSessionId ? { isSubagent: true } : {})");

    expect(zh).toContain('"settings.display.notifyOnSubagentDone": "Subagent 完成时通知"');
    expect(zh).toContain('"notifications.subagentDoneTitle": "Subagent 已完成"');
    expect(en).toContain('"settings.display.notifyOnSubagentDone": "Notify when a Sub-agent completes"');
    expect(en).toContain('"notifications.subagentDoneTitle": "Sub-agent complete"');
  });

  it("adds file diff review target settings that default to the current window", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const chatChangesPanel = read("src/components/ChatChangesPanel.vue");
    const chatView = read("src/components/ChatView.vue");
    const chatReviewWindow = read("src/components/ChatDiffReviewWindow.vue");
    const fileDiffViewer = read("src/components/diff/FileDiffViewer.vue");
    const collabView = read("src/components/CollabView.vue");
    const app = read("src/App.vue");
    const capability = read("src-tauri/capabilities/default.json");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain('export type DiffReviewTarget = "inline" | "window";');
    expect(displaySettings).toContain("chatDiffReviewTarget: DiffReviewTarget;");
    expect(displaySettings).toContain("gitDiffReviewTarget: DiffReviewTarget;");
    expect(displaySettings).toContain('chatDiffReviewTarget: "inline",');
    expect(displaySettings).toContain('gitDiffReviewTarget: "inline",');

    expect(displayPanel).toContain('<div class="section-label">{{ t("settings.display.diffReviewTitle") }}</div>');
    expect(displayPanel).toContain('<p class="section-desc">{{ t("settings.display.diffReviewDesc") }}</p>');
    expect(displayPanel).toContain("settings.display.diffReviewChatTarget");
    expect(displayPanel).toContain("settings.display.diffReviewGitTarget");
    expect(displayPanel).toContain(":model-value=\"display.chatDiffReviewTarget\"");
    expect(displayPanel).toContain(":model-value=\"display.gitDiffReviewTarget\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('chatDiffReviewTarget', $event as DiffReviewTarget)\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('gitDiffReviewTarget', $event as DiffReviewTarget)\"");
    expect(displayPanel.indexOf("settings.display.diffReviewTitle")).toBeGreaterThan(
      displayPanel.indexOf("settings.display.panelBehaviorTitle"),
    );
    expect(displayPanel.indexOf("settings.display.diffReviewTitle")).toBeLessThan(
      displayPanel.indexOf("settings.display.gitViewTitle"),
    );

    expect(chatChangesPanel).toContain("displaySettings.chatDiffReviewTarget === \"window\"");
    expect(chatChangesPanel).toContain("openChatDiffReviewWindow({ request })");
    expect(collabView).toContain("displaySettings.gitDiffReviewTarget === \"window\"");
    expect(collabView).toContain("openFileDiffReviewWindow({ request })");
    expect(chatView).toContain("openInlineDiffInWindow");
    expect(chatView).toContain("chat.changes.openReviewWindow");
    expect(chatReviewWindow).toContain(":hide-text-display-controls=\"true\"");
    expect(chatReviewWindow).toContain("fileDiffViewerRef?.hasTextDisplayModeControl");
    expect(chatReviewWindow).toContain("fileDiffViewerRef.toggleTextDisplayMode()");
    expect(chatReviewWindow.indexOf("common.openInEditor")).toBeLessThan(
      chatReviewWindow.indexOf("diff.mode.sideBySide"),
    );
    expect(fileDiffViewer).toContain("hideTextDisplayControls?: boolean;");
    expect(fileDiffViewer).toContain("hasTextDisplayModeControl");
    expect(fileDiffViewer).toContain("toggleTextDisplayMode,");

    expect(app).toContain("isChatDiffReviewWindowLocation");
    expect(app).toContain("<ChatDiffReviewWindow v-else-if=\"isChatDiffReviewWindow\" />");
    expect(capability).toContain('"chat-diff-review"');

    expect(zh).toContain('"settings.display.panelBehaviorDesc": "控制会话面板的打开与关闭"');
    expect(zh).toContain('"settings.display.diffReviewTitle": "文件修改审查"');
    expect(zh).toContain('"settings.display.diffReviewDesc": "选择文件修改审查的默认打开位置"');
    expect(zh).toContain('"settings.display.diffReviewChatTarget": "会话修改"');
    expect(zh).toContain('"settings.display.diffReviewGitTarget": "Git 修改"');
    expect(zh).toContain('"settings.display.diffReviewWindow": "独立窗口"');
    expect(en).toContain('"settings.display.panelBehaviorDesc": "Control how session panels open and close"');
    expect(en).toContain('"settings.display.diffReviewTitle": "File Change Review"');
    expect(en).toContain('"settings.display.diffReviewDesc": "Choose where file change reviews open by default"');
    expect(en).toContain('"settings.display.diffReviewChatTarget": "Session changes"');
    expect(en).toContain('"settings.display.diffReviewGitTarget": "Git changes"');
    expect(en).toContain('"settings.display.diffReviewWindow": "Separate window"');
  });

  it("adds a completed thinking block visibility toggle that defaults to hidden", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("hideThinkingBlocks: boolean;");
    expect(displaySettings).toContain("hideThinkingBlocks: true,");

    expect(displayPanel).toContain(":model-value=\"display.hideThinkingBlocks\"");
    expect(displayPanel).toContain(":aria-label=\"t('settings.display.hideThinkingBlocks')\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('hideThinkingBlocks', $event)\"");
    expect(displayPanel).toContain("{{ t(\"settings.display.hideThinkingBlocks\") }}");

    expect(transcript).toContain("function shouldHideThinkingBlocks()");
    expect(transcript).toContain("return displaySettings.hideThinkingBlocks !== false;");
    expect(transcript).toContain("return !shouldHideThinkingBlocks() && !!item.message.thinkingContent?.trim();");
    expect(transcript).toContain("const hasVisibleCompletedThinkingContent = computed(() =>");
    expect(transcript).toContain("&& canonicalLiveRenderParts.value.some((part) =>");
    expect(transcript).toContain("const hasVisibleActiveThinkingBlock = computed(() =>");
    expect(transcript).toContain("part.kind === \"thinking\" && part.active");
    expect(transcript).toContain("hasVisibleActiveThinkingBlock.value || hasVisibleCompletedThinkingContent.value");
    expect(transcript).toContain("hasThinkingContent: hasVisibleCompletedThinkingContent.value,");
    expect(transcript).toContain("function shouldRenderTransientThinkingSegment(");
    expect(transcript).toContain("return !!part.active || (!shouldHideThinkingBlocks() && part.content.trim().length > 0);");
    expect(transcript).toMatch(/if \(part\.kind === "thinking"\) \{\s+if \(!shouldRenderTransientThinkingSegment\(part\)\) continue;\s+flushPendingTools\(\);/);

    expect(zh).toContain('"settings.display.hideThinkingBlocks": "隐藏已完成思考块"');
    expect(en).toContain('"settings.display.hideThinkingBlocks": "Hide completed thinking blocks"');
  });
});
