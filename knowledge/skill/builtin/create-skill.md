---
id: kd_skill_create_skill
type: skill
path: builtin/create-skill.md
title: Create Skill
injectMode: excerpt
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
  - knowledge_edit
createdAt: 1775552858000
updatedAt: 1781049600000
---

# Create Skill

## Summary
Use when the user explicitly asks to create or edit a Locus Skill. Ignore Unity project skills, abilities, code, assets, and runtime concepts.

## Content
## Instructions

Command arguments: `<skill-name>` names the skill to create or edit; `--package` forces the package storage model. Ask for a name only when the request does not provide or imply one.

1. Scope the workflow before creating anything.
   - Ask what repeated task this skill standardizes, what output it must produce, and which checks must always happen.
   - Create a skill only when the workflow has stable steps, reusable judgment rules, or a consistent deliverable. Keep skills focused on SOPs: execution order, checks, and output requirements.
   - Keep the full workflow under agent control through the skill body: sequencing, branching, project inspection, retries, validation, and final reporting belong in instructions, not in tools. Executable capabilities stay subordinate; step 6 defines when a package tool is justified.
   - Design a validation path the agent can run independently when practical: tests, type checks, compiler passes, deterministic scripts, readback queries, diffs, asset inspections, screenshots, or structured captures. Keep validation claims tied to observable evidence and report unverified scope explicitly.
   - Treat subjective play, long manual interaction, taste judgment, game feel, and hidden device state as human validation unless the skill provides reliable instrumentation, scripted simulation, or observable captures.

2. Choose the storage model.
   - Use a single Markdown document (`kind: "md"`) only for a project-local SOP that needs instructions and no local runtime assets.
   - You MUST use a package (`kind: "package"`) when the skill depends on anything beyond one Markdown file: a CLI or compiled binary, Python or shell helpers, Unity C# files, package-local reference docs, multiple documents, distribution, or app installation — even when the initial instructions look short. Do not create an md skill that merely tells the user to install the dependency unless the user explicitly asks for docs-only guidance.
   - Honor `--package` from the command arguments as an explicit package request.
   - Use short kebab-case package ids like `asset-audit` by default. Use an author-owned namespace like `studio.tools.asset-audit` or `io.github.user.asset-audit` for distributed packages.

3. Create the skill with the lifecycle tools.
   - Run `skill_list` first when a name or command-trigger conflict is likely.
   - Markdown document: call `skill_create` with `kind: "md"`, `name: <kebab-slug>`, `summary: <one line>`, `body`, and optionally `tools` (step 4). `path` defaults to `<slug>.md`, which maps to `Locus/knowledge/skill/<slug>.md`; use a nested path such as `unity/<slug>.md` only when topic grouping materially improves retrieval. Titles are human-readable Title Case.
   - Package: call `skill_create` with `kind: "package"`, `name: <display name>`, `version: <semver>`, `summary: <one line>`, plus optional `packageId`, `commandTrigger`, `argumentHint`, `commandEnabled`, and `modelInvocationEnabled`. When `packageId` is omitted, Locus derives a short kebab-case id from `name`; if the derived id already exists, ask the user for an exact package id before calling `skill_create` again.
   - Seed `summary` and `body` in `skill_create` so the skill is usable immediately. The default command trigger is `/<name>`.
   - Storage locations: project skill documents live under the project knowledge root; built-in skills live under `skill/builtin/` in the app knowledge root and are user-level workflows; new app packages are created under the app skill package root, `%APPDATA%/locus/skills/<package-id>/` on Windows. The package result includes `packageRoot` — use it for all later file edits.
   - For an existing Markdown skill, update content with `knowledge_edit`. For an existing package, edit files under its package root with filesystem tools.
   - Run `skill_reload` after creation or content edits to validate that Locus can read the manifest. Pass `source` (`project`, `app`, `pluginApp`, `pluginProject`) when the same name exists in multiple sources.

