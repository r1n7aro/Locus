import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("parallel tool completion streaming", () => {
  it("streams each completed result before the whole round is persisted", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const roundStart = agent.indexOf("let completed_results = if execute_sequentially");
    const parallelStart = agent.indexOf(
      "let mut pending = futures::stream::FuturesUnordered::new()",
      roundStart,
    );
    const completionPoll = agent.indexOf(
      "while let Some((index, result)) = pending.next().await",
      parallelStart,
    );
    const completionStream = agent.indexOf(
      "self.stream_completed_tool_result(",
      completionPoll,
    );
    const orderedPersistence = agent.indexOf(
      "Persist in request order so history/model context stays deterministic",
      completionStream,
    );
    const toolResultSave = agent.indexOf(
      "store.add_tool_result_with_images_for_run(",
      orderedPersistence,
    );

    expect(roundStart).toBeGreaterThan(0);
    expect(parallelStart).toBeGreaterThan(roundStart);
    expect(completionPoll).toBeGreaterThan(parallelStart);
    expect(completionStream).toBeGreaterThan(completionPoll);
    expect(orderedPersistence).toBeGreaterThan(completionStream);
    expect(toolResultSave).toBeGreaterThan(orderedPersistence);
    expect(agent.slice(parallelStart, orderedPersistence)).not.toContain("join_all");
  });
});
