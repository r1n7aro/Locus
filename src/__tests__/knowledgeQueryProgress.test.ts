import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

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
    expect(agentSource).toContain("query_documents_with_progress");
    expect(agentSource).toContain("StreamEvent::ToolCallProgress");

    expect(indexSource).toContain("pub async fn query_documents_with_progress");
    expect(indexSource).toContain("Loading knowledge search config");
    expect(indexSource).toContain("Checking knowledge catalog");
    expect(indexSource).toContain("Running lexical index search");
    expect(indexSource).toContain("Running text scan");
    expect(indexSource).toContain("Checking semantic search");
    expect(indexSource).toContain("Running semantic search");
    expect(indexSource).toContain("Loading matched documents");
    expect(indexSource).toContain("Filtering knowledge access");
    expect(indexSource).toContain("Ranking knowledge results");
  });

  it("uses a dedicated knowledge_query tool block for visible runtime stages", () => {
    const overrideSource = read("src/components/tool-block-overrides/toolBlockOverrides.ts");
    const blockSource = read("src/components/tool-block-overrides/KnowledgeQueryToolBlock.vue");

    expect(overrideSource).toContain("knowledge_query: KnowledgeQueryToolBlock");
    expect(blockSource).toContain("props.toolCall.progress");
    expect(blockSource).toContain("class=\"tool-call-progress-line\"");
    expect(blockSource).toContain("class=\"knowledge-query-progress-track\"");
    expect(blockSource).toContain("buildToolCallArgsSummary");
    expect(blockSource).toContain("var(--border-color)");
    expect(blockSource).not.toContain("#8b7cf6");
  });
});
