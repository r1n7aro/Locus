import { CdpClient } from "../locus-webview2-stress-client";
import { writeFile } from "node:fs/promises";

export async function connect(browserUrl: string, targetId: string) {
  if (!/^http:\/\/127\.0\.0\.1:\d+$/.test(browserUrl) || !targetId) throw new Error("An explicit local browser URL and target ID are required.");
  const version = await (await fetch(`${browserUrl}/json/version`)).json();
  const targets = await (await fetch(`${browserUrl}/json/list`)).json();
  const target = targets.find((entry: { id: string; type: string }) => entry.id === targetId && entry.type === "page");
  if (!target) throw new Error("Requested test page no longer exists.");
  return { client: await CdpClient.connect(target.webSocketDebuggerUrl), version, target };
}

if (import.meta.main) {
  const [browserUrl, targetId, action, value] = process.argv.slice(2);
  const { client } = await connect(browserUrl!, targetId!);
  try {
    if (action === "navigate") {
      if (!/^http:\/\/(localhost|127\.0\.0\.1):1492[12]\//.test(value!)) throw new Error("Only the validation host may be navigated to.");
      console.log(await client.send("Page.navigate", { url: value }));
    } else if (action === "screenshot") {
      const result = await client.send("Page.captureScreenshot", { format: "png" }) as { data: string };
      await writeFile(value!, Buffer.from(result.data, "base64"));
      console.log(value);
    } else if (action === "eval") console.log(JSON.stringify(await client.evaluate(value!), null, 2));
    else throw new Error("Expected navigate, screenshot or eval.");
  } finally { client.close(); }
}
