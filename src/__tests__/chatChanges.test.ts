import { describe, expect, it } from "vitest";
import {
  buildMergedFiles,
  buildRounds,
  mergeRoundFiles,
  type ChatChangeRound,
} from "../services/chatChanges";
import type { ChangedFile, VcsUndoEntry } from "../types";

function round(
  createdAt: number,
  files: ChangedFile[],
  assistantMessageId = `msg-${createdAt}`,
): ChatChangeRound {
  return {
    assistantMessageId,
    runId: "run-1",
    checkpoint: { id: `cp-${createdAt}`, label: "round", createdAt },
    files,
  };
}

function entry(
  createdAt: number,
  files: ChangedFile[],
  assistantMessageId = `msg-${createdAt}`,
): VcsUndoEntry {
  return {
    id: `entry-${createdAt}`,
    sessionId: "s1",
    assistantMessageId,
    runId: "run-1",
    checkpoint: { id: `cp-${createdAt}`, label: "round", createdAt },
    changedFiles: files,
    hasUnityExecute: false,
    consumed: false,
  };
}

const A = (path: string): ChangedFile => ({ status: "A", path });
const M = (path: string): ChangedFile => ({ status: "M", path });
const D = (path: string): ChangedFile => ({ status: "D", path });
const R = (oldPath: string, path: string): ChangedFile => ({ status: "R", path, oldPath });

describe("mergeRoundFiles", () => {
  it("nets a file created then deleted across rounds to nothing", () => {
    expect(mergeRoundFiles([round(1, [A("tmp.ts")]), round(2, [D("tmp.ts")])])).toEqual([]);
  });

  it("keeps A for a file created then modified across rounds", () => {
    const merged = mergeRoundFiles([round(1, [A("new.ts")]), round(2, [M("new.ts")])]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({ finalPath: "new.ts", status: "A", roundCount: 2 });
    expect(merged[0].baseOldPath).toBeUndefined();
  });

  it("nets delete-then-recreate of a pre-existing file to M", () => {
    const merged = mergeRoundFiles([round(1, [D("file.ts")]), round(2, [A("file.ts")])]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({ finalPath: "file.ts", status: "M" });
  });

  it("nets modify-then-delete of a pre-existing file to D", () => {
    const merged = mergeRoundFiles([round(1, [M("file.ts")]), round(2, [D("file.ts")])]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({ finalPath: "file.ts", status: "D" });
  });

  it("collapses rename chains across rounds", () => {
    const merged = mergeRoundFiles([
      round(1, [R("a.ts", "b.ts")]),
      round(2, [R("b.ts", "c.ts")]),
    ]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({
      finalPath: "c.ts",
      baseOldPath: "a.ts",
      status: "R",
      roundCount: 2,
    });
  });

  it("merges modify-then-rename into a single renamed row", () => {
    const merged = mergeRoundFiles([round(1, [M("a.ts")]), round(2, [R("a.ts", "b.ts")])]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({ finalPath: "b.ts", baseOldPath: "a.ts", status: "R" });
  });

  it("keeps A without old path for a file introduced then renamed", () => {
    const merged = mergeRoundFiles([round(1, [A("a.ts")]), round(2, [R("a.ts", "b.ts")])]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({ finalPath: "b.ts", status: "A" });
    expect(merged[0].baseOldPath).toBeUndefined();
  });

  it("nets a renamed file deleted at its new path to D at the origin path", () => {
    const merged = mergeRoundFiles([round(1, [R("a.ts", "b.ts")]), round(2, [D("b.ts")])]);
    expect(merged).toHaveLength(1);
    expect(merged[0]).toMatchObject({ finalPath: "b.ts", baseOldPath: "a.ts", status: "D" });
  });

  it("orders rounds by checkpoint time regardless of input order", () => {
    expect(mergeRoundFiles([round(2, [D("tmp.ts")]), round(1, [A("tmp.ts")])])).toEqual([]);
  });

  it("anchors baseAssistantMessageId to the first round that touched the file", () => {
    const merged = mergeRoundFiles([
      round(1, [M("a.ts")], "msg-first"),
      round(2, [M("a.ts"), M("b.ts")], "msg-second"),
    ]);
    expect(merged).toEqual([
      expect.objectContaining({
        finalPath: "a.ts",
        baseAssistantMessageId: "msg-first",
        roundCount: 2,
      }),
      expect.objectContaining({
        finalPath: "b.ts",
        baseAssistantMessageId: "msg-second",
        roundCount: 1,
      }),
    ]);
  });
});

describe("buildMergedFiles", () => {
  it("matches mergeRoundFiles over the rounds built from entries", () => {
    const entries = [
      entry(1, [A("tmp.ts"), M("kept.ts")]),
      entry(2, [D("tmp.ts"), R("kept.ts", "renamed.ts")]),
    ];
    expect(buildMergedFiles(entries)).toEqual(mergeRoundFiles(buildRounds(entries)));
    expect(buildMergedFiles(entries)).toEqual([
      expect.objectContaining({
        finalPath: "renamed.ts",
        baseOldPath: "kept.ts",
        status: "R",
      }),
    ]);
  });
});
