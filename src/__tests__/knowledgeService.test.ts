import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import { projectKnowledgeList } from "../services/workspaceExplorer";
import {
  knowledgeEdit,
  knowledgeList,
  knowledgeMove,
  knowledgeQuery,
  knowledgeRead,
  listSkills,
  readSkillManifest,
} from "../services/knowledge";

const mockedInvoke = vi.mocked(ipcInvoke);
const workspaceRef = {
  checkoutId: "checkout-feature",
  expectedGeneration: 7,
};

describe("knowledge service visibility defaults", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue([]);
  });

  it.each([true, false])("maps project knowledge update times for aiMaintained=%s", async (aiMaintained) => {
    const updatedAt = 1_789_623_600_000;
    const source = {
      sourceCheckoutId: "checkout-feature",
      sourceWorkspaceGeneration: 7,
      sourceRoot: "F:/Project",
      availableCheckoutIds: ["checkout-main", "checkout-feature"],
    };
    mockedInvoke.mockResolvedValueOnce([{
      id: "kd_combat", type: "design", path: "combat.md", title: "Combat",
      aiMaintained, updatedAt, ...source,
    }]);

    const documents = await projectKnowledgeList("project-a", { type: "design", pathPrefix: "combat" });

    expect(mockedInvoke).toHaveBeenCalledWith("project_knowledge_list", {
      projectId: "project-a", docType: "design", pathPrefix: "combat",
    });
    expect(documents).toEqual([{
      id: "kd_combat", type: "design", path: "combat.md", title: "Combat",
      aiMaintained, modifiedAt: updatedAt, ...source,
    }]);
  });

  it.each([undefined, [], ["unity", "reviewer"]])("preserves injection targets when reading metadata: %j", async (injectAgents) => {
    mockedInvoke.mockResolvedValueOnce({
      kind: "document", path: "agents.md",
      document: { id: "kd_agents", type: "design", path: "agents.md", title: "Agents", injectMode: "full", injectAgents, body: "Body", updatedAt: 0 },
    });
    const result = await knowledgeRead({ kind: "document", type: "design", path: "agents.md" }, workspaceRef);
    expect(result.document?.injectAgents).toEqual(injectAgents ?? ["unity"]);
  });

  it("sends an empty injection target list instead of omitting the metadata patch", async () => {
    mockedInvoke.mockResolvedValueOnce({ kind: "document", document: null });
    await knowledgeEdit({ kind: "document", type: "design", path: "agents.md", document: { injectAgents: [] } }, workspaceRef);
    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_edit", expect.objectContaining({
      request: expect.objectContaining({ document: expect.objectContaining({ injectAgents: [] }) }),
    }));
  });

  it("keeps hidden documents in the management list", async () => {
    await knowledgeList({}, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_list", {
      docType: undefined,
      pathPrefix: undefined,
      includeHidden: true,
      workspaceRef,
    });
  });

  it("preserves the destination category of a document move", async () => {
    mockedInvoke.mockResolvedValueOnce({ kind: "document", type: "memory", path: "note.md", resultPath: "note.md" });
    await knowledgeMove({ kind: "document", type: "design", path: "note.md", newPath: "memory/note.md" }, workspaceRef);
    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_move", {
      workspaceRef,
      request: { kind: "document", type: "design", path: "design/note.md", newPath: "memory/note.md" },
    });
  });

  it("carries the checkout reference through Skill list and read requests", async () => {
    await listSkills(workspaceRef);
    await readSkillManifest("asset-audit", "project", workspaceRef);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "list_skills", {
      workspaceRef,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "read_skill_manifest", {
      dirName: "asset-audit",
      source: "project",
      workspaceRef,
    });
  });

  it("excludes hidden documents from retrieval by default", async () => {
    await knowledgeQuery({ query: "external skill" }, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_query", {
      query: "external skill",
      limit: undefined,
      types: undefined,
      pathPrefix: undefined,
      includeHidden: false,
      workspaceRef,
    });
  });

  it("preserves an explicit hidden-document query", async () => {
    await knowledgeQuery({ query: "external skill", includeHidden: true }, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith(
      "knowledge_query",
      expect.objectContaining({ includeHidden: true }),
    );
  });

  it.each(["design", "plan", "memory", "skill", "reference"] as const)(
    "keeps the %s type root path stable when reading its directory config",
    async (type) => {
      mockedInvoke.mockResolvedValueOnce({
        kind: "directory",
        document: null,
        directory: {
          type,
          path: "",
        },
      });

      await knowledgeRead({
        kind: "directory",
        type,
        path: type,
      }, workspaceRef);

      expect(mockedInvoke).toHaveBeenCalledWith("knowledge_read", {
        request: {
          kind: "directory",
          path: type,
          type,
          part: "full",
          includeHistory: false,
        },
        workspaceRef,
      });
    },
  );

  it("keeps a type-named child directory relative to its explicit parent type", async () => {
    mockedInvoke.mockResolvedValueOnce({
      kind: "directory",
      document: null,
      directory: {
        type: "design",
        path: "memory",
      },
    });

    await knowledgeRead({
      kind: "directory",
      type: "design",
      path: "memory",
    }, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith(
      "knowledge_read",
      expect.objectContaining({
        request: expect.objectContaining({
          path: "design/memory",
          type: "design",
        }),
      }),
    );
  });

  it("keeps the memory type root path stable when saving its directory config", async () => {
    mockedInvoke.mockResolvedValueOnce({
      kind: "directory",
      type: "memory",
      path: "",
      directory: {
        type: "memory",
        path: "",
      },
    });

    await knowledgeEdit({
      kind: "directory",
      type: "memory",
      path: "memory",
      config: {
        summary: "Memory 根规则",
      },
    }, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_edit", {
      request: {
        kind: "directory",
        path: "memory",
        type: "memory",
        document: undefined,
        config: {
          summary: "Memory 根规则",
        },
      },
      workspaceRef,
    });
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
    }, workspaceRef);

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
      workspaceRef,
    });
  });

  it("sends the four-state AI edit mode for document metadata updates", async () => {
    mockedInvoke.mockResolvedValueOnce({
      kind: "document",
      type: "design",
      path: "combat.md",
    });

    await knowledgeEdit({
      kind: "document",
      type: "design",
      path: "combat.md",
      document: {
        readOnly: false,
        aiEditMode: "confirm",
      },
    }, workspaceRef);

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_edit", {
      request: {
        kind: "document",
        path: "design/combat.md",
        type: "design",
        document: {
          readOnly: false,
          aiEditMode: "confirm",
        },
        config: undefined,
      },
      workspaceRef,
    });
  });
});