4. Author the body to match the trigger surface.
   - Declare the Locus tool names the skill needs on its first turn in frontmatter `tools` — the `tools` parameter of `skill_create` for documents, or `SKILL.md` frontmatter for packages. Mentioning tool names in the body does not register them; Locus loads the declared tools when the user invokes the slash command.
   - For command-only skills, the body is `## Instructions` followed by execution steps, required checks, and output requirements. Skip selection guidance: the whole document is injected only on invocation.
   - For auto-recalled skills, put one or two selection sentences first — what to use the skill for and what it must ignore — then the instructions. Recall sees the one-line summary or package description, so make that line discriminating.
   - Keep one `## L1` section right under a package root document's title: one or two load-guidance sentences telling the agent when to load this skill and what to ignore — selection guidance, not a recap of the skill's content or workflow. When the package inject mode is `excerpt`, Locus injects the `## L1` text as the package's line in the knowledge structure — a workspace description override wins, then `## L1`, then the manifest `description` — and the UI cannot enable `excerpt` for a package whose root document lacks `## L1`. `skill_create` seeds `## L1` from `summary`.
   - Keep `## L1` aligned with the manifest `description`, which plays the same role on the recall surface. Do not add a `## Summary` section to package documents. `## L0`/`## L2` headings are recorded as presence flags only and are never injected.
   - To repair a command-only Markdown skill file by hand, write this exact shape, then run `skill_reload` (`injectMode: none` keeps a command-only skill out of recall; use `excerpt` with `summaryEnabled: true` when its one-line summary should stay visible to the knowledge index, as this document does):

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

5. Lay out package contents under the returned `packageRoot`.
   - Use the app temp directory, `%APPDATA%/locus/temp/` on Windows, for clone checkouts, archives, generated source, build caches, and intermediate compiler output. Copy only final package assets into `packageRoot`.
   - Use this structure; keep all manifest paths package-relative with forward slashes:

```text
<package-id>/
├── skill.json
├── SKILL.md
├── references/
│   └── external-api.md
├── scripts/
│   └── helpers.py
├── bin/
│   └── tool.exe
└── unity/
    └── Editor/
        └── ExternalLayoutBridge.cs
```

   - `skill.json` holds Locus package metadata; `SKILL.md` is the model-facing workflow document. `skill_create` seeds both, and the manifest parser accepts camelCase or kebab-case keys:

```json
{
  "schema": "locus.skill.v1",
  "id": "external-layout",
  "version": "0.1.0",
  "name": "External Layout",
  "description": "Use when inspecting external layout files or APIs and converting the gathered facts into project assets.",
  "argumentHint": "<scope>",
  "command": { "enabled": true, "trigger": "/external-layout" },
  "capabilities": {
    "unity": [
      { "name": "ExternalLayoutBridge", "path": "unity/Editor/ExternalLayoutBridge.cs", "api": "unity_execute" }
    ],
    "python": [],
    "cli": []
  },
  "tools": []
}
```

   - Only `capabilities.unity` drives runtime behavior (install and compile, step 7). `capabilities.python` and `capabilities.cli` are stored as informational metadata: record script and CLI dependencies there for readers and future tooling, but execution always comes from `tools[]` entries and `SKILL.md` instructions. Add optional metadata such as `source` (`type`/`url`/`reference`) and `disableModelInvocation` when relevant.
   - `SKILL.md` contains the full execution workflow, required checks, agent-runnable validation steps, expected outputs, validation boundaries, and links to package-local docs. Inside package files, relative links like `[workflow](references/workflow.md)` are allowed; in user-facing replies, cite the full knowledge path such as `skill/external-layout/references/workflow.md`.
   - Keep `SKILL.md` as the routing root: brief, high-frequency guidance inline; longer details in `references/`, one hop away, with clear names and explicit load conditions. Example routing line: `Read [psd-tools notes](references/psd-tools.md) when the task needs layer-effect, text, mask, or blend-mode behavior beyond basic layer traversal.`
   - For external file formats, APIs, and libraries: when a server-side search tool is available, first check for official or well-maintained SDKs, Python packages, and mature CLI programs. Keep short usage guidance for widely known libraries directly in `SKILL.md`; use `references/` for long API notes, project-specific mappings, version-sensitive behavior, output schemas, edge cases, and verified source links.
   - Put importable Python helpers under `scripts/` without required stdin/stdout contracts; put JSON tool adapters in separate `scripts/*_tool.py` files that import the helpers. Put mature CLI binaries under `bin/` and document representative commands.

