// Run against an isolated dev instance with an output path in its runtime root:
// bun run scripts/locus-console-export-cdp-test.ts --browser-url http://127.0.0.1:<port> --output <absolute-path.log>
import assert from "node:assert/strict";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { CdpClient, findLocusWebViewTarget } from "./locus-webview2-stress-client";

function option(name: string) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}
const browserUrl = option("--browser-url");
const outputPath = option("--output");
assert(browserUrl && outputPath && path.isAbsolute(outputPath), "Pass an isolated --browser-url and absolute --output path.");
await fetch(`${browserUrl}/json/version`).then((response) => response.json());
const target = await findLocusWebViewTarget(browserUrl, 10_000);
assert(/^http:\/\/(localhost|127\.0\.0\.1):\d+\/$/.test(target.url), "Refusing to inject logs into a published instance.");
const cdp = await CdpClient.connect(target.webSocketDebuggerUrl!);
const runPrefix = `complete-export-test-${crypto.randomUUID()}`;

try {
  const result = await cdp.evaluate<{ displayedBeforeClear: number; displayedAfterExport: number; savedPath: string }>(`(async () => {
    const service = await import('/src/services/debugConsole.ts');
    const prefix = ${JSON.stringify(`[${runPrefix}]`)};
    console.info(prefix, '完整正文开始' + '文'.repeat(700000) + '完整正文结束');
    for (let i = 0; i < 2500; i++) console.info(prefix + ' record-' + i);
    const displayedBeforeClear = service.getDebugConsoleSnapshot().length;
    await service.clearDebugConsole();
    const savedPath = await service.saveDebugConsoleLogExport(${JSON.stringify(outputPath)});
    return {displayedBeforeClear, displayedAfterExport: service.getDebugConsoleSnapshot().length, savedPath};
  })()`);
  assert(result.displayedBeforeClear <= 2000);
  assert.equal(result.displayedAfterExport, 0, "Export reloaded the full archive into the display buffer");
  const content = await readFile(result.savedPath, "utf8");
  assert(content.includes(`[${runPrefix}] 完整正文开始` + "文".repeat(700000) + "完整正文结束"), "Full body was truncated");
  const records = [...content.matchAll(new RegExp(`\\[${runPrefix}\\] record-(\\d+)\\r?\\n`, "g"))].map((match) => Number(match[1]));
  assert.deepEqual(records, Array.from({ length: 2500 }, (_, index) => index), "Records were lost or duplicated before export");
  assert(!content.includes("…(truncated"));
  console.log(JSON.stringify({ passed: true, ...result, exportedRecords: records.length + 1, exportedBytes: (await stat(result.savedPath)).size }));
} finally {
  cdp.close();
}
