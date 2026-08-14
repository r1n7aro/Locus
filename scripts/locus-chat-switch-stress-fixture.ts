export interface PersistedStressToolCall {
  id: string;
  name: string;
  arguments: string;
  order?: number;
  outcome?: "done" | "error" | "interrupted";
  recordedOutput?: string;
  nestedToolCalls?: PersistedStressToolCall[];
}

export interface PersistedStressMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  createdAt: number;
  toolCalls?: PersistedStressToolCall[];
  thinkingContent?: string;
  thinkingDuration?: number;
  metadata?: Record<string, unknown>;
}

export interface LongSessionFixture {
  key: string;
  title: string;
  messages: PersistedStressMessage[];
}

const SESSION_TURNS = 30;

export function createLongSessionFixtures(baseTimestamp = 1_750_000_000): LongSessionFixture[] {
  return [
    buildFixture("unity-properties", "Stress 1 · Unity Property", baseTimestamp, unityPropertyContent),
    buildFixture("markdown-tables", "Stress 2 · Markdown Tables", baseTimestamp + 10_000, markdownTableContent),
    buildFixture("edit-diffs", "Stress 3 · Edit Diffs", baseTimestamp + 20_000, editDiffContent),
    buildFixture("nested-tools", "Stress 4 · Nested Tools", baseTimestamp + 30_000, nestedToolContent),
    buildFixture("mixed-layout", "Stress 5 · Mixed Layout", baseTimestamp + 40_000, mixedLayoutContent),
  ];
}

function buildFixture(
  key: string,
  title: string,
  createdAt: number,
  contentFactory: (turn: number) => string,
): LongSessionFixture {
  const messages: PersistedStressMessage[] = [];
  for (let turn = 0; turn < SESSION_TURNS; turn += 1) {
    const runId = `${key}-run-${turn}`;
    const userId = `${key}-user-${turn}`;
    const assistantId = `${key}-assistant-${turn}`;
    const toolCalls = buildToolCalls(key, turn);
    const content = contentFactory(turn);
    messages.push({
      id: userId,
      role: "user",
      content: `第 ${turn + 1} 轮：检查 ${title} 的复杂布局与滚动锚点。`,
      createdAt: createdAt + turn * 2,
    });
    messages.push({
      id: assistantId,
      role: "assistant",
      content,
      createdAt: createdAt + turn * 2 + 1,
      toolCalls,
      thinkingContent: `已分析第 ${turn + 1} 轮资源、序列化字段和工具结果。`,
      thinkingDuration: 2 + (turn % 5),
      metadata: {
        contentOrder: 100,
        thinkingOrder: 1,
        renderParts: [
          {
            kind: "thinking",
            id: `${assistantId}-thinking`,
            order: { runId, seq: 1 },
            content: `已分析第 ${turn + 1} 轮资源、序列化字段和工具结果。`,
            active: false,
            duration: 2 + (turn % 5),
          },
          ...toolCalls.map((toolCall, index) => ({
            kind: "toolCall",
            id: `${assistantId}-tool-part-${index}`,
            order: { runId, seq: 10 + index },
            toolCall,
          })),
          {
            kind: "text",
            id: `${assistantId}-text`,
            order: { runId, seq: 100 },
            content,
          },
        ],
      },
    });
  }
  return { key, title, messages };
}

function buildToolCalls(key: string, turn: number): PersistedStressToolCall[] {
  const prefix = `${key}-tool-${turn}`;
  const filePath = `Assets/Stress/${key}/RenderProbe${String(turn + 1).padStart(2, "0")}.cs`;
  const readOutput = Array.from({ length: 20 + (turn % 8) }, (_, index) =>
    `${String(index + 1).padStart(3, "0")}\tpublic float Probe${index} = ${turn + index}.0f;`
  ).join("\n");
  const oldString = Array.from({ length: 12 }, (_, index) =>
    `        values[${index}] = ${turn + index};`
  ).join("\n");
  const newString = Array.from({ length: 14 }, (_, index) =>
    `        values[${index}] = math.max(0, ${turn + index});`
  ).join("\n");

  if (key === "nested-tools") {
    return [{
      id: `${prefix}-task`,
      name: "task",
      arguments: JSON.stringify({ prompt: `分析 ${filePath} 的序列化布局与运行时依赖。` }),
      order: 10,
      outcome: "done",
      recordedOutput: `子任务完成：${filePath}\n\n- 收集字段\n- 校验引用\n- 生成变更建议`,
      nestedToolCalls: [
        {
          id: `${prefix}-nested-list`,
          name: "list",
          arguments: JSON.stringify({ path: `Assets/Stress/${key}`, depth: 3 }),
          order: 11,
          outcome: "done",
          recordedOutput: `${filePath}\nAssets/Stress/${key}/Probe.prefab`,
        },
        {
          id: `${prefix}-nested-read`,
          name: "read",
          arguments: JSON.stringify({ filePath, offset: 1, limit: 80 }),
          order: 12,
          outcome: "done",
          recordedOutput: readOutput,
        },
        {
          id: `${prefix}-nested-edit`,
          name: "edit",
          arguments: JSON.stringify({ filePath, oldString, newString }),
          order: 13,
          outcome: turn % 4 === 0 ? "error" : "done",
          recordedOutput: turn % 4 === 0
            ? "Found multiple matches for oldString. Provide more surrounding context."
            : `Edited ${filePath} [lines:12-26]`,
        },
      ],
    }];
  }

  return [
    {
      id: `${prefix}-read`,
      name: "read",
      arguments: JSON.stringify({ filePath, offset: 1, limit: 80 }),
      order: 10,
      outcome: "done",
      recordedOutput: readOutput,
    },
    {
      id: `${prefix}-edit`,
      name: "edit",
      arguments: JSON.stringify({ filePath, oldString, newString }),
      order: 11,
      outcome: turn % 7 === 0 ? "error" : "done",
      recordedOutput: turn % 7 === 0
        ? "Found multiple matches for oldString at lines 42, 88."
        : `Edited ${filePath} [lines:10-24]`,
    },
    {
      id: `${prefix}-diagnostics`,
      name: "code_diagnostics",
      arguments: JSON.stringify({ filePath }),
      order: 12,
      outcome: "done",
      recordedOutput: turn % 6 === 0
        ? `${filePath}:18: warning CS0414: synthetic field is assigned but never used`
        : "No diagnostics found.",
    },
  ];
}

