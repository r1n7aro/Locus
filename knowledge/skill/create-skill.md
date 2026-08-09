---
id: kd_skill_create_skill
injectMode: excerpt
summary: >-
  Use when the user explicitly asks to create or edit a Locus Skill. Ignore Unity project skills, abilities, code, assets, and runtime concepts.
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /create-skill
argumentHint: <skill-name> [--package]
tools:
  - create_skill_package
  - skill_reload
  - skill_list
  - read
  - write
  - edit
---

# Create Skill

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

3. Create the skill through its storage boundary.
   - Run `skill_list` first when a name or command-trigger conflict is likely.
   - Markdown document: use `write` to create `Locus/knowledge/skill/<path>.md` with ordinary Markdown body content. Use a nested path such as `unity/<slug>.md` only when topic grouping materially improves retrieval. `write` generates and reports the frontmatter.
   - After `write`, use the reported frontmatter in one exact `edit` to add the required one-line `summary`. Add `tools`, `argumentHint`, `commandTrigger`, or a different `skillSurface` in the same edit only when the workflow needs them. The default surface is command-only and the default command trigger comes from the file name.
   - Package: call `create_skill_package` with `name: <display name>`, `version: <semver>`, `summary: <one line>`, plus optional `packageId`, `commandTrigger`, `argumentHint`, `commandEnabled`, and `modelInvocationEnabled`. When `packageId` is omitted, Locus derives a short kebab-case id from `name`; if the derived id already exists, ask the user for an exact package id before calling `create_skill_package` again.
   - Seed `summary` and `body` in `create_skill_package` so the package is usable immediately. Its default command trigger comes from the final package-id segment.
   - Storage locations: project skill documents live under the project knowledge root; built-in skills live at the root of `skill/` in the app knowledge root and are user-level workflows; new app packages are created under the app skill package root, `%APPDATA%/locus/skills/<package-id>/` on Windows. The package result includes `packageRoot` — use it for all later file edits.
   - For an existing Markdown skill, update its physical file with `edit`. For an existing package, edit files under its package root with filesystem tools.
   - `create_skill_package` validates and reloads the new package before returning. Run `skill_reload` after later content edits, and after creating or editing a Markdown document. Use `source: project` for project documents, `source: app` for app packages, `pluginApp` or `pluginProject` for plugin packages, and `externalUser` or `externalProject` for generic external Skills.

4. Author the body to match the trigger surface.
   - Declare the Locus tool names the skill needs on its first turn in frontmatter `tools`, for both Markdown documents and package `SKILL.md` files. Mentioning tool names in the body does not register them; Locus loads the declared tools when the user invokes the slash command.
   - Skills default to `excerpt` injection: the knowledge structure carries the frontmatter `summary` so the agent can decide when to load it. Write every summary as load guidance — when to load the skill and what to ignore — never as a recap of its content.
   - For command-only skills, the body is `## Instructions` followed by execution steps, required checks, and output requirements. The full document is injected only on invocation, but the one-line summary still surfaces at the default `excerpt` level, so keep it discriminating.
   - For auto-recalled skills, keep selection guidance in frontmatter `summary`; the body begins with instructions or the first domain section. Recall uses that summary, so make it discriminating.
   - Every package root `SKILL.md` requires a non-empty frontmatter `summary`. At the `excerpt` level a workspace override takes precedence over the root summary. `create_skill_package` seeds frontmatter `summary` and writes `"injectMode": "excerpt"` into `skill.json`.
   - Keep root frontmatter `summary` aligned with manifest `description`. `## L1`, `## Summary`, and `## Content` are ordinary body headings and do not define Skill metadata.
   - To create or repair a command-only Markdown skill file by hand, write ordinary Markdown to its designated project Skill directory, add the summary to the generated frontmatter with `edit`, then run `skill_reload`. The `write` execution layer reports the generated metadata and first content line. Existing files retain their frontmatter when edited.

```markdown
---
id: kd_skill_example
injectMode: excerpt
summary: Use when ... Ignore ...
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /example
tools:
  - read
---

# <Title Case Name>

## Instructions

<workflow steps, checks, and output requirements>
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

   - `skill.json` holds Locus package metadata; `SKILL.md` is the model-facing workflow document. `create_skill_package` seeds both. Use camelCase keys in the manifest:

```markdown
---
summary: Use when ... Ignore ...
tools:
  - unity_execute
---

# External Layout