6. Register package tools only for stable, reusable atomic operations.
   - A package tool exposes an interface boundary — one operation such as parse, list, inspect, validate, export, import, configure, or save — never the end-to-end workflow. Orchestration, judgment, project-specific decisions, retries, validation sequencing, and final reporting stay in `SKILL.md`.
   - Do not create a tool just because the skill needs executable code. Prefer agent-run Python snippets, importable `scripts/` helpers, documented `unity_execute` snippets, or package C# helper methods when they are easy for the agent to run and inspect.
   - Register a `tools[]` entry only after the parameter schema, output schema, timeout, failure modes, and reuse value are stable enough for a narrow interface, or when a permission boundary or repeatable tool-call UI is required. For exploratory parsing of external files or APIs, provide reference docs plus library examples instead.
   - Choose narrow names such as `extract-psd-layer-tree`, `validate-manifest`, or `save-prefab`. Locus exposes the tool by the underscore form of `tools[].name` (`validate_manifest`); on a name conflict with a built-in or another package tool, it prefixes the package segment (`external_layout_validate_manifest`).
   - Runtimes: `python` runs a package-relative adapter with managed or system Python — use it for deterministic adapters around known operations; `bash` runs a package-local script or trusted command through `sh`; `cli` runs a mature package-local binary or PATH command directly (official or well-maintained CLIs with stable commands); `unity` is covered in step 7. Tool input defaults to JSON on stdin (`json-stdin`; alternatives `argv-json`, `none`); output defaults to `text` (use `json-stdout` for structured results).
   - Minimal stable adapter example:

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

7. Handle Unity C# package files correctly.
   - Put source files under `unity/Editor/`. Use a unique namespace derived from the package id, for example `Locus.SkillPackages.Studio.Tools.PsdToUgui`, and avoid generic type names such as `SkillBridge`, `Builder`, or `Helper`. Duplicate namespace/type pairs across packages or project code cause Unity compile errors or reflection ambiguity.
   - When an agent loads a package document with `knowledge_read`, Locus compiles the package C# sources before the tool returns, then refreshes Unity type discovery for later `unity_execute` and `unity_run_states` calls. Default to documenting the required `unity_execute` calls or providing C# helper functions those snippets invoke after the package has been read.
   - For calls that must work before any package document is read, install the helper through `capabilities.unity` (persistent Editor bridge files) or include the C# logic directly in the documented `unity_execute` snippet. Installed files are copied to `Packages/com.farlocus.locus/Editor/Skills/<package-id>/` in the target Unity project; installation status (`installed`, `modified`, `partial`, `notInstalled`, ...) is computed from the real target files and hashes, so report it from the Skill UI when relevant.
   - Register a `runtime: "unity"` tool when a stable package-level Unity operation needs a schema, permission boundary, repeatable tool-call UI, or reuse across many workflows. With `path`, Locus dynamically compiles the package C# source and invokes `method` on `entryType`; the method accepts zero parameters or one JSON parameter and returns JSON-compatible data. With only `typeName`, Locus invokes an already loaded static type — reserved for bridge code intentionally installed or otherwise present in the project. Use fully qualified type names in `unity_execute` snippets and `entryType`; do not rely on short type-name resolution when duplicates are possible.
   - For Unity asset or scene authoring, prefer small helpers that collect facts or perform one deterministic write; let the agent use `unity_execute` directly for project-specific creation, repair, and verification steps where inspectability matters.

8. Migrate legacy skills into the current model.
   - Map legacy frontmatter `name` to the new `path`, `title`, and default command trigger; move the legacy description into `## Summary` and the legacy body into `## Content`, normalized per step 4.
   - Preserve useful examples and decision rules. Drop obsolete path conventions; do not recreate legacy `knowledge/Skill/<name>/SKILL.md` directories.
   - If the legacy skill has bundled files or references, migrate it into a package and link detailed docs from the root `SKILL.md`.

9. Validate, then report.
   - Run `skill_reload` and confirm it returns the manifest without errors.
   - For a document: report the knowledge path, repo file path, and slash command trigger.
   - For a package: report the package id, `packageRoot`, root document path, command trigger, and any Unity C# install target.
   - Cite package child documents by full knowledge path, such as `skill/external-layout/references/workflow.md`; package-relative paths belong only inside package docs.
   - State the validation path the skill gives the agent, plus any manual or subjective checks that remain outside tool-observable verification.
