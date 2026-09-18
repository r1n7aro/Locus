import { describe, expect, it, vi } from "vitest";
import { createMentionSearchQueue } from "../components/chat/mentionSearchQueue";

describe("mention search request coalescing", () => {
  it("keeps one active request and runs only the latest queued query", async () => {
    const queue = createMentionSearchQueue();
    let finish!: (value: string) => void;
    const first = queue.run(() => new Promise<string>((resolve) => { finish = resolve; }));
    await Promise.resolve();
    const skipped = vi.fn(async () => "ab");
    const intermediate = queue.run(skipped);
    const last = queue.run(async () => "abc");
    expect(await intermediate).toBeUndefined();
    expect(skipped).not.toHaveBeenCalled();
    finish("a");
    expect(await first).toBe("a");
    expect(await last).toBe("abc");
  });

  it("discards queued work on close and recovers after a provider fails", async () => {
    const queue = createMentionSearchQueue();
    const failed = queue.run(async () => { throw new Error("offline"); });
    const skipped = vi.fn(async () => "queued");
    const pending = queue.run(skipped);
    queue.clear();
    expect(await pending).toBeUndefined();
    await expect(failed).rejects.toThrow("offline");
    expect(await queue.run(async () => "recovered")).toBe("recovered");
    expect(skipped).not.toHaveBeenCalled();
  });
});
