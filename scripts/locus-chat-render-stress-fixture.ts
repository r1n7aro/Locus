export interface ChatRenderStressToolSpec {
  name: string;
  arguments: Record<string, unknown>;
  output: string;
}

export interface ChatRenderStressFixture {
  toolSpecs: ChatRenderStressToolSpec[];
  mixedPieces: string[];
  longOutputLines: number;
}

export const CHAT_RENDER_STRESS_LONG_OUTPUT_LINES = 160;

/**
 * Shared deterministic data for both the WebView stress driver and the
 * headless Vitest reproduction. Keep browser control out of this module so it
 * can be imported without starting Locus or opening a DevTools connection.
 */
export function createChatRenderStressFixture(): ChatRenderStressFixture {
  const longOutput = Array.from({ length: CHAT_RENDER_STRESS_LONG_OUTPUT_LINES }, (_, index) =>
    `${String(index + 1).padStart(3, "0")}  synthetic build output: processing Assets/Stress/RenderProbe${String(index + 1).padStart(3, "0")}.cs`
  ).join("\n");

  return {
    longOutputLines: CHAT_RENDER_STRESS_LONG_OUTPUT_LINES,
    toolSpecs: [
      { name: "list", arguments: { path: "Assets", depth: 3, include_files: true }, output: "Assets/\n  Scripts/\n  Scenes/" },
      { name: "grep", arguments: { pattern: "MonoBehaviour", path: "Assets", include: "*.cs" }, output: "Assets/Scripts/Player.cs:12: public sealed class Player : MonoBehaviour" },
      { name: "read", arguments: { filePath: "Assets/Scripts/Player.cs", offset: 1, limit: 80 }, output: "1\tusing UnityEngine;\n2\tpublic sealed class Player : MonoBehaviour {}" },
      { name: "code_diagnostics", arguments: { filePath: "Assets/Scripts/Player.cs" }, output: "No diagnostics found." },
      { name: "list", arguments: { path: "Assets/Scenes", depth: 2, include_files: true }, output: "Assets/Scenes/\n  Main.unity\n  Bootstrap.unity" },
      { name: "bash", arguments: { command: "simulate-long-build --verbose", timeout: 120000 }, output: longOutput },
    ],
    mixedPieces: [
      "普通短行，用于建立正文基线。\n\n",
      "## ",
      "动态标题会切换字号、行高和上下边距\n\n",
      "这是一段会自动换行的长文本，用于逼近内容列的换行边界。".repeat(6) + "\n\n",
      "- ",
      "列表项会引入 marker、缩进与 4px 行间距。\n",
      "  - 嵌套列表继续改变可用行宽与换行位置。\n\n",
      "> ",
      "引用块会增加内边距、边框和独立背景，从而改变块高度。\n\n",
      "```ts\n",
      "const renderProbe = { streaming: true, lineHeight: \"mixed\" };\n",
      "console.log(renderProbe);\n",
      "```\n\n",
      "| 类型 | 高度 |\n",
      "| --- | ---: |\n",
      "| paragraph | 1.68 |\n\n",
    ],
  };
}
