---
id: kd_skill_create_skill
type: skill
path: builtin/create-skill.md
title: Create Skill
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /create-skill
argumentHint: <skill-name> [--package]
createdAt: 1775552858000
updatedAt: 1779026645000
---

# Create Skill

## Summary
Create a reusable Skill through the dedicated Skill tools, as either a project-level single document or an APP-installed package with docs, metadata, scripts, CLI assets, and optional Unity C# capabilities.

## Content
## Instructions

1. Clarify the workflow boundary before creating the skill.
   - Ask what repeated task this skill should standardize, what output it should produce, and which checks must always happen.
   - Create a skill only when the workflow has stable steps, reusable judgment rules, or a consistent deliverable.

2. Choose the storage model.
   - Use a single document only for a project-local SOP that needs Markdown instructions and no local runtime assets.
   - Use a package when the user asks for distribution, APP installation, bundled scripts, CLI binaries, multiple docs, or Unity C# files.
   - You MUST use a package for any Skill that depends on a CLI, compiled binary, Python/script helper, Unity C# file, package-local docs, or other local dependency environment. Do not create an `md` Skill that merely tells the user to install the dependency unless the user explicitly asks for docs-only guidance.
   - Prefer package IDs like `com.example.asset-audit` for distributed packages and kebab-case slugs like `asset-audit` for single documents.

3. Choose the path, slug, and title for a single document.
   - Convert the requested name to kebab-case.
   - Default `skill_create.path` to `<slug>.md`, which maps to `Locus/knowledge/skill/<slug>.md` in the repo.
   - Use a nested path such as `unity/<slug>.md` only when topic grouping materially improves retrieval.
   - Use a human-readable Title Case title.

4. Use the current knowledge semantics.
   - Project skills live under the project knowledge root.
   - APP-installed built-in skills live under `skill/builtin/` in the APP knowledge root and are treated as user-level workflows.
   - APP-installed Skill packages live under the APP skill package directory, usually `%APPDATA%/locus/skills/<package-id>/` on Windows, `<repo>/skills/<package-id>/` during local development, or `<app>/skills/<package-id>/` next to the packaged executable.
   - Keep skills focused on SOPs, execution order, checks, and output requirements.

5. Use Skill lifecycle tools for Markdown documents and packages.
   - Create a Markdown document with `skill_create` using `kind: "md"`, `name: <slug>`, `summary: <one-line description>`, and, when needed, `path: <folder>/<slug>.md`.
   - Create a package with `skill_create` using `kind: "package"`, `name: <display name>`, `packageId: <reverse-dns-id>`, `version: <semver>`, `summary: <one-line description>`, plus optional command metadata.
   - If the Skill needs a dependency environment such as an external CLI, downloaded binary, generated helper script, or Unity C# bridge, choose `kind: "package"` even when the initial instructions look short.
   - Seed `summary` and `body` in `skill_create` so the skill is usable immediately.
   - Use `skill_list` before creating when command or name conflicts are likely.
   - For an existing Markdown Skill, update content with `knowledge_edit`; for an existing package, edit files under its package root.
   - Run `skill_reload` after creation or content edits to validate that Locus can read the Skill manifest.

6. Use the body template that matches the trigger surface.
   - For command-only Skills, use only execution instructions:

```markdown
## Instructions
```

   - For auto-recalled Skills, add concise selection guidance before the instructions.

7. If you need to repair a command-only Markdown Skill file directly, use this exact document shape, then run `skill_reload`:

```markdown
---
id: kd_skill_<slug_with_underscores>
type: skill
path: <relative-path>.md
title: <Title Case Name>
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /<slug>
argumentHint:
createdAt: <unix-ms>
updatedAt: <unix-ms>
---

# <Title Case Name>

## Summary
<one-line description>

## Content
## Instructions
```

8. Create a package when the Skill needs bundled capabilities.
   - This is mandatory for dependency-bearing Skills: CLI integrations, local executables, scripts, Unity C# capabilities, and workflows that require package-local reference docs must be packaged.
   - Create the initial package with `skill_create`; the result includes `packageRoot` for later file edits.
   - Use the APP temp directory, usually `%APPDATA%/locus/temp/` on Windows, for clone checkouts, archives, generated source, build caches, and intermediate compiler output.
   - Copy only the final package assets from the APP temp directory into `packageRoot`, such as `skill.json`, `SKILL.md`, docs, scripts, compiled CLI binaries, and Unity C# files.
   - Edit files inside the returned `packageRoot` with filesystem tools when adding docs, scripts, CLI binaries, or Unity C# files.
   - Keep Locus package metadata in `skill.json`; keep `SKILL.md` as the model-facing workflow document.
   - Add optional docs under `references/`, Python scripts under `scripts/`, executable CLI files under `bin/`, and Unity C# scripts under `unity/Editor/`.
   - When a server-side search tool is available and the package needs to integrate external software, first check GitHub for an official or well-maintained CLI. Build the CLI into a platform binary, place it under `bin/`, and declare it in `capabilities.cli` or expose it through `tools[]`.
   - Keep all manifest paths package-relative and use forward slashes.

9. Use this package structure:

```text
<package-id>/
├── skill.json
├── SKILL.md
├── references/
│   └── details.md
├── scripts/
│   └── tool.py
├── bin/
│   └── tool.exe
└── unity/
    └── Editor/
        └── SkillBridge.cs
```

10. Use this root `skill.json` shape for packages:

```json
{
  "schema": "locus.skill.v1",
  "id": "com.example.asset-audit",
  "version": "0.1.0",
  "name": "Asset Audit",
  "description": "Use when auditing Unity project assets, unused files, import settings, or cleanup risks.",
  "argumentHint": "<scope>",
  "disableModelInvocation": false,
  "source": {
    "type": "github",
    "url": "https://github.com/example/locus-skills",
    "reference": "asset-audit"
  },
  "command": {
    "enabled": true,
    "trigger": "/asset-audit"
  },
  "capabilities": {
    "unity": [
      {
        "name": "AssetAuditBridge",
        "path": "unity/Editor/SkillBridge.cs",
        "api": "unity_execute"
      }
    ],
    "python": [
      {
        "name": "asset-audit",
        "path": "scripts/tool.py",
        "mode": "cli"
      }
    ],
    "cli": [
      {
        "name": "asset-audit",
        "path": "bin/tool.exe"
      }
    ]
  },
  "tools": [
    {
      "name": "run-audit",
      "description": "Run the package audit script and return structured findings.",
      "runtime": "python",
      "path": "scripts/tool.py",
      "input": "json-stdin",
      "output": "json-stdout",
      "timeoutMs": 120000,
      "parameters": {
        "type": "object",
        "properties": {
          "scope": { "type": "string" }
        }
      }
    },
    {
      "name": "capture-game-view",
      "description": "Dynamically compile the package C# script, capture the Game view, and return findings.",
      "runtime": "unity",
      "path": "unity/Editor/SkillBridge.cs",
      "entryType": "Locus.Skills.AssetAuditBridge",
      "method": "CaptureGameView",
      "requestEditorStatus": "playing",
      "parameters": {
        "type": "object",
        "properties": {
          "frameCount": { "type": "integer", "minimum": 1 }
        }
      }
    }
  ]
}
```

11. Use this root `SKILL.md` shape for packages:

```markdown
# Asset Audit

## Instructions

Full execution workflow, required checks, expected outputs, and references to package-local docs such as [details](references/details.md).
```

12. Register package tools only for stable, reusable operations.
   - `runtime: "python"` executes a package-relative Python script with managed/system Python and sends tool arguments as JSON stdin by default.
   - `runtime: "bash"` executes a package-local shell script or trusted command through `sh`.
   - `runtime: "cli"` executes a package-local binary or PATH command directly.
   - `runtime: "unity"` with `path` dynamically compiles the package C# source in Unity, then invokes `method` on `entryType`; the method accepts zero parameters or one JSON parameter and returns JSON-compatible data.
   - `runtime: "unity"` with only `typeName` invokes an already loaded static Unity type and is reserved for bridge code that is intentionally installed or otherwise present in the project.
   - Tool names are registered as namespaced Locus tools derived from package id and tool name, so use short, action-oriented names inside `tools[]`.

13. Respect package document levels when the Skill benefits from progressive disclosure.
   - `L0`, `L1`, and `L2` are optional package sections.
   - Use `L0` for selection guidance, `L1` for a compact workflow, and `L2` for full instructions.
   - Use links to other package docs for progressive disclosure instead of putting every detail in the root document.

14. Handle Unity C# package files correctly.
   - Put source files in the package under `unity/Editor/`.
   - Prefer exposing callable Unity operations through `tools[]` with `runtime: "unity"` and `path`; this compiles dynamically and leaves the Unity project unchanged.
   - Declare C# files in `capabilities.unity` only when they must be installed as persistent Editor bridge files.
   - Installed Unity C# files are copied into `Packages/com.farlocus.locus/Editor/Skills/<package-id>/` in the target Unity project.
   - Installation status is based on the real target file and hash, so report `installed`, `modified`, `partial`, or `notInstalled` from the Skill UI when relevant.

15. Keep the current skill storage model simple.
   - Prefer `skill_create` for a single Markdown document only when the Skill is text-only.
   - Use a package when bundled resources, multiple docs, dependency setup, CLI tooling, scripts, Unity C# files, or distribution are part of the requirement.
   - Do not recreate legacy `knowledge/Skill/<name>/SKILL.md` directories.

16. When migrating from a legacy skill:
   - Map legacy frontmatter `name` to the new `path`, `title`, and default command trigger.
   - Move the legacy description into `## Summary`.
   - Move the legacy body into `## Content`, then normalize it into `Instructions` for command-only Skills or selection guidance plus `Instructions` for auto-recalled Skills.
   - Preserve useful examples and decision rules, and drop obsolete path conventions such as `knowledge/Skill/<name>/SKILL.md`.
   - If the legacy skill has bundled files or references, migrate it into a package and link detailed docs from the root `SKILL.md`.

17. After creation or migration, run `skill_reload` and report:
   - For a single document: the knowledge path, repo file path, and slash command trigger.
   - For a package: the package ID, package root path, root document path, command trigger, and any Unity C# install target.
