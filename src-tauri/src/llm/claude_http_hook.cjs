// Locus debug hook for the Claude Code CLI subprocess.
//
// Injected via NODE_OPTIONS=--require <abs path to this file> when Locus runs the
// Claude Code CLI backend with debug mode enabled. Patches http/https/fetch
// inside the CLI's JS runtime so every POST to api.anthropic.com/v1/messages is
// dumped to LOCUS_DEBUG_DIR as a `.http` file. Failures here must NEVER break the
// real request flow — every observable side effect is wrapped in try/catch.
"use strict";

const fs = require("fs");
const path = require("path");
const http = require("http");
const https = require("https");

const debugDir =
  process.env.LOCUS_DEBUG_DIR && process.env.LOCUS_DEBUG_DIR.length > 0
    ? process.env.LOCUS_DEBUG_DIR
    : path.join(process.cwd(), "debug", "llm");

try {
  fs.mkdirSync(debugDir, { recursive: true });
} catch (e) {
  // best effort
}

// Sentinel: write a marker file as soon as this script is loaded so the Rust side
// can tell whether the runtime actually honored NODE_OPTIONS=--require. If this
// file does not appear, the hook was never executed (likely bun ignoring --require
// or NODE_OPTIONS not propagated).
try {
  const runtime =
    typeof Bun !== "undefined"
      ? `bun ${Bun.version}`
      : typeof process !== "undefined" && process.versions && process.versions.node
        ? `node ${process.versions.node}`
        : "unknown";
  const sentinel = path.join(debugDir, "_locus_hook_loaded.txt");
  const line = `${new Date().toISOString()} pid=${process.pid} runtime=${runtime} cwd=${process.cwd()}\n`;
  fs.appendFileSync(sentinel, line);
  // Also surface to stderr — captured live by Rust in debug mode.
  try {
    process.stderr.write(`[locus-hook] loaded pid=${process.pid} runtime=${runtime}\n`);
  } catch (e) {
    // ignore
  }
} catch (e) {
  // sentinel must never break the run
}

let seq = 0;

function nextOutFile() {
  seq += 1;
  const now = new Date();
  const pad = (n, w) => String(n).padStart(w, "0");
  const ts =
    now.getFullYear().toString() +
    pad(now.getMonth() + 1, 2) +
    pad(now.getDate(), 2) +
    "_" +
    pad(now.getHours(), 2) +
    pad(now.getMinutes(), 2) +
    pad(now.getSeconds(), 2) +
    "." +
    pad(now.getMilliseconds(), 3);
  return path.join(
    debugDir,
    `${ts}_${pad(seq, 4)}_claude_code_real_pid${process.pid}.http`
  );
}

function toBuffer(chunk, encoding) {
  if (chunk == null) return Buffer.alloc(0);
  if (Buffer.isBuffer(chunk)) return Buffer.from(chunk);
  if (chunk instanceof Uint8Array) return Buffer.from(chunk);
  return Buffer.from(String(chunk), encoding);
}

function normalizeRequestArgs(defaultProtocol, args) {
  let urlObj = null;
  let options = {};

  if (args.length > 0) {
    const first = args[0];
    if (typeof first === "string" || first instanceof URL) {
      try {
        urlObj = new URL(first.toString());
      } catch (e) {
        urlObj = null;
      }
      if (args[1] && typeof args[1] === "object" && !(args[1] instanceof Function)) {
        options = { ...args[1] };
      }
    } else if (first && typeof first === "object") {
      options = { ...first };
    }
  }

  const protocol = (options.protocol || urlObj?.protocol || defaultProtocol).replace(/:$/, "");
  const hostname = options.hostname || options.host || urlObj?.hostname || "";
  const port = options.port || urlObj?.port || "";
  const method = (options.method || "GET").toUpperCase();
  const pathValue =
    options.path || ((urlObj?.pathname || "/") + (urlObj?.search || "")) || "/";
  const headers = { ...(options.headers || {}) };

  return { protocol, hostname, port, method, path: pathValue, headers };
}

function shouldCapture(meta) {
  return (
    meta.hostname === "api.anthropic.com" &&
    typeof meta.path === "string" &&
    meta.path.startsWith("/v1/messages")
  );
}

const REDACTED_HEADER_VALUE = "<redacted>";

function shouldRedactHeader(key) {
  const normalized = String(key || "").toLowerCase();
  return (
    normalized === "authorization" ||
    normalized === "proxy-authorization" ||
    normalized === "x-api-key" ||
    normalized === "api-key" ||
    normalized === "apikey" ||
    normalized === "cookie" ||
    normalized === "set-cookie" ||
    normalized.startsWith("anthropic-") ||
    normalized.includes("token") ||
    normalized.includes("secret") ||
    normalized.includes("credential") ||
    normalized.includes("password")
  );
}

