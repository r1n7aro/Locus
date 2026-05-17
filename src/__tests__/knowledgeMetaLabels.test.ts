import { describe, expect, it, vi } from "vitest";

vi.mock("../i18n", () => ({
  t: (key: string) => ({
    "knowledge.meta.inject.none": "仅搜索",
    "knowledge.meta.inject.path": "L0 - 路径注入",
    "knowledge.meta.inject.excerpt": "L1 - 摘要注入",
    "knowledge.meta.inject.full": "L2 - 全文注入",
    "knowledge.meta.inject.rule": "L3 - 全文规则注入",
    "knowledge.meta.aiMaintained": "AI 自动维护",
    "knowledge.meta.tag.auto": "AUTO",
    "knowledge.meta.tag.lexicalOn": "LX",
    "knowledge.meta.tag.lexicalOff": "LX-",
    "knowledge.meta.tag.semanticOn": "SM",
    "knowledge.meta.tag.semanticOff": "SM-",
    "knowledge.meta.tag.grep": "TXT",
    "knowledge.search.lexical": "全文匹配",
    "knowledge.search.semantic": "语义匹配",
    "knowledge.search.grep": "文本匹配",
    "knowledge.directoryConfig.lexicalSearch": "全文检索",
    "knowledge.directoryConfig.semanticSearch": "语义检索",
    "knowledge.directoryConfig.lexicalRuleInheritHint": "沿用 LX 规则",
    "knowledge.directoryConfig.lexicalRuleEnableHint": "LX。进入全文索引",
    "knowledge.directoryConfig.lexicalRuleDisableHint": "停用 LX。",
    "knowledge.directoryConfig.semanticRuleInheritHint": "沿用 SM 规则",
    "knowledge.directoryConfig.semanticRuleEnableHint": "SM。保留向量嵌入",
    "knowledge.directoryConfig.semanticRuleDisableHint": "停用 SM。",
    "knowledge.folder.ruleEnable": "开启",
    "knowledge.folder.ruleDisable": "关闭",
    "knowledge.source.external": "外部来源",
    "knowledge.source.localFolder": "本地文件夹",
    "knowledge.source.feishu": "飞书",
    "knowledge.source.url": "链接",
    "knowledge.source.package": "包文档",
    "knowledge.source.unity": "Unity",
    "knowledge.source.custom": "自定义来源",
  }[key] ?? key),
}));

import {
  buildExternalFolderTag,
  buildFolderListTags,
  buildFolderSearchTags,
  buildKnowledgeListTags,
  buildKnowledgeSearchMatchTags,
  hintForFolderSearchRule,
  labelForFolderSearchRule,
  labelForInjectMode,
} from "../components/knowledge/knowledgeMetaLabels";

