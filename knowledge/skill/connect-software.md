---
id: kd_skill_builtin_connect_software
type: skill
path: connect-software.md
title: Connect External Software
injectMode: excerpt
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: both
commandTrigger: /connect
argumentHint: <software-or-service>
tools:
  - bash
  - mcp_reload
  - web_fetch
  - knowledge_create
  - knowledge_edit
createdAt: 1784246400000
updatedAt: 1784419200000
---

# Connect External Software

## Summary
Use when the user wants Locus to connect to, control, or automate external software, a service, or a web API — adding, installing, fixing, or reloading an MCP server (Blender, Figma, community `uvx`/`npx` servers, ...), setting up a CLI (`gh`, `adb`, `ffmpeg`, ...), or wiring up an HTTP API. Ignore Unity connection issues and Locus skill/plugin management.

## Content

You can research integration options, configure the connection (MCP server, CLI, or raw HTTP), verify it end to end, and persist what you learned — all without leaving the conversation. Do not tell the user a connection is unavailable before walking the channels below.

Command arguments: `/connect <software-or-service>` names the connection target (e.g. `/connect obs`, `/connect github`). When invoked without arguments, ask what the user wants to connect. The workflow is the same either way.

## Choose the channel

Survey what the target actually exposes (its docs/README, or the Research step below), then pick the cheapest channel that fits:

1. **The software hosts its own endpoint** (MCP or plain HTTP served by the app itself, e.g. Figma desktop) — use the MCP `http` transport or call the API directly. Nothing to install.
2. **An official or well-maintained CLI exists** (`gh`, `adb`, `docker`, `ffmpeg`, cloud CLIs) — call it through `bash`. No standing process, no config file, zero context cost; best for one-shot and batch operations.
3. **A community MCP server exists** (`uvx`/`npx` package) — configure it in Locus. Best when the session needs many structured tool calls against live software state (DCC control like Blender).
4. **Only a raw HTTP API exists** — call it with `bash` curl (auth via env vars) or `web_fetch` (public GETs).
5. **None of the above** — fall back to file-level integration (export/watch folders, editing the software's own project files), or tell the user plainly what is missing.

When both a CLI and an MCP server exist, prefer official over community, then the more actively maintained one. An MCP server buys typed tools and a persistent connection; a CLI buys zero setup cost in future sessions.

## Research connection options

When you do not already know the concrete package or endpoint, research instead of guessing — `web_fetch` handles JSON APIs directly:

- Official MCP registry: `https://registry.modelcontextprotocol.io/v0/servers?search=<software>` → `{"servers": [...]}`. Coverage is partial — many popular servers never registered there, so always cross-check GitHub.
- GitHub: `https://api.github.com/search/repositories?q=<software>+mcp&sort=stars&per_page=5` → judge by `stargazers_count` and `pushed_at`. Works for CLIs too (`q=<software>+cli`).
- npm: `https://registry.npmjs.org/-/v1/search?text=<software>%20mcp&size=5` → `objects[].package.{name,date}`.
- The software's official docs, on pages named "API", "CLI", "automation", "scripting", or "MCP".

Prefer vendor/official over community, recently pushed over stale, and a README with a complete config sample over bare code. Fetch the winning repo's README for the exact command/args/env before configuring anything. If these endpoints are unreachable, ask the user for the package name or a docs link.

## Channel: MCP server

### How MCP servers work in Locus

- Locus stores MCP servers globally in `%APPDATA%\locus\mcp_servers.json` (all workspaces share it). The Settings → MCP Servers page reads the same file, so your edits show up there too.
- `mcp_reload` reconciles live connections against the file and reports per server: connected or failed (with the server's stderr), plus the wire name of every available tool. **The report is your connection test.**
- After a successful reload the tools become available in this conversation as `mcp__<server-id>__<tool>` (e.g. `mcp__blender__get_scene_info`). They are lazy-loaded by default: call one directly if it is already visible to you, otherwise load it first via `tool_load` / `tool_search` with the exact wire name from the reload report.
- Most DCC servers are two-hop: Locus spawns a bridge process (e.g. `uvx blender-mcp`), and the bridge connects to a plugin *inside* the target software. `mcp_reload` showing "connected" proves the first hop only — if tool calls then fail with a connection error, the software-side plugin is not running; guide the user to start it and simply retry the tool (no reload needed).
- File tools (`read`/`write`/`edit`) accept absolute paths anywhere on disk, including `%APPDATA%` — prefer them over `bash` for every config-file step (no shell quoting/escaping pitfalls). Resolve the concrete path once with `echo "$APPDATA"` if you do not know it.

### Config file schema

```json
{
  "servers": [
    {
      "id": "blender",
      "name": "Blender",
      "transport": "stdio",
      "command": "uvx",
      "args": ["blender-mcp"],
      "env": {},
      "cwd": "",
      "url": "",
      "headers": {},
      "enabled": true,
      "callTimeoutMs": 240000,
      "autoRestart": false,
      "loadMode": "lazy",
      "toolAllowlist": [],
      "toolDenylist": []
    }
  ]
}
```

- `id`: unique slug, lowercase `[a-z0-9_-]` only (it becomes part of tool names).
- `transport`: `"stdio"` (local process; fill `command`/`args`/`env`/`cwd`) or `"http"` (Streamable HTTP; fill `url` and optionally `headers`, leave `command` empty). Use `"http"` for software-embedded servers such as Figma desktop (`http://127.0.0.1:3845/mcp`). Legacy HTTP+SSE servers (a separate `/sse` endpoint) are NOT supported.
- `env` values may reference the system environment as `"${VAR}"` — prefer that over pasting secrets into the file. `headers` values expand the same way (e.g. `"Authorization": "Bearer ${MY_TOKEN}"`).
- `callTimeoutMs`: per-tool-call timeout. Default 120000; use 240000 for Blender (its internal socket timeout is 180s).
- `autoRestart` (stdio only, default false): restart the process with backoff when it exits unexpectedly (crash-loop protected). Leave off unless the server is daemon-style; a dead server is restarted lazily on the next tool call anyway.
- `loadMode`: `"lazy"` (default; tools load on demand) or `"direct"` (all tool schemas always in context).
- `toolAllowlist` / `toolDenylist`: raw MCP tool names (not the `mcp__` wire names). Non-empty allowlist = only those tools are exposed; denylist wins over allowlist. Use the denylist to hide dangerous tools (e.g. `execute_blender_code`) instead of disabling the whole server.

### Workflow

1. Decide the shape from your research: a package to spawn (stdio — `uvx` for Python servers, `npx -y` for Node servers) or an HTTP endpoint the software hosts itself (http — e.g. Figma desktop). For http servers skip step 2 (no runner needed).
2. Check the runner exists: `where.exe uvx` (or `npx --version`). If missing, offer to install it (`winget install --id=astral-sh.uv -e` for uvx) or point the user to the package's install docs.
3. Read the current config with the `read` tool at `C:\Users\<you>\AppData\Roaming\locus\mcp_servers.json` (it may not exist yet; get the exact prefix from `echo "$APPDATA"` once if needed).
4. Merge — never overwrite other entries. Write the updated JSON back with `edit` (or `write` when the file is new), keeping every existing server unchanged and appending yours. Do not build the JSON through bash heredocs/sed — Windows paths inside JSON need `\\` escaping that shell quoting layers corrupt.
5. Call `mcp_reload` — if its schema is still deferred, load it via `tool_search` first; that one extra step is required, not optional. Do NOT hand-roll the MCP handshake with a bash/python script instead: that only tests your script, leaves Locus's own connection and tool list untouched, and skips the tool schemas (you will guess required arguments wrong). Read the reload report: a ✓ line lists the wire tool names you can now call directly; a ✗ line carries the error plus the server's own stderr — diagnose from that (missing runtime, wrong package name, port in use, ...), fix the config, and reload again.
6. Software-side setup (two-hop servers): remind the user of the in-app step, e.g. for Blender — install the blender-mcp addon in Blender's preferences, then click "Connect to MCP server" in the BlenderMCP sidebar tab (N-panel) so the bridge can reach it on port 9876. Then prove the full chain with one cheap tool call (e.g. `mcp__blender__get_scene_info`).
7. The user can also manage the same servers visually under **Settings → MCP Servers** (add/edit/enable/test); mention it once so they know where the config lives.

## Channel: CLI

1. Check it exists: `where.exe <cli>`. Already installed → skip to auth.
2. Install only after the user confirms: `winget install --id=<id> -e` (find the exact id with `winget search <name>`), or npm/pip/scoop per the tool's docs.
3. Authenticate. `bash` runs non-interactively — never start interactive logins (`gh auth login`-style device flows hang forever). Either hand the user the exact login command to run in their own terminal, or use a token the user sets as a system environment variable. Note the propagation rule: `setx` (and Windows Settings edits) only reach processes started afterwards — Locus keeps the environment it launched with, so `$VAR` in bash and `"${VAR}"` in MCP configs see the new value only after Locus restarts. For the current session, prefix the variable inline per call instead (`GH_TOKEN=... gh ...`) without echoing the value anywhere.
4. Verify with one cheap read-only command: `gh auth status`, `adb devices`, `<cli> --version` or `<cli> whoami`.

## Channel: raw HTTP API

- Put the token in a system environment variable (same restart caveat as CLI auth) and reference it as `$TOKEN` inside the curl command — never paste the literal into a command line, a config file, or the conversation.
- Verify with one cheap GET: `curl -sS -H "Authorization: Bearer $TOKEN" <url>`, or `web_fetch` for public endpoints.
- If the user will call this API often, record the base URL, auth header shape, and two or three example calls in the memory document below.

## Persist what you learned

After the first successful end-to-end call, record the integration so future sessions skip the setup pain. Check the knowledge tree for an existing document first, then `knowledge_create` (or `knowledge_edit`) `memory/integrations/<software>.md` containing, briefly:

- channel and config location (the `mcp_servers.json` entry, CLI name, or API base URL)
- auth: which env var and where it is set — never the secret itself
- the verification command or tool call that proves the connection works
- software-side prerequisites (plugin to enable, mode toggles, "click Connect")
- pitfalls hit during setup and their fixes

Update the existing document on later changes instead of creating duplicates.

## Known recipes

- **Blender** (`ahujasid/blender-mcp`): command `uvx`, args `["blender-mcp"]`, `callTimeoutMs` 240000. Requires the Blender addon (`addon.py` from the project README) and its "Connect to MCP server" button. Pin a Python if needed: args `["--python", "3.11", "blender-mcp"]`.
- **Figma desktop**: transport `"http"`, url `http://127.0.0.1:3845/mcp`, no command. Requires the Figma desktop app with its Dev Mode MCP server enabled (Figma menu → Preferences → Enable Dev Mode MCP Server). "Connection refused" simply means Figma is not running or the toggle is off — fix that and retry; Locus reconnects on the next call.
- **GitHub CLI** (`gh`): winget id `GitHub.cli`. Auth is interactive — the user runs `gh auth login` in their own terminal, or sets a `GH_TOKEN` env var. Verify with `gh auth status`.
- **Generic Python server**: command `uvx`, args `["<package-name>"]`.
- **Generic Node server**: command `npx`, args `["-y", "<package-name>"]`.
- **Generic HTTP server** (software-embedded, e.g. unity-mcp beta hubs): transport `"http"` + the endpoint URL from the software's docs; add `headers` with `"Authorization": "Bearer ${TOKEN_VAR}"` when the endpoint needs auth.

## Bash pitfalls on Windows

- Do not embed `powershell.exe -Command "..."` inside a bash command — bash expands `$var` before PowerShell sees it, and error output can arrive garbled. Use `cygpath`, `find`, `cat`, or the file tools instead.
- If you must generate a config through bash, a python heredoc (`python - <<'PY' ... PY`) is more quote-safe than `cat > file <<'EOF'` when values contain backslashes (Windows paths) — but the file tools remain the first choice.

## Safety

- Only add MCP servers or install CLIs the user explicitly asked for or confirmed — both are arbitrary local programs.
- Never paste API keys or tokens as literal values into configs or command lines when the user can export them as system environment variables instead (`"${MY_KEY}"` / `$MY_KEY`), and never echo a secret back into the conversation.
- Never attempt an interactive auth flow from `bash`; give the user the exact command for their own terminal.
- If the config file contains JSON you cannot parse, stop and show the user instead of overwriting it.