## Instructions
```

```json
{
  "schema": "locus.skill.v1",
  "id": "external-layout",
  "version": "0.1.0",
  "name": "External Layout",
  "description": "Use when inspecting external layout files or APIs and converting the gathered facts into project assets.",
  "injectMode": "excerpt",
  "ignoredMarkdownFiles": ["bin/**/LICENSE.md"],
  "argumentHint": "<scope>",
  "command": { "enabled": true, "trigger": "/external-layout" },
  "capabilities": {
    "unity": [
      { "name": "ExternalLayoutBridge", "path": "unity/Editor/ExternalLayoutBridge.cs", "api": "unity_execute" }
    ],
    "python": [
      { "name": "external-layout-python", "path": "scripts/external_layout.py", "module": "external_layout" }
    ],
    "cli": []
  },
  "tools": []
}
```

   - `capabilities.python` entries with a `module` field are importable package APIs. Locus validates that `path` matches the dotted module name, registers its import root when the Skill is selected or read, and adds the root to every Locus-managed Python process. A single-file module uses `{ "path": "scripts/external_layout.py", "module": "external_layout" }`; a package module uses a directory containing `__init__.py`, such as `{ "path": "scripts/studio/layout", "module": "studio.layout" }`. The agent can then run `import external_layout` directly in Python without a package tool or CLI adapter. Keep the public API data-oriented and return ordinary Python types (`dict`, `list`, `bytes`, numbers, strings). Module names must remain unique across active Skill packages.
   - `capabilities.python` entries without `module` and `capabilities.cli` entries remain informational metadata. `capabilities.unity` drives Unity install and compile behavior (step 7). Add optional metadata such as `source` (`type`/`url`/`reference`) and `disableModelInvocation` when relevant.
   - Use `ignoredMarkdownFiles` for bundled Markdown assets that should stay out of the Knowledge tree, search, and Skill read activation, such as dependency licenses. Entries are package-relative globs with `/` separators: `*` and `?` match within one path segment, while `**` matches any number of segments. Ignoring changes Knowledge visibility only; export and installation retain the files. A rule cannot match the required root `SKILL.md`.
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
   - When an agent loads a package document with `read`, Locus compiles the package C# sources before the tool returns, then refreshes Unity type discovery for later `unity_execute` and `unity_run_states` calls. Default to documenting the required `unity_execute` calls or providing C# helper functions those snippets invoke after the package has been read.
   - For calls that must work before any package document is read, install the helper through `capabilities.unity` (persistent Editor bridge files) or include the C# logic directly in the documented `unity_execute` snippet. Installed files are copied to `Packages/com.farlocus.locus/Editor/Skills/<package-id>/` in the target Unity project; installation status (`installed`, `modified`, `partial`, `notInstalled`, ...) is computed from the real target files and hashes, so report it from the Skill UI when relevant.
   - Register a `runtime: "unity"` tool when a stable package-level Unity operation needs a schema, permission boundary, repeatable tool-call UI, or reuse across many workflows. With `path`, Locus dynamically compiles the package C# source and invokes `method` on `entryType`; the method accepts zero parameters or one JSON parameter and returns JSON-compatible data. With only `typeName`, Locus invokes an already loaded static type — reserved for bridge code intentionally installed or otherwise present in the project. Use fully qualified type names in `unity_execute` snippets and `entryType`; do not rely on short type-name resolution when duplicates are possible.
   - For Unity asset or scene authoring, prefer small helpers that collect facts or perform one deterministic write; let the agent use `unity_execute` directly for project-specific creation, repair, and verification steps where inspectability matters.

8. Migrate legacy skills into the current model.
   - Derive document paths and titles from physical location, keep stable ids, move the legacy description into frontmatter `summary`, and keep the legacy body as ordinary Markdown under the H1 title.
   - Remove obsolete frontmatter (`type`, `path`, `title`, `summaryEnabled`, `commandEnabled`, `readOnly: false`, timestamps) and remove structural `## L1`, `## Summary`, and `## Content` wrappers.
   - Preserve useful examples and decision rules. Drop obsolete path conventions; do not recreate legacy `knowledge/Skill/<name>/SKILL.md` directories.
   - If the legacy skill has bundled files or references, migrate it into a package and link detailed docs from the root `SKILL.md`.

9. Validate, then report.
   - For a Markdown document or an edited package, run `skill_reload` and confirm it returns the manifest without errors. A newly created package is already validated by `create_skill_package`.
   - For a document: report the knowledge path, repo file path, and slash command trigger.
   - For a package: report the package id, `packageRoot`, root document path, command trigger, and any Unity C# install target.
   - Cite package child documents by full knowledge path, such as `skill/external-layout/references/workflow.md`; package-relative paths belong only inside package docs.
   - State the validation path the skill gives the agent, plus any manual or subjective checks that remain outside tool-observable verification.
