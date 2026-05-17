import { t } from "../../i18n";
import type {
  FolderIndexRuleSetting,
  KnowledgeConfigSource,
  KnowledgeEditMode,
  KnowledgeExternalSource,
  KnowledgeInjectMode,
  KnowledgeSearchMatchKind,
} from "../../types";

export interface KnowledgeListTag {
  text: string;
  tone: "inject" | "inject-strong" | "auto" | "search-on" | "external";
  title: string;
}

export type KnowledgeSearchTagKind = "lexical" | "semantic";

export function labelForInjectMode(mode: KnowledgeInjectMode): string {
  switch (mode) {
    case "path":
      return t("knowledge.meta.inject.path");
    case "excerpt":
      return t("knowledge.meta.inject.excerpt");
    case "full":
      return t("knowledge.meta.inject.full");
    case "rule":
      return t("knowledge.meta.inject.rule");
    default:
      return t("knowledge.meta.inject.none");
  }
}

export function hintForInjectMode(mode: KnowledgeInjectMode): string {
  switch (mode) {
    case "path":
      return t("knowledge.meta.inject.pathHint");
    case "excerpt":
      return t("knowledge.meta.inject.excerptHint");
    case "full":
      return t("knowledge.meta.inject.fullHint");
    case "rule":
      return t("knowledge.meta.inject.ruleHint");
    default:
      return t("knowledge.meta.inject.noneHint");
  }
}

export function labelForKnowledgeEditMode(mode: KnowledgeEditMode): string {
  switch (mode) {
    case "inherit_parent":
      return t("knowledge.meta.editMode.inheritParent");
    case "auto":
      return t("knowledge.meta.editMode.auto");
    case "proposal":
      return t("knowledge.meta.editMode.proposal");
    default:
      return t("knowledge.meta.editMode.readOnly");
  }
}

export function hintForKnowledgeEditMode(mode: KnowledgeEditMode): string {
  switch (mode) {
    case "inherit_parent":
      return t("knowledge.meta.editMode.inheritParentHint");
    case "auto":
      return t("knowledge.meta.editMode.autoHint");
    case "proposal":
      return t("knowledge.meta.editMode.proposalHint");
    default:
      return t("knowledge.meta.editMode.readOnlyHint");
  }
}

export function labelForConfigSource(source?: KnowledgeConfigSource | null): string {
  switch (source?.kind) {
    case "parent_directory":
      return t("knowledge.meta.configSource.parentDirectory");
    case "type_default":
      return t("knowledge.meta.configSource.typeDefault");
    default:
      return t("knowledge.meta.configSource.self");
  }
}

export function describeConfigSource(source?: KnowledgeConfigSource | null): string {
  const label = labelForConfigSource(source);
  const path = source?.path?.trim();
  if (source?.kind === "parent_directory" && path) {
    return `${label} · ${path}`;
  }
  return label;
}

export function labelForInheritedValue(
  valueLabel: string,
  source?: KnowledgeConfigSource | null,
): string {
  const inheritLabel = source?.kind === "type_default"
    ? t("knowledge.meta.inheritTypeDefault")
    : t("knowledge.meta.inheritParent");
  return `${inheritLabel}：${valueLabel}`;
}

export function injectLevelTag(mode: KnowledgeInjectMode): KnowledgeListTag | null {
  switch (mode) {
    case "path":
      return { text: "L0", tone: "inject", title: labelForInjectMode(mode) };
    case "excerpt":
      return { text: "L1", tone: "inject", title: labelForInjectMode(mode) };
    case "full":
      return { text: "L2", tone: "inject", title: labelForInjectMode(mode) };
    case "rule":
      return { text: "L3", tone: "inject-strong", title: labelForInjectMode(mode) };
    default:
      return null;
  }
}

export function tagForSearchKind(kind: KnowledgeSearchTagKind): string {
  return kind === "lexical"
    ? t("knowledge.meta.tag.lexicalOn")
    : t("knowledge.meta.tag.semanticOn");
}

export function labelForSearchKind(kind: KnowledgeSearchTagKind): string {
  return kind === "lexical"
    ? t("knowledge.directoryConfig.lexicalSearch")
    : t("knowledge.directoryConfig.semanticSearch");
}

export function labelForFolderSearchRule(
  kind: KnowledgeSearchTagKind,
  enabled: boolean,
): string {
  return `${tagForSearchKind(kind)} - ${enabled
    ? t("knowledge.folder.ruleEnable")
    : t("knowledge.folder.ruleDisable")}`;
}

