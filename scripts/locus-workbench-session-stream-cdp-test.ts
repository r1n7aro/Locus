import path from "node:path";
import process from "node:process";
import {
  CdpClient,
  findLocusWebViewTarget,
} from "./locus-webview2-stress-client";

const browserUrl = argument("--browser-url")?.replace(/\/$/, "") || "";
const devSourceUrl = argument("--dev-source-url")?.replace(/\/$/, "") || "http://localhost:14901";
const runtimeRootArg = argument("--runtime-root");
const verify = process.argv.includes("--verify");

if (runtimeRootArg) {
  const runtimeRoot = path.resolve(runtimeRootArg);
  if (!runtimeRoot.toLocaleLowerCase().startsWith("e:\\locustemp\\")) {
    throw new Error("--runtime-root must point to an isolated E:\\LocusTemp instance.");
  }
}

const target = await findLocusWebViewTarget(browserUrl, 60_000);
const client = await CdpClient.connect(target.webSocketDebuggerUrl!);

try {
  await client.send("Runtime.enable");
  const sessionId = `cdp-remount-gap-${Date.now()}`;
  const result = await client.evaluate<{
    sessionId: string;
    deliveredBeforeUnmount: number;
    replayedAfterRemount: number;
    replayedTypes: string[];
    passed: boolean;
  }>(`(async () => {
    const hub = await import('${devSourceUrl}/src/services/sessionStreamEventHub.ts');
    const sessionId = ${JSON.stringify(sessionId)};
    const consumer = {};
    let deliveredBeforeUnmount = 0;
    const unsubscribe = hub.subscribeSessionStreamEventConsumer(
      (dispatch) => dispatch.event.sessionId === sessionId ? consumer : null,
      () => { deliveredBeforeUnmount += 1; },
    );
    hub.bindSessionStreamEventConsumer(sessionId);
    hub.publishSessionStreamEvent({
      event: { type: 'runStart', sessionId, runId: 'run-before-remount' },
      source: { kind: 'legacy' },
    });
    unsubscribe();
    hub.publishSessionStreamEvent({
      event: {
        type: 'done',
        sessionId,
        runId: 'run-before-remount',
        messageId: 'assistant-during-remount',
        fullText: 'finished during remount',
      },
      source: { kind: 'legacy' },
    });
    const replayed = hub.bindSessionStreamEventConsumer(sessionId);
    return {
      sessionId,
      deliveredBeforeUnmount,
      replayedAfterRemount: replayed.length,
      replayedTypes: replayed.map((dispatch) => dispatch.event.type),
      passed: deliveredBeforeUnmount === 1
        && replayed.length === 1
        && replayed[0]?.event.type === 'done',
    };
  })()`);

  console.log(`LOCUS_WORKBENCH_SESSION_STREAM_CDP_JSON ${JSON.stringify({
    target: target.url,
    ...result,
  })}`);
  if (verify && !result.passed) process.exitCode = 1;
} finally {
  client.close();
}

function argument(name: string): string | null {
  const index = process.argv.indexOf(name);
  if (index >= 0) return process.argv[index + 1] ?? null;
  const inline = process.argv.find((value) => value.startsWith(`${name}=`));
  return inline?.slice(name.length + 1) ?? null;
}
