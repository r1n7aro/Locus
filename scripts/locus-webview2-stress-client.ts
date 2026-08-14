const DEBUG_PORT_START = 19222;
const DEBUG_PORT_ATTEMPTS = 25;

interface DevtoolsTarget {
  id: string;
  type: string;
  url: string;
  title?: string;
  webSocketDebuggerUrl?: string;
}

interface CdpMessage {
  id?: number;
  result?: unknown;
  error?: { message?: string };
}

export async function findLocusWebViewTarget(
  requestedBrowserUrl: string,
  timeoutMs: number,
): Promise<DevtoolsTarget> {
  const deadline = Date.now() + timeoutMs;
  const browserUrls = requestedBrowserUrl
    ? [requestedBrowserUrl.replace(/\/$/, "")]
    : Array.from(
        { length: DEBUG_PORT_ATTEMPTS },
        (_, offset) => `http://127.0.0.1:${DEBUG_PORT_START + offset}`,
      );

  while (Date.now() < deadline) {
    for (const browserUrl of browserUrls) {
      const targets = await readTargets(browserUrl);
      const target = targets.find((item) => (
        item.type === "page"
        && !!item.webSocketDebuggerUrl
        && /^http:\/\/localhost:\d+\/$/.test(item.url)
      ));
      if (target) return target;
    }
    await sleep(250);
  }

  throw new Error(
    requestedBrowserUrl
      ? `No Locus page found at ${requestedBrowserUrl}.`
      : "No Locus WebView2 page found on ports 19222-19246. Start an isolated locus:test:app instance first.",
  );
}

async function readTargets(browserUrl: string): Promise<DevtoolsTarget[]> {
  try {
    const response = await fetch(`${browserUrl}/json/list`, {
      signal: AbortSignal.timeout(350),
    });
    if (!response.ok) return [];
    return await response.json() as DevtoolsTarget[];
  } catch {
    return [];
  }
}

export function sleep(ms: number) {
  return new Promise<void>((resolve) => setTimeout(resolve, ms));
}

export class CdpClient {
  private nextId = 1;
  private pending = new Map<number, {
    resolve: (value: unknown) => void;
    reject: (reason: Error) => void;
  }>();

  private constructor(private readonly socket: WebSocket) {
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data)) as CdpMessage;
      if (!message.id) return;
      const request = this.pending.get(message.id);
      if (!request) return;
      this.pending.delete(message.id);
      if (message.error) {
        request.reject(new Error(message.error.message || "CDP request failed"));
      } else {
        request.resolve(message.result);
      }
    });
    socket.addEventListener("close", () => {
      for (const request of this.pending.values()) {
        request.reject(new Error("WebView2 DevTools connection closed."));
      }
      this.pending.clear();
    });
  }

  static connect(url: string): Promise<CdpClient> {
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(url);
      const onError = () => reject(new Error(`Failed to connect to ${url}`));
      socket.addEventListener("error", onError, { once: true });
      socket.addEventListener("open", () => {
        socket.removeEventListener("error", onError);
        resolve(new CdpClient(socket));
      }, { once: true });
    });
  }

  send(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate<T = Record<string, unknown>>(expression: string): Promise<T> {
    const response = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    }) as {
      result?: { value?: T; description?: string };
      exceptionDetails?: { text?: string; exception?: { description?: string } };
    };
    if (response.exceptionDetails) {
      throw new Error(
        response.exceptionDetails.exception?.description
        || response.exceptionDetails.text
        || "Browser evaluation failed.",
      );
    }
    return response.result?.value as T;
  }

  close() {
    this.socket.close();
  }
}
