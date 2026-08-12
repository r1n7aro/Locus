import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("parallel tool completion streaming", () => {
  it("streams each completed result before the whole round is persisted", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const orderedPersistence = agent.indexOf(
      "Persist in request order so history/model context stays deterministic",
    );
    const parallelStart = agent.lastIndexOf(
      "let mut pending = futures::stream::FuturesUnordered::new()",
      orderedPersistence,
    );
    const completionPoll = agent.lastIndexOf(
      "while let Some((index, result)) = pending.next().await",
      orderedPersistence,
    );
    const completionMarker = agent.lastIndexOf(
      '"tool_result_completed"',
      orderedPersistence,
    );
    const completionStream = agent.lastIndexOf(
      "self.stream_completed_tool_result(",
      orderedPersistence,
    );
    const toolResultSave = agent.indexOf(
      "store.add_tool_result_with_images_for_run(",
      orderedPersistence,
    );

    expect(completionMarker).toBeGreaterThan(0);
    expect(parallelStart).toBeGreaterThan(0);
    expect(completionPoll).toBeGreaterThan(parallelStart);
    expect(completionMarker).toBeGreaterThan(completionPoll);
    expect(completionStream).toBeGreaterThan(completionMarker);
    expect(orderedPersistence).toBeGreaterThan(completionStream);
    expect(toolResultSave).toBeGreaterThan(orderedPersistence);
    expect(agent.slice(parallelStart, orderedPersistence)).not.toContain("join_all");
  });
});
