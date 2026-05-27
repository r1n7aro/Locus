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
tools:
  - skill_create
  - skill_reload
  - skill_list
createdAt: 1775552858000
updatedAt: 1779840000000
---

# Create Skill

## Summary
Create a reusable Skill through the dedicated Skill tools, as either a project-level single document or an APP-installed package with docs, metadata, scripts, CLI assets, and optional Unity C# capabilities.

## Content
## Instructions

1. Clarify the workflow boundary before creating the skill.
   - Ask what repeated task this skill should standardize, what output it should produce, and which checks must always happen.
   - Create a skill only when the workflow has stable steps, reusable judgment rules, or a consistent deliverable.
   - Keep the full workflow under agent control through `SKILL.md`: sequencing, branching, project inspection, retries, validation, and final reporting belong in instructions.
   - Use package tools only as callable interfaces for one stable operation, such as parse, list, inspect, validate, export, import, configure, or save.
   - Do not create a new package tool just because the Skill needs executable code. If the capability can be handled by agent-run Python snippets, package-local `scripts/` helpers, documented `unity_execute` snippets, or package C# helper methods, keep it as scripts/C# plus `SKILL.md` instructions.
   - Create a package tool only when a stable parameter schema, repeatable output schema, permission boundary, or reuse across many workflows makes the callable interface worthwhile.
   - Keep package tools scoped below the full workflow. Tasks that need project conventions, user intent, iterative inspection, or repair decisions stay in `SKILL.md`.
   - For flexible external file formats or APIs, teach the agent through concise `SKILL.md` guidance, references for longer details, and importable library helpers. Let the agent inspect docs, call library functions, adapt parameters, and compose small scripts for the task.
   - Reserve CLI packaging for mature external command-line programs with documented options and stable behavior. Put exploratory Python helpers in `scripts/` as importable modules or examples.
   - Design a validation path the agent can run independently when practical: tests, type checks, compiler passes, deterministic scripts, readback queries, diffs, asset inspections, screenshots, or structured captures.
   - Keep validation claims tied to observable evidence from tools or files. Report any unverified scope explicitly.
   - Treat subjective play, long manual interaction, taste judgment, game feel, and hidden device state as human validation unless the Skill provides reliable instrumentation, scripted simulation, or observable captures.

2. Choose the storage model.
   - Use a single document only for a project-local SOP that needs Markdown instructions and no local runtime assets.
   - Use a package when the user asks for distribution, APP installation, bundled scripts, CLI binaries, multiple docs, or Unity C# files.
   - You MUST use a package for any Skill that depends on a CLI, compiled binary, Python/script helper, Unity C# file, package-local docs, or other local dependency environment. Do not create an `md` Skill that merely tells the user to install the dependency unless the user explicitly asks for docs-only guidance.
   - Use short kebab-case package IDs like `asset-audit` by default. Use IDs like `studio.tools.asset-audit`, `io.github.user.asset-audit`, or another author-owned namespace for distributed packages.

3. Choose the path, slug, and title for a single document.
   - Convert the requested name to kebab-case.
   - Default `skill_create.path` to `<slug>.md`, which maps to `Locus/knowledge/skill/<slug>.md` in the repo.
   - Use a nested path such as `unity/<slug>.md` only when topic grouping materially improves retrieval.
   - Use a human-readable Title Case title.

4. Use the current knowledge semantics.
   - Project skills live under the project knowledge root.
   - APP-installed built-in skills live under `skill/builtin/` in the APP knowledge root and are treated as user-level workflows.
   - New APP-installed Skill packages are created under the APP skill package directory. On Windows the default package root is `%APPDATA%/locus/skills/<package-id>/`, for example `C:/Users/ExampleUser/AppData/Roaming/locus/skills/psd-to-ugui/`.
   - Bundled or development Skill package directories may be discoverable for loading. Use the `packageRoot` returned by `skill_create` for edits.
   - Keep skills focused on SOPs, execution order, checks, and output requirements.