describe("knowledgeMetaLabels", () => {
  it("maps inject modes to L0/L1/L2/L3 labels in metadata config", () => {
    expect(labelForInjectMode("none")).toBe("仅搜索");
    expect(labelForInjectMode("path")).toBe("L0 - 路径注入");
    expect(labelForInjectMode("excerpt")).toBe("L1 - 摘要注入");
    expect(labelForInjectMode("full")).toBe("L2 - 全文注入");
    expect(labelForInjectMode("rule")).toBe("L3 - 全文规则注入");
  });

  it("builds explorer tags from inject mode and maintenance mode", () => {
    expect(buildKnowledgeListTags({ injectMode: "none", aiMaintained: false })).toEqual([]);
    expect(buildKnowledgeListTags({ injectMode: "path", aiMaintained: false })).toMatchObject([
      { text: "L0", tone: "inject" },
    ]);
    expect(buildKnowledgeListTags({ injectMode: "excerpt", aiMaintained: false })).toMatchObject([
      { text: "L1", tone: "inject" },
    ]);
    expect(buildKnowledgeListTags({ injectMode: "full", aiMaintained: true })).toMatchObject([
      { text: "L2", tone: "inject" },
      { text: "AUTO", tone: "auto" },
    ]);
    expect(buildKnowledgeListTags({ injectMode: "rule", aiMaintained: true })).toMatchObject([
      { text: "L3", tone: "inject-strong" },
      { text: "AUTO", tone: "auto" },
    ]);
  });

  it("builds root folder retrieval tags from effective lexical and semantic access", () => {
    expect(buildFolderSearchTags({ lexicalEnabled: true, semanticEnabled: false })).toEqual([
      {
        text: "LX",
        tone: "search-on",
        title: "LX - 全文检索 · 开启",
      },
    ]);
    expect(buildFolderSearchTags({ lexicalEnabled: false, semanticEnabled: true })).toEqual([
      {
        text: "SM",
        tone: "search-on",
        title: "SM - 语义检索 · 开启",
      },
    ]);
    expect(buildFolderSearchTags({ lexicalEnabled: false, semanticEnabled: false })).toEqual([]);
  });

  it("builds root folder explorer tags from inject mode and retrieval access", () => {
    expect(buildFolderListTags({
      injectMode: "excerpt",
      lexicalEnabled: true,
      semanticEnabled: true,
    })).toEqual([
      {
        text: "L1",
        tone: "inject",
        title: "L1 - 摘要注入",
      },
      {
        text: "LX",
        tone: "search-on",
        title: "LX - 全文检索 · 开启",
      },
      {
        text: "SM",
        tone: "search-on",
        title: "SM - 语义检索 · 开启",
      },
    ]);
  });

  it("builds provider-specific external folder tags for known managed sources", () => {
    expect(
      buildExternalFolderTag([{ provider: "feishu" }]),
    ).toEqual({
      text: "FEISHU",
      tone: "external",
      title: "外部来源 · 飞书",
    });
    expect(
      buildExternalFolderTag([{ provider: "unity" }]),
    ).toEqual({
      text: "UNITY-DOC",
      tone: "external",
      title: "外部来源 · Unity",
    });
    expect(
      buildExternalFolderTag([{ provider: "local_folder" }]),
    ).toEqual({
      text: "EXT",
      tone: "external",
      title: "外部来源 · 本地文件夹",
    });
  });

  it("builds search-result retrieval tags from the recall path", () => {
    expect(buildKnowledgeSearchMatchTags("lexical")).toEqual([
      {
        text: "LX",
        tone: "search-on",
        title: "LX - 全文匹配",
      },
    ]);
    expect(buildKnowledgeSearchMatchTags("semantic")).toEqual([
      {
        text: "SM",
        tone: "search-on",
        title: "SM - 语义匹配",
      },
    ]);
    expect(buildKnowledgeSearchMatchTags("hybrid")).toEqual([
      {
        text: "LX",
        tone: "search-on",
        title: "LX - 全文匹配",
      },
      {
        text: "SM",
        tone: "search-on",
        title: "SM - 语义匹配",
      },
    ]);
    expect(buildKnowledgeSearchMatchTags("grep")).toEqual([
      {
        text: "TXT",
        tone: "search-on",
        title: "TXT - 文本匹配",
      },
    ]);
    expect(buildKnowledgeSearchMatchTags("grepHybrid")).toEqual([
      {
        text: "TXT",
        tone: "search-on",
        title: "TXT - 文本匹配",
      },
      {
        text: "SM",
        tone: "search-on",
        title: "SM - 语义匹配",
      },
    ]);
  });

  it("builds explicit folder rule labels and hints with LX/SM prefixes", () => {
    expect(labelForFolderSearchRule("lexical", true)).toBe("LX - 开启");
    expect(labelForFolderSearchRule("semantic", false)).toBe("SM - 关闭");
    expect(hintForFolderSearchRule("lexical", "enabled")).toBe("LX。进入全文索引");
    expect(hintForFolderSearchRule("semantic", "inherit")).toBe("沿用 SM 规则");
  });
});