function unityPropertyContent(turn: number) {
  const scene = `Assets/Stress/Scenes/RenderStress${turn % 3}.unity`;
  const prefab = `Assets/Stress/Prefabs/StressActor${turn % 4}.prefab`;
  return [
    `## Unity Property 批次 ${turn + 1}`,
    "以下字段会挂载真实 Unity Property 复杂组件，并在无 Unity 连接时异步进入错误布局。",
    "```unity_property",
    `${scene}/Root/Player | Transform:m_LocalPosition.x`,
    `${scene}/Root/Player | Transform:m_LocalPosition.y`,
    `${scene}/Root/Player | Rigidbody:m_Mass`,
    `${prefab}/Visual | MeshRenderer:m_Enabled`,
    `${prefab}/Visual | MeshRenderer:m_Materials.Array.data[0]`,
    "```",
    "",
    `对象路径：\`${scene}/Root/Player\`，轮次 ${turn + 1}。`,
    "",
    propertySummaryTable(turn, 12),
  ].join("\n");
}

function markdownTableContent(turn: number) {
  return [
    `## 序列化快照 ${turn + 1}`,
    "复杂表格、引用、嵌套列表和跨行代码用于制造不同的块级高度。",
    "",
    propertySummaryTable(turn, 28),
    "",
    "> 当前快照包含动态列宽和较长路径，切换时应一次完成排版。",
    "",
    ...Array.from({ length: 8 }, (_, index) =>
      `- 层级 ${index + 1}\n  - Assets/Stress/Tables/Probe_${turn}_${index}.asset 的依赖项与序列化状态。`
    ),
  ].join("\n");
}

function editDiffContent(turn: number) {
  const code = Array.from({ length: 70 }, (_, index) =>
    `    public float Sample${String(index).padStart(2, "0")} => ${turn + index}.0f;`
  ).join("\n");
  return [
    `## 编辑结果 ${turn + 1}`,
    "工具详情包含多段 diff；正文继续包含长代码块以覆盖语法高亮布局。",
    "",
    "```csharp",
    "public sealed class RenderStressProbe : MonoBehaviour",
    "{",
    code,
    "}",
    "```",
    "",
    `完成第 ${turn + 1} 轮编辑检查。`,
  ].join("\n");
}

function nestedToolContent(turn: number) {
  return [
    `## 子任务轨迹 ${turn + 1}`,
    "该会话包含父工具、嵌套工具、长输出和错误状态。",
    "",
    ...Array.from({ length: 12 }, (_, index) =>
      `${index + 1}. 子任务步骤 ${index + 1}：读取资源、分析引用并核对第 ${turn + 1} 轮结果。`
    ),
    "",
    "```json",
    JSON.stringify({ turn, state: "complete", nested: { reads: 4, edits: 2, diagnostics: 1 } }, null, 2),
    "```",
  ].join("\n");
}

function mixedLayoutContent(turn: number) {
  return [
    `# 综合布局 ${turn + 1}`,
    "正文会混合标题、表格、代码、引用、分隔线和长段落。",
    "",
    "### 说明",
    `这是第 ${turn + 1} 轮很长的自动换行文本。`.repeat(20),
    "",
    "---",
    "",
    propertySummaryTable(turn, 16),
    "",
    "```ts",
    ...Array.from({ length: 35 }, (_, index) =>
      `const layoutProbe${index} = { turn: ${turn}, row: ${index}, stable: true };`
    ),
    "```",
    "",
    "> 切换完成后，底部锚点和视口位置应保持稳定。",
  ].join("\n");
}

function propertySummaryTable(turn: number, rows: number) {
  return [
    "| 字段 | 类型 | 当前值 | 资源路径 |",
    "| --- | --- | ---: | --- |",
    ...Array.from({ length: rows }, (_, index) =>
      `| property_${turn}_${index} | ${index % 3 === 0 ? "Vector3" : index % 3 === 1 ? "float" : "Object"} | ${turn * 10 + index} | Assets/Stress/Generated/VeryLongResourceName_${turn}_${index}.asset |`
    ),
  ].join("\n");
}