export function hintForFolderSearchRule(
  kind: KnowledgeSearchTagKind,
  value: FolderIndexRuleSetting,
): string {
  if (kind === "lexical") {
    switch (value) {
      case "enabled":
        return t("knowledge.directoryConfig.lexicalRuleEnableHint");
      case "disabled":
        return t("knowledge.directoryConfig.lexicalRuleDisableHint");
      default:
        return t("knowledge.directoryConfig.lexicalRuleInheritHint");
    }
  }

  switch (value) {
    case "enabled":
      return t("knowledge.directoryConfig.semanticRuleEnableHint");
    case "disabled":
      return t("knowledge.directoryConfig.semanticRuleDisableHint");
    default:
      return t("knowledge.directoryConfig.semanticRuleInheritHint");
  }
}

export function buildKnowledgeListTags(meta: {
  injectMode: KnowledgeInjectMode;
  aiMaintained: boolean;
}): KnowledgeListTag[] {
  const tags: KnowledgeListTag[] = [];
  const injectTag = injectLevelTag(meta.injectMode);
  if (injectTag) tags.push(injectTag);
  if (meta.aiMaintained) {
    tags.push({
      text: t("knowledge.meta.tag.auto"),
      tone: "auto",
      title: t("knowledge.meta.aiMaintained"),
    });
  }
  return tags;
}

function labelForExternalSourceProvider(provider: KnowledgeExternalSource["provider"]): string {
  switch (provider) {
    case "local_folder":
      return t("knowledge.source.localFolder");
    case "feishu":
      return t("knowledge.source.feishu");
    case "url":
      return t("knowledge.source.url");
    case "package":
      return t("knowledge.source.package");
    case "unity":
      return t("knowledge.source.unity");
    default:
      return t("knowledge.source.custom");
  }
}

function textForExternalFolderTag(
  providers: KnowledgeExternalSource["provider"][],
): string {
  if (providers.length === 1) {
    if (providers[0] === "feishu") return "FEISHU";
    if (providers[0] === "unity") return "UNITY-DOC";
  }
  return "EXT";
}

export function buildExternalFolderTag(
  sources: KnowledgeExternalSource[] | null | undefined,
): KnowledgeListTag | null {
  if (!Array.isArray(sources) || !sources.length) return null;
  const providers = Array.from(
    new Set(
      sources
        .map((source) => source?.provider)
        .filter((provider): provider is KnowledgeExternalSource["provider"] => !!provider),
    ),
  );
  const providerLabels = providers.map(labelForExternalSourceProvider);
  return {
    text: textForExternalFolderTag(providers),
    tone: "external",
    title: providerLabels.length
      ? `${t("knowledge.source.external")} · ${providerLabels.join(" / ")}`
      : t("knowledge.source.external"),
  };
}

export function buildFolderListTags(meta: {
  injectMode: KnowledgeInjectMode;
  lexicalEnabled: boolean;
  semanticEnabled: boolean;
}): KnowledgeListTag[] {
  const tags: KnowledgeListTag[] = [];
  const injectTag = injectLevelTag(meta.injectMode);
  if (injectTag) tags.push(injectTag);
  tags.push(...buildFolderSearchTags({
    lexicalEnabled: meta.lexicalEnabled,
    semanticEnabled: meta.semanticEnabled,
  }));
  return tags;
}

export function buildFolderSearchTags(meta: {
  lexicalEnabled: boolean;
  semanticEnabled: boolean;
}): KnowledgeListTag[] {
  const tags: KnowledgeListTag[] = [];
  if (meta.lexicalEnabled) {
    tags.push({
      text: tagForSearchKind("lexical"),
      tone: "search-on",
      title: `${tagForSearchKind("lexical")} - ${labelForSearchKind("lexical")} · ${t("knowledge.folder.ruleEnable")}`,
    });
  }
  if (meta.semanticEnabled) {
    tags.push({
      text: tagForSearchKind("semantic"),
      tone: "search-on",
      title: `${tagForSearchKind("semantic")} - ${labelForSearchKind("semantic")} · ${t("knowledge.folder.ruleEnable")}`,
    });
  }
  return tags;
}

export function buildKnowledgeSearchMatchTags(
  matchKind: KnowledgeSearchMatchKind,
): KnowledgeListTag[] {
  const tags: KnowledgeListTag[] = [];
  if (matchKind === "grep" || matchKind === "grepHybrid") {
    tags.push({
      text: t("knowledge.meta.tag.grep"),
      tone: "search-on",
      title: `${t("knowledge.meta.tag.grep")} - ${t("knowledge.search.grep")}`,
    });
  }
  if (matchKind === "lexical" || matchKind === "hybrid") {
    tags.push({
      text: tagForSearchKind("lexical"),
      tone: "search-on",
      title: `${tagForSearchKind("lexical")} - ${t("knowledge.search.lexical")}`,
    });
  }
  if (
    matchKind === "semantic" ||
    matchKind === "hybrid" ||
    matchKind === "grepHybrid"
  ) {
    tags.push({
      text: tagForSearchKind("semantic"),
      tone: "search-on",
      title: `${tagForSearchKind("semantic")} - ${t("knowledge.search.semantic")}`,
    });
  }
  return tags;
}
