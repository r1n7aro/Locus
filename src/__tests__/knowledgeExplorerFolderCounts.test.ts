import { describe, expect, it } from "vitest";
import type { ExplorerNode } from "../composables/useKnowledgeState";
import { buildFolderDisplayStats } from "../components/knowledge/knowledgeExplorerFolderCounts";

describe("knowledge explorer folder counts", () => {
  it("counts descendant documents instead of only direct child nodes", () => {
    const tree: ExplorerNode[] = [
      {
        kind: "folder",
        path: "reference/unity-official-docs",
        relativePath: "unity-official-docs",
        name: "unity-official-docs",
        depth: 1,
        children: [
          {
            kind: "document",
            path: "reference/unity-official-docs/index.md",
            name: "index.md",
            depth: 2,
            document: {
              id: "doc-index",
              path: "unity-official-docs/index.md",
              title: "Unity Index",
              type: "reference",
              injectMode: "none",
              hasSummary: true,
              hasBodyContent: true,
              summaryEnabled: true,
              commandEnabled: false,
              readOnly: true,
              aiMaintained: false,
              explicitMaintenanceRules: false,
              createdAt: 1,
              updatedAt: 1,
            },
          },
          {
            kind: "folder",
            path: "reference/unity-official-docs/script-reference",
            relativePath: "unity-official-docs/script-reference",
            name: "script-reference",
            depth: 2,
            children: [
              {
                kind: "folder",
                path: "reference/unity-official-docs/script-reference/Advertisements",
                relativePath: "unity-official-docs/script-reference/Advertisements",
                name: "Advertisements",
                depth: 3,
                children: [
                  {
                    kind: "folder",
                    path: "reference/unity-official-docs/script-reference/Advertisements/AdvertisementSettings",
                    relativePath: "unity-official-docs/script-reference/Advertisements/AdvertisementSettings",
                    name: "AdvertisementSettings",
                    depth: 4,
                    children: [
                      {
                        kind: "document",
                        path: "reference/unity-official-docs/script-reference/Advertisements/AdvertisementSettings/Advertisements.AdvertisementSettings.md",
                        name: "Advertisements.AdvertisementSettings.md",
                        depth: 5,
                        document: {
                          id: "doc-ad-settings",
                          path: "unity-official-docs/script-reference/Advertisements/AdvertisementSettings/Advertisements.AdvertisementSettings.md",
                          title: "AdvertisementSettings",
                          type: "reference",
                          injectMode: "none",
                          hasSummary: true,
                          hasBodyContent: true,
                          summaryEnabled: true,
                          commandEnabled: false,
                          readOnly: true,
                          aiMaintained: false,
                          explicitMaintenanceRules: false,
                          createdAt: 1,
                          updatedAt: 1,
                        },
                      },
                      {
                        kind: "document",
                        path: "reference/unity-official-docs/script-reference/Advertisements/AdvertisementSettings/Advertisements.AdvertisementSettings.GetGameId.md",
                        name: "Advertisements.AdvertisementSettings.GetGameId.md",
                        depth: 5,
                        document: {
                          id: "doc-ad-game-id",
                          path: "unity-official-docs/script-reference/Advertisements/AdvertisementSettings/Advertisements.AdvertisementSettings.GetGameId.md",
                          title: "AdvertisementSettings.GetGameId",
                          type: "reference",
                          injectMode: "none",
                          hasSummary: true,
                          hasBodyContent: true,
                          summaryEnabled: true,
                          commandEnabled: false,
                          readOnly: true,
                          aiMaintained: false,
                          explicitMaintenanceRules: false,
                          createdAt: 1,
                          updatedAt: 1,
                        },
                      },
                    ],
                  },
                ],
              },
            ],
          },
        ],
      },
    ];

    const stats = buildFolderDisplayStats(tree);

    expect(stats.get("reference/unity-official-docs")).toEqual({
      directChildCount: 2,
      descendantDocumentCount: 3,
    });
    expect(stats.get("reference/unity-official-docs/script-reference")).toEqual({
      directChildCount: 1,
      descendantDocumentCount: 2,
    });
    expect(stats.get("reference/unity-official-docs/script-reference/Advertisements")).toEqual({
      directChildCount: 1,
      descendantDocumentCount: 2,
    });
  });

  it("prefers cached managed folder stats when provided", () => {
    const tree: ExplorerNode[] = [
      {
        kind: "folder",
        path: "reference/unity-official-docs",
        relativePath: "unity-official-docs",
        name: "unity-official-docs",
        depth: 1,
        children: [
          {
            kind: "folder",
            path: "reference/unity-official-docs/manual",
            relativePath: "unity-official-docs/manual",
            name: "manual",
            depth: 2,
            children: [],
          },
        ],
      },
    ];

    const stats = buildFolderDisplayStats(tree, {
      "reference/unity-official-docs": {
        directChildCount: 2,
        descendantDocumentCount: 2429,
      },
      "reference/unity-official-docs/manual": {
        directChildCount: 2162,
        descendantDocumentCount: 2162,
      },
    });

    expect(stats.get("reference/unity-official-docs")).toEqual({
      directChildCount: 2,
      descendantDocumentCount: 2429,
    });
    expect(stats.get("reference/unity-official-docs/manual")).toEqual({
      directChildCount: 2162,
      descendantDocumentCount: 2162,
    });
  });
});