5. Use Skill lifecycle tools for Markdown documents and packages.
   - Create a Markdown document with `skill_create` using `kind: "md"`, `name: <slug>`, `summary: <one-line description>`, and, when needed, `path: <folder>/<slug>.md`.
   - For a command Skill that depends on lazy or Skill-mode tools, set `tools` to the exact Locus tool names the Skill needs on its first turn. Locus loads these tools directly when the user invokes the Skill with its slash command.
   - Declare tool dependencies only in frontmatter `tools`; mentioning tool names in the Markdown body does not register them for first-turn loading.
   - Create a package with `skill_create` using `kind: "package"`, `name: <display name>`, `version: <semver>`, `summary: <one-line description>`, plus optional command metadata.
   - Provide `packageId` when the user gives an exact package ID. When `packageId` is omitted, Locus derives the short `<skill-slug>` package ID from `name`.
   - If the derived package ID already exists, ask the user for an exact package ID before calling `skill_create` again.
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
tools:
  - <tool_name>
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
   - Copy only the final package assets from the APP temp directory into the returned `packageRoot`, such as `skill.json`, `SKILL.md`, docs, scripts, compiled CLI binaries, and Unity C# files.
   - Edit files inside the returned `packageRoot` with filesystem tools when adding docs, scripts, CLI binaries, or Unity C# files.
   - Keep Locus package metadata in `skill.json`; keep `SKILL.md` as the model-facing workflow document.
   - Add optional docs under `references/`, importable Python helpers under `scripts/`, executable CLI files under `bin/`, and Unity C# scripts under `unity/Editor/`.
   - When a server-side search tool is available and the package integrates external software, first check for official or well-maintained SDKs, Python packages, and mature CLI programs.
   - Keep short library usage guidance directly in `SKILL.md`, especially for mature and widely known libraries where the model likely knows the basic API.
   - Use `references/` for long API notes, project-specific mappings, version-sensitive behavior, source links, output schemas, edge cases, and examples that would bloat the root workflow.
   - For Python libraries such as file parsers or API SDKs, document only the non-obvious imports, project conventions, output shapes, dependency installation, and edge cases in `references/`; add a small importable helper module under `scripts/` when repeated glue code is useful.
   - Keep importable helpers free of required stdin/stdout contracts. Put JSON tool adapters in separate `scripts/*_tool.py` files that import the helper module.
   - For mature CLI programs, place the binary under `bin/`, declare it in `capabilities.cli`, and document representative commands. Expose it through `tools[]` only for stable, bounded operations.
   - Keep all manifest paths package-relative and use forward slashes.

9. Use this package structure:

```text
<package-id>/
├── skill.json
├── SKILL.md
├── references/
│   ├── workflow.md
│   └── external-api.md
├── scripts/
│   └── helpers.py
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
  "id": "external-layout",
  "version": "0.1.0",
  "name": "External Layout",
  "description": "Use when inspecting external layout files or APIs and converting the gathered facts into project assets.",
  "argumentHint": "<scope>",
  "disableModelInvocation": false,
  "source": {
    "type": "github",
    "url": "https://github.com/example/locus-skills",
    "reference": "external-layout"
  },
  "command": {
    "enabled": true,
    "trigger": "/external-layout"
  },
  "capabilities": {
    "unity": [
      {
        "name": "ExternalLayoutBridge",
        "path": "unity/Editor/SkillBridge.cs",
        "api": "unity_execute"
      }
    ],
    "python": [
      {
        "name": "external-layout-helpers",
        "path": "scripts/helpers.py",
        "mode": "library",
        "imports": ["psd_tools", "PIL"]
      }
    ],
    "cli": []
  },
  "tools": []
}
```

11. Use this root `SKILL.md` shape for packages:

```markdown
# External Layout

## Instructions

Full execution workflow, required checks, agent-runnable validation steps, expected outputs, validation boundaries, and links to package-local docs. Inside the package file, relative links such as [workflow](references/workflow.md) are allowed. In user-facing replies, cite the full knowledge path such as `skill/external-layout/references/workflow.md`.
```

12. Register package tools only for stable, reusable atomic operations.
   - A package tool should expose an interface boundary, not the end-to-end Skill workflow.
   - Keep orchestration, judgment, project-specific decisions, retries, validation sequencing, and final user reporting in `SKILL.md`.
   - Before adding a `tools[]` entry, check whether the same capability is already clear through a Python script, importable Python helper, documented `unity_execute` snippet, or package-local C# method. Keep those execution paths unregistered when they are easy for the agent to run and inspect.
   - Choose narrow tool names such as `extract-psd-layer-tree`, `validate-manifest`, `list-assets`, `configure-sprites`, or `save-prefab` when the tool exposes one operation.
   - Use `tools[]` after the input schema, output schema, timeout, failure modes, and reuse value are stable enough for a narrow interface.
   - For external file/API parsing that needs exploration, provide reference docs plus Python library examples. Let the agent run short Python snippets or import package helpers with task-specific logic.
   - `runtime: "python"` executes a package-relative Python adapter with managed/system Python and sends tool arguments as JSON stdin by default. Use it for deterministic adapters around known operations.
   - `runtime: "bash"` executes a package-local shell script or trusted command through `sh`.
   - `runtime: "cli"` executes a mature package-local binary or PATH command directly. Use it for tools such as an official Feishu CLI, documented vendor CLI, or a maintained community CLI with stable commands.
   - Default Unity C# support to `SKILL.md` instructions or package C# helper files intended for installation, reading, or direct inclusion in `unity_execute` snippets.
   - Register `runtime: "unity"` tools when a stable package-level Unity operation needs a schema, permission boundary, repeatable tool-call UI, or reuse across many workflows.
   - `runtime: "unity"` with `path` dynamically compiles the package C# source in Unity, then invokes `method` on `entryType`; the method accepts zero parameters or one JSON parameter and returns JSON-compatible data.
   - `runtime: "unity"` with only `typeName` invokes an already loaded static Unity type and is reserved for bridge code that is intentionally installed or otherwise present in the project.
   - For Unity asset or scene authoring, prefer small helpers that collect facts or perform one deterministic write; let the agent use `unity_execute` directly for project-specific creation, repair, and verification steps when inspectability matters.
   - Package tools use short Locus tool names derived from `tools[].name`, such as `validate_manifest`. When a name conflicts with a built-in tool or another package tool, Locus prefixes the package segment, such as `external_layout_validate_manifest`.

