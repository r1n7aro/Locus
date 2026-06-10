---
id: kd_skill_ask_locus
type: skill
path: builtin/ask-locus.md
title: Ask Locus
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: both
commandTrigger: /ask-locus
argumentHint: <question>
createdAt: 1780934400000
updatedAt: 1781049600000
---

# Ask Locus

## Summary
Use when the user asks how Locus works, how to operate a Locus feature, how Locus connects to Unity, or where a Locus behavior is implemented. Ignore questions about the current Unity project itself.

## Content
Use this skill when the user asks how Locus works, how to operate a Locus feature, how Locus connects to Unity, how a built-in workflow behaves, or where a Locus behavior is implemented.

## When to use

- The user asks how to use Chat, Agent, Knowledge, Skill, Plugin, View, Collab, Asset, Diff, settings, updates, or Unity integration in Locus.
- The user asks what a Locus button, panel, command, setting, tool, permission, or workflow does.
- The user asks why Locus behaves a certain way and the answer should be grounded in the current source or docs.
- The user invokes `/ask-locus`.

## When to ask first

- The question could refer to the installed app, the current development branch, or a specific released version, and the distinction changes the answer.
- The user asks for account, billing, or private deployment details that are not inferable from public Locus source.
- The user asks for a destructive troubleshooting step, migration, or data cleanup and the relevant storage path is unclear.

## Source cache

Repository: `https://github.com/r1n7aro/Locus`

Keep a reusable clone in the Locus app temp directory. The app temp directory is usually `%APPDATA%/locus/temp/` on Windows; if Locus settings expose a different temp path, use that path.

Use this layout:

- Cache root: `<app-temp>/ask-locus/`
- Source checkout: `<app-temp>/ask-locus/Locus/`
- Refresh stamp: `<app-temp>/ask-locus/.last-fetch`

Do not clone on every question.

1. If `<app-temp>/ask-locus/Locus/.git` exists, use the existing checkout.
2. Refresh with `git pull --ff-only --depth=1` only when the refresh stamp is missing or older than 24 hours.
3. If refresh fails, keep the existing checkout and mention that the answer is based on the cached revision.
4. If the checkout is missing, clone `https://github.com/r1n7aro/Locus.git` with `--depth=1`.
5. Write the refresh stamp only after a successful clone or refresh.
6. Do not edit files in the cached checkout.

PowerShell cache preparation pattern:

```powershell
$root = Join-Path $env:APPDATA "locus\temp\ask-locus"
$repo = Join-Path $root "Locus"
$stamp = Join-Path $root ".last-fetch"
New-Item -ItemType Directory -Force $root | Out-Null

if (Test-Path (Join-Path $repo ".git")) {
  $shouldRefresh = $true
  if (Test-Path $stamp) {
    $age = (Get-Date) - [DateTime]::Parse((Get-Content $stamp -Raw).Trim())
    $shouldRefresh = $age.TotalHours -ge 24
  }
  if ($shouldRefresh) {
    git -C $repo pull --ff-only --depth=1
    if ($LASTEXITCODE -eq 0) {
      (Get-Date).ToString("o") | Set-Content -NoNewline $stamp
    }
  }
} else {
  git clone --depth=1 https://github.com/r1n7aro/Locus.git $repo
  if ($LASTEXITCODE -eq 0) {
    (Get-Date).ToString("o") | Set-Content -NoNewline $stamp
  }
}
```

## Research workflow

1. Identify the feature area from the question.
   - Product docs: `docs/overview/`, `docs/product/`, `docs/en/`.
   - Frontend UI: `src/components/`, `src/composables/`, `src/stores/`, `src/services/`, `src/language/`.
   - Desktop backend: `src-tauri/src/commands/`, `src-tauri/src/agent/`, `src-tauri/src/session/`, `src-tauri/src/tool/`, `src-tauri/src/knowledge_*`.
   - Unity bridge and package: `locus_unity/`, `src-tauri/src/unity_*`, `src-tauri/src/commands/unity_*`.
   - Built-in tools and workflows: `tools/`, `knowledge/skill/builtin/`, `skills/`, `agent/`.

2. Search narrowly before reading files.
   - Start with docs for user-facing behavior.
   - Search source identifiers, UI labels, translation keys, command names, and tool names.
   - Read the smallest set of files that can answer the question.

3. Answer from evidence.
   - Prefer current source and docs over memory.
   - Cite relevant paths and line numbers when the source location matters.
   - State the cached commit or branch when freshness could affect the answer.
   - Mark inferences as inferences.

4. Keep answers practical.
   - Give direct steps for user operations.
   - Explain implementation behavior only to the depth needed for the question.
   - For troubleshooting, include observable checks before remediation.
   - For risky operations, confirm the target path or project before suggesting changes.

## Bug reporting

If source inspection indicates a likely Locus bug, report the evidence first: observed behavior, expected behavior, reproduction steps, relevant version or commit, environment details, and the source files or docs inspected.

When the user wants to file the bug and GitHub is logged in, use the GitHub CLI from the repository context:

1. Check authentication with `gh auth status -h github.com`.
2. Draft a concise issue for `r1n7aro/Locus`.
3. Include reproduction steps, actual result, expected result, logs or screenshots when available, platform details, Unity version when relevant, and source references used for the diagnosis.
4. Ask the user to confirm the final title and body before publishing.
5. Create the issue with `gh issue create --repo r1n7aro/Locus --title "<title>" --body-file <body.md>`.
6. Return the created issue URL.

## Answer style

- Be concise and concrete.
- Reply in the same language as the user's question unless the user explicitly asks for another language.
- Avoid unsupported claims about unreleased behavior.
- If the source and docs disagree, say which source is newer and base the recommendation on the newer evidence.
