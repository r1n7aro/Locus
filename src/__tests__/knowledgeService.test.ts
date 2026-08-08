import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import { knowledgeEdit, knowledgeList, knowledgeQuery } from "../services/knowledge";

const mockedInvoke = vi.mocked(ipcInvoke);

describe("knowledge service visibility defaults", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue([]);
  });

  it("keeps hidden documents in the management list", async () => {
    await knowledgeList();

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_list", {
      docType: undefined,
      pathPrefix: undefined,
      includeHidden: true,
    });
  });

  it("excludes hidden documents from retrieval by default", async () => {
    await knowledgeQuery({ query: "external skill" });

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_query", {
      query: "external skill",
      limit: undefined,
      types: undefined,
      pathPrefix: undefined,
      includeHidden: false,
    });
  });

  it("preserves an explicit hidden-document query", async () => {
    await knowledgeQuery({ query: "external skill", includeHidden: true });

    expect(mockedInvoke).toHaveBeenCalledWith(
      "knowledge_query",
      expect.objectContaining({ includeHidden: true }),
    );
  });

  it("omits local maintenance rules when a directory inherits its edit config", async () => {
    mockedInvoke.mockResolvedValueOnce({
      kind: "directory",
      type: "memory",
      path: "ecs-migration",
    });

    await knowledgeEdit({
      kind: "directory",
      type: "memory",
      path: "ecs-migration",
      config: {
        summary: "ECS",
        aiMaintained: "inherit",
        maintenanceRules: null,
      },
    });

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_edit", {
      request: {
        kind: "directory",
        path: "memory/ecs-migration",
        type: "memory",
        document: undefined,
        config: {
          summary: "ECS",
          inheritAiConfig: true,
        },
      },
    });
  });
});