function formatHeaderValue(key, value) {
  return shouldRedactHeader(key) ? REDACTED_HEADER_VALUE : value;
}

function formatHeaders(headers) {
  const lines = [];
  for (const [key, value] of Object.entries(headers || {})) {
    if (Array.isArray(value)) {
      for (const item of value) {
        lines.push(`${key}: ${formatHeaderValue(key, item)}`);
      }
    } else {
      lines.push(`${key}: ${formatHeaderValue(key, value)}`);
    }
  }
  return lines.join("\r\n");
}

function writeCapture(meta, headers, body) {
  try {
    const requestLine = `${meta.method} ${meta.path} HTTP/1.1\r\n`;
    const headerText = formatHeaders(headers);
    const rawText = requestLine + headerText + "\r\n\r\n" + body.toString("utf8");
    fs.writeFileSync(nextOutFile(), rawText, "utf8");
  } catch (e) {
    // swallow — debug logging must not break the request
  }
}

function wrapRequest(moduleObj, defaultProtocol) {
  const originalRequest = moduleObj.request;
  if (typeof originalRequest !== "function") return;

  moduleObj.request = function patchedRequest(...args) {
    const meta = normalizeRequestArgs(defaultProtocol, args);
    const req = originalRequest.apply(this, args);
    if (!shouldCapture(meta)) return req;

    const chunks = [];
    const originalWrite = req.write.bind(req);
    const originalEnd = req.end.bind(req);

    req.write = function patchedWrite(chunk, encoding, callback) {
      if (chunk != null) chunks.push(toBuffer(chunk, encoding));
      return originalWrite(chunk, encoding, callback);
    };

    req.end = function patchedEnd(chunk, encoding, callback) {
      if (chunk != null) chunks.push(toBuffer(chunk, encoding));
      const finalHeaders =
        typeof req.getHeaders === "function" ? req.getHeaders() : meta.headers;
      writeCapture(meta, finalHeaders, Buffer.concat(chunks));
      return originalEnd(chunk, encoding, callback);
    };

    return req;
  };
}

wrapRequest(http, "http:");
wrapRequest(https, "https:");

if (typeof globalThis.fetch === "function") {
  const originalFetch = globalThis.fetch.bind(globalThis);

  globalThis.fetch = async function patchedFetch(input, init) {
    let url = null;
    try {
      url =
        typeof input === "string"
          ? new URL(input)
          : input instanceof URL
            ? input
            : input && typeof input.url === "string"
              ? new URL(input.url)
              : null;
    } catch (e) {
      url = null;
    }

    if (url && url.hostname === "api.anthropic.com" && url.pathname.startsWith("/v1/messages")) {
      const method =
        (init && init.method) ||
        (input && typeof input === "object" && "method" in input ? input.method : "GET");

      const headersSource =
        (init && init.headers) ||
        (input && typeof input === "object" && "headers" in input ? input.headers : undefined);

      const bodySource =
        (init && init.body) ||
        (input && typeof input === "object" && "body" in input ? input.body : undefined);

      const headers = {};
      if (headersSource && typeof headersSource.forEach === "function") {
        headersSource.forEach((value, key) => {
          headers[key] = value;
        });
      } else if (Array.isArray(headersSource)) {
        for (const [key, value] of headersSource) {
          headers[key] = value;
        }
      } else if (headersSource && typeof headersSource === "object") {
        Object.assign(headers, headersSource);
      }

      let bodyText = "";
      if (typeof bodySource === "string") {
        bodyText = bodySource;
      } else if (bodySource instanceof URLSearchParams) {
        bodyText = bodySource.toString();
      } else if (bodySource instanceof Uint8Array || Buffer.isBuffer(bodySource)) {
        bodyText = Buffer.from(bodySource).toString("utf8");
      } else if (bodySource != null) {
        bodyText = String(bodySource);
      }

      try {
        const requestLine = `${String(method || "GET").toUpperCase()} ${url.pathname}${url.search} HTTP/1.1\r\n`;
        const headerText = formatHeaders(headers);
        const rawText = requestLine + headerText + "\r\n\r\n" + bodyText;
        fs.writeFileSync(nextOutFile(), rawText, "utf8");
      } catch (e) {
        // swallow
      }
    }

    return originalFetch(input, init);
  };
}
