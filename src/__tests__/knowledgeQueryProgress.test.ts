// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createApp, nextTick } from "vue";
import { describe, expect, it } from "vitest";
import KnowledgeQueryToolBlock from "../components/tool-block-overrides/KnowledgeQueryToolBlock.vue";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("knowledgeQueryProgress", () => {
  it("wires knowledge_query execution stages through the agent stream", () => {
    const agentSource = read("src-tauri/src/agent/instance/mod.rs");
    const indexSource = read("src-tauri/src/knowledge_index/mod.rs");

    expect(agentSource).toContain("execute_knowledge_query(app_handle, &tc.id, args, run_id)");
    expect(agentSource).toContain("Preparing knowledge query");
    expect(agentSource).toContain("Formatting knowledge results");
    expect(agentSource).toContain("Knowledge query timed out");
    expect(agentSource).toContain("tokio::time::timeout");
    expect(agentSource).toContain("query_documents_with_progress");
    expect(agentSource).toContain("StreamEvent::ToolCallProgress");

    expect(indexSource).toContain("pub async fn query_documents_with_progress");
    expect(indexSource).toContain("Loading knowledge search config");
    expect(indexSource).toContain("Checking knowledge catalog");
    expect(indexSource).toContain("Running lexical index search");
    expect(indexSource).toContain("Running text scan");
    expect(indexSource).toContain("Checking text scan documents");
    expect(indexSource).toContain("Loading text scan documents");
    expect(indexSource).toContain("Scanning knowledge text");
    expect(indexSource).toContain("Sorting text scan results");
    expect(indexSource).toContain("knowledge_query text scan timed out");
    expect(indexSource).toContain("Text scan document limit exceeded");
    expect(indexSource).toContain("knowledge_query text scan can scan at most");
    expect(indexSource).toContain("Checking semantic search");
    expect(indexSource).toContain("Running semantic search");
    expect(indexSource).toContain("Loading matched documents");
    expect(indexSource).toContain("Filtering knowledge access");
    expect(indexSource).toContain("Ranking knowledge results");

    expect(agentSource).toContain("knowledge.query_text_scan_too_large");
    expect(agentSource).toContain("知识文档数量过多");
    expect(agentSource).toContain("AppError::emit_background");
  });

  it("uses a dedicated knowledge_query tool block for visible runtime stages", () => {
    const overrideSource = read("src/components/tool-block-overrides/toolBlockOverrides.ts");
    const blockSource = read("src/components/tool-block-overrides/KnowledgeQueryToolBlock.vue");

    expect(overrideSource).toContain("knowledge_query: KnowledgeQueryToolBlock");
    expect(blockSource).toContain("props.toolCall.progress");
    expect(blockSource).toContain("class=\"tool-call-progress-line\"");
    expect(blockSource).toContain("class=\"knowledge-query-progress-track\"");
    expect(blockSource).toContain("buildToolCallArgsSummary");
    expect(blockSource).toContain("class=\"tool-args-table\"");
    expect(blockSource).toContain("v-for=\"arg in parsedArgs\"");
    expect(blockSource).not.toContain("{{ toolCall.arguments }}</pre>");
    expect(blockSource).toContain("var(--border-color)");
    expect(blockSource).not.toContain("#8b7cf6");
  });

  it("returns physical source text around each knowledge hit", () => {
    const agentSource = read("src-tauri/src/agent/instance/mod.rs");
    const indexSource = read("src-tauri/src/knowledge_index/mod.rs");

    expect(indexSource).toContain("read_sanitized_search_hit_context(");
    expect(indexSource).toContain("hit.snippet = context.text;");
    expect(agentSource).toContain("knowledge_hit_context_anchor");
    expect(agentSource).toContain('output.push_str(" | lines ")');
    expect(agentSource).toContain('output.push_str(" ---")');
  });

  it("renders knowledge_query arguments with the standard key-value layout", async () => {
    const host = document.createElement("div");
    const app = createApp(KnowledgeQueryToolBlock, {
      toolCall: {
        id: "tool-knowledge-query",
        name: "knowledge_query",
        arguments: JSON.stringify({
          lexicalQuery: "memory preferences",
          limit: 5,
          includeSummary: false,
          includeHitContext: true,
        }),
        status: "done",
        output: "No results.",
      },
    });
    app.mount(host);
    host.querySelector<HTMLButtonElement>(".tool-call-header")?.click();
    await nextTick();

    const rows = [...host.querySelectorAll<HTMLElement>(".tool-arg-row")];
    expect(rows.map((row) => row.querySelector(".tool-arg-key")?.textContent)).toEqual([
      "lexical query",
      "limit",
      "include summary",
      "include hit context",
    ]);
    expect(rows.map((row) => row.querySelector(".tool-arg-value")?.textContent)).toEqual([
      "memory preferences",
      "5",
      "false",
      "true",
    ]);
    expect(host.querySelector(".tool-call-section:first-child .tool-call-pre")).toBeNull();

    app.unmount();
  });

  it("injects registered physical knowledge directories without a synthetic tree root", () => {
    const agentSource = read("src-tauri/src/agent/instance/mod.rs");

    expect(agentSource).toContain("prompt_relative_physical_path(&resolved.physical_path");
    expect(agentSource).toContain("`Structure` lists physical directories: workspace-relative inside the checkout");
    expect(agentSource).toContain('render_scope("Project", project_roots)');
    expect(agentSource).toContain('render_scope("App", app_roots)');
    expect(agentSource).toContain("KnowledgeSourceKind::AppSkillPackage => source");
    expect(agentSource).toContain("app_skill_packages: BTreeMap<String, PromptAppSkillPackage>");
    expect(agentSource).toContain("relative_root: prompt_relative_physical_path(");
    expect(agentSource).toContain("for package in root.app_skill_packages.values()");
    expect(agentSource).toContain("package.desc.as_deref()");
    expect(agentSource).not.toContain("package.display_root.trim_end_matches('/')");
    expect(agentSource).not.toContain('"knowledge/".to_string(),');
  });

  it("hides L1 structure entries without explicit summaries", () => {
    const agentSource = read("src-tauri/src/agent/instance/mod.rs");

    expect(agentSource).not.toContain("body_excerpt");
    expect(agentSource).toContain("return summary;");
    expect(agentSource).toContain("KnowledgeInjectMode::Excerpt => item");
    expect(agentSource).toContain(".is_some_and(|summary| !summary.trim().is_empty())");
  });
});
