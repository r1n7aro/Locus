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
argumentHint: <skill-name>
createdAt: 1775552858000
updatedAt: 1776267594940
---

# Create Skill

## Summary
Create a new unified Skill document in the project's knowledge store, using the current single-file format and the correct command-facing metadata.

## Content
## When to use

- Turn a repeated workflow into a reusable project skill.
- Recover or migrate a legacy `knowledge/Skill/<name>/SKILL.md` workflow into the current `Locus/knowledge/skill/*.md` format.
- Add a slash-command entry that the team can trigger repeatedly for the same kind of task.

## When NOT to use

- A one-off project conclusion belongs in `design`.
- A user preference or hidden long-term context belongs in `memory`.
- External package docs, version notes, or copied references belong in `reference`.
- Temporary task notes or scratchpad content should stay in the current conversation.

## Instructions

1. Clarify the workflow boundary before creating the skill.
   - Ask what repeated task this skill should standardize, what output it should produce, and which checks must always happen.
   - Create a skill only when the workflow has stable steps, reusable judgment rules, or a consistent deliverable.

2. Choose the path, slug, and title.
   - Convert the requested name to kebab-case.
   - Default to `skill/<slug>.md` in knowledge tools, which maps to `Locus/knowledge/skill/<slug>.md` in the repo.
   - Use a nested path such as `skill/unity/<slug>.md` only when topic grouping materially improves retrieval.
   - Use a human-readable Title Case title.

3. Use the current knowledge semantics.
   - Project skills live under the project knowledge root.
   - APP-installed built-in skills live under `skill/builtin/` in the APP knowledge root and are treated as user-level workflows.
   - Keep skills focused on SOPs, execution order, checks, and output requirements.

4. Prefer the unified knowledge tools.
   - Create the document with `knowledge_create` using a type-prefixed path such as `skill/<slug>.md`.
   - Seed both `summary` and `body` in the create call so the skill is usable immediately.
   - If the skill already exists, update its body with `knowledge_edit` instead of creating a duplicate.

5. Use this body template for the `document.body` content:

```markdown
## When to use

## When NOT to use

## Instructions
```

6. If you need to create or repair the Markdown file directly, use this exact document shape:

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
## When to use

## When NOT to use

## Instructions
```

7. Keep the current skill storage model simple.
   - Prefer a single Markdown document.
   - Do not recreate legacy `SKILL.md` directories, sidecar `references/`, `scripts/`, or `assets/` unless the user explicitly asks for a richer skill package.

8. When migrating from a legacy skill:
   - Map legacy frontmatter `name` to the new `path`, `title`, and default command trigger.
   - Move the legacy description into `## Summary`.
   - Move the legacy body into `## Content`, then normalize it into `When to use`, `When NOT to use`, and `Instructions` sections when needed.
   - Preserve useful examples and decision rules, and drop obsolete path conventions such as `knowledge/Skill/<name>/SKILL.md`.

9. After creation or migration, report:
   - The knowledge path, for example `skill/<slug>.md`.
   - The repo file path, for example `Locus/knowledge/skill/<slug>.md`.
   - The slash command trigger, usually `/<slug>`.
