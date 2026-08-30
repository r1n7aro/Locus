import { describe, expect, it, vi } from "vitest";
// @ts-expect-error Node runtime helper is JavaScript by design.
import * as cdpDiscovery from "../../scripts/locus-cdp-discovery.mjs";

const {
  browserUrlFromArgs,
  discoverLocusBrowserUrl,
  resolveLocusBrowserArgs,
  withBrowserUrlArg,
} = cdpDiscovery;

function jsonResponse(value: unknown, ok = true) {
  return {
    ok,
    json: async () => value,
  } as Response;
}

function locusTarget(port: number) {
  return {
    id: "main",
    type: "page",
    title: "Locus",
    url: "http://tauri.localhost/",
    webSocketDebuggerUrl: `ws://127.0.0.1:${port}/devtools/page/main`,
  };
}

describe("Locus CDP discovery", () => {
  it("falls back from a stale registered port to the live runtime port", async () => {
    const fetchImpl = vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url === "http://127.0.0.1:19223/json/list") {
        return jsonResponse([locusTarget(19223)]);
      }
      return jsonResponse([], false);
    });

    await expect(discoverLocusBrowserUrl({
      preferredUrl: "http://127.0.0.1:19222",
      startPort: 19222,
      attempts: 3,
      fetchImpl,
    })).resolves.toBe("http://127.0.0.1:19223");

    expect(fetchImpl.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:19222/json/list",
      "http://127.0.0.1:19223/json/list",
      "http://127.0.0.1:19224/json/list",
    ]);
  });

  it("keeps a valid configured Locus target ahead of fallback candidates", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse([locusTarget(19300)]));

    await expect(discoverLocusBrowserUrl({
      preferredUrl: "http://127.0.0.1:19300/",
      startPort: 19222,
      attempts: 2,
      fetchImpl,
    })).resolves.toBe("http://127.0.0.1:19300");

    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it("rewrites both supported browserUrl argument forms without duplicates", async () => {
    expect(browserUrlFromArgs(["--browserUrl", "http://127.0.0.1:19222"])).toBe(
      "http://127.0.0.1:19222",
    );
    expect(withBrowserUrlArg(
      ["--browserUrl=http://127.0.0.1:19222", "--no-usage-statistics"],
      "http://127.0.0.1:19223",
    )).toEqual([
      "--browserUrl=http://127.0.0.1:19223",
      "--no-usage-statistics",
    ]);

    const fetchImpl = vi.fn(async (input: string | URL | Request) => (
      String(input) === "http://127.0.0.1:19223/json/list"
        ? jsonResponse([locusTarget(19223)])
        : jsonResponse([], false)
    ));
    await expect(resolveLocusBrowserArgs(
      ["--browserUrl", "http://127.0.0.1:19222", "--no-usage-statistics"],
      { fetchImpl, startPort: 19222, attempts: 2 },
    )).resolves.toEqual({
      args: ["--browserUrl", "http://127.0.0.1:19223", "--no-usage-statistics"],
      browserUrl: "http://127.0.0.1:19223",
      preferredUrl: "http://127.0.0.1:19222",
    });
  });

  it("refuses to guess when several fallback Locus instances are live", async () => {
    const fetchImpl = vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url === "http://127.0.0.1:19223/json/list") {
        return jsonResponse([locusTarget(19223)]);
      }
      if (url === "http://127.0.0.1:19224/json/list") {
        return jsonResponse([locusTarget(19224)]);
      }
      return jsonResponse([], false);
    });

    await expect(discoverLocusBrowserUrl({
      preferredUrl: "http://127.0.0.1:19222",
      startPort: 19222,
      attempts: 3,
      fetchImpl,
    })).rejects.toThrow("Multiple Locus CDP targets are available");
  });
});