Minimal `tools[]` example for a stable adapter:

```json
[
  {
    "name": "validate-manifest",
    "description": "Validate one manifest file and return normalized errors.",
    "runtime": "python",
    "path": "scripts/validate_manifest_tool.py",
    "input": "json-stdin",
    "output": "json-stdout",
    "timeoutMs": 30000,
    "parameters": {
      "type": "object",
      "required": ["manifestPath"],
      "properties": {
        "manifestPath": { "type": "string" }
      }
    }
  }
]
```

13. Respect package document levels when the Skill benefits from progressive disclosure.
   - `L0`, `L1`, and `L2` are optional package sections.
   - Use `L0` for selection guidance, `L1` for a compact workflow, and `L2` for full instructions.
   - Use `SKILL.md` as the progressive disclosure router: keep the core workflow in the root document, then list package-local references with explicit load conditions.
   - Put brief, high-frequency instructions in `SKILL.md`; move longer details into `references/` only when they reduce root-document noise.
   - Keep references one hop from `SKILL.md`, with clear names such as `references/psd-tools.md`, `references/unity-mapping.md`, or `references/api-output.md`.
   - For mature libraries with common model knowledge, include only package-specific constraints, version assumptions, installation commands, and verified source links.
   - Example reference routing: `Read [psd-tools notes](references/psd-tools.md) when the task needs layer-effect, text, mask, or blend-mode behavior beyond basic layer traversal.`

14. Handle Unity C# package files correctly.
   - Put source files in the package under `unity/Editor/`.
   - When an agent loads a Skill package document with `knowledge_read`, Locus compiles package C# sources before the tool returns, then refreshes Unity type discovery for later `unity_execute` and `unity_run_states` calls.
   - Prefer documenting the required `unity_execute` calls or providing C# helper functions that `unity_execute` can invoke after `knowledge_read` has loaded the package.
   - For calls before a package document has been read, install the helper through `capabilities.unity` so Unity compiles it into the project, or include the required C# logic directly in the documented `unity_execute` snippet.
   - Dynamically compiled `runtime: "unity"` tools can invoke package-local C# by `entryType` and `method`, while ordinary workflow orchestration should still stay in `SKILL.md`.
   - Use `tools[]` with `runtime: "unity"` for stable, reusable package interfaces that benefit from explicit parameters and tool-call output.
   - Declare C# files in `capabilities.unity` only when they must be installed as persistent Editor bridge files.
   - Installed Unity C# files are copied into `Packages/com.farlocus.locus/Editor/Skills/<package-id>/` in the target Unity project.
   - Installation status is based on the real target file and hash, so report `installed`, `modified`, `partial`, or `notInstalled` from the Skill UI when relevant.
   - Use a unique namespace derived from the package ID, for example `Locus.SkillPackages.Studio.Tools.PsdToUgui`, and avoid generic type names such as `SkillBridge`, `Builder`, or `Helper`.
   - Document and use fully qualified type names in `unity_execute` snippets and `entryType`; do not rely on short type-name auto-resolution when package or project code may contain duplicate names.
   - If a package installs Unity C# files, duplicate namespace/type pairs across packages or project code can cause Unity compile errors or reflection ambiguity.

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
   - Cite package child documents with the full knowledge path, for example `skill/external-layout/references/workflow.md`; package-local paths belong inside package docs.
   - Include the validation path the Skill gives the agent, plus any manual or subjective checks that remain outside tool-observable verification.
