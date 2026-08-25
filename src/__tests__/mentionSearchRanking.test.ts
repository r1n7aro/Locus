import { describe, expect, it } from "vitest";
import {
  rankMentionSearchResults,
  type MentionSearchRankable,
} from "../components/chat/mentionSearchRanking";

function result(
  entryKind: MentionSearchRankable["entryKind"],
  name: string,
  relPath: string,
): MentionSearchRankable {
  return {
    entryKind,
    name,
    relPath,
    parentPath: relPath.slice(0, Math.max(0, relPath.lastIndexOf("/"))),
    matchScore: 1,
  };
}

describe("mention search ranking", () => {
  it("places knowledge documents before similarly matching assets", () => {
    const asset = result("asset", "PlayerInput.cs", "Assets/Input/PlayerInput.cs");
    const knowledge = result(
      "knowledge",
      "Input architecture",
      "design/input-architecture.md",
    );

    expect(rankMentionSearchResults([asset, knowledge], "input")).toEqual([
      knowledge,
      asset,
    ]);
  });

  it("applies the knowledge priority to Chinese queries", () => {
    const asset = result("asset", "输入配置", "Assets/Config/输入配置.asset");
    const knowledge = result("knowledge", "输入设计", "design/输入设计.md");

    expect(rankMentionSearchResults([asset, knowledge], "输入")).toEqual([
      knowledge,
      asset,
    ]);
  });

  it("keeps a clearly more accurate asset match first", () => {
    const asset = result("asset", "Input", "Assets/Input");
    const knowledge = result(
      "knowledge",
      "Input architecture",
      "design/input-architecture.md",
    );

    expect(rankMentionSearchResults([knowledge, asset], "input")).toEqual([
      asset,
      knowledge,
    ]);
  });
});
