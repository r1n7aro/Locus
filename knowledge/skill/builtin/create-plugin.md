---
id: kd_skill_create_plugin
type: skill
path: builtin/create-plugin.md
title: Create Plugin
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /create-plugin
argumentHint: <view-or-skill> [plugin-id]
tools:
  - skill_list
  - view_list
  - knowledge_read
  - skill_reload
  - plugin_export
createdAt: 1780185600000
updatedAt: 1780189200000
---

# Create Plugin

## Summary
Prepare and export an existing View or Skill package as a Locus plugin after auditing portability, project dependencies, and plugin structure.

## Content
## Instructions

1. Identify the export target.
   - Use `skill_list` and `view_list` to locate the requested Skill package or View.
   - Only package Skills are exportable. For a Markdown Skill, explain that it must be converted to a package before plugin export.
   - Plugin-managed components are exported through their original plugin. Report the owning plugin id and stop.

2. Audit component completeness.
   - For a Skill package, inspect `skill.json`, `SKILL.md`, package-local scripts, references, and Unity files listed in package metadata.
   - For a View, inspect `view.json`, entry files, style files, bindings, scripts, and package-local runtime files.
   - Confirm every referenced package-local file exists and uses a relative path inside the component package.

3. Classify project portability.
   - A project-independent plugin carries all runtime code, docs, scripts, View files, and Unity C# it needs inside the exported component packages.
   - A project-dependent plugin is allowed when it requires a Unity package, assembly, project script, asset path, generated type, bridge type, or project convention.
   - For every project dependency, produce a metadata record with `kind`, `name`, optional `version`, and optional `notes`.
   - If no project dependencies are found, set the planned metadata to `compatibility.projectIndependent = true`.
   - If any project dependencies are found, set the planned metadata to `compatibility.projectIndependent = false` and list them under `dependencies.project`.

4. Check Unity C# usage.
   - For Skill tools with `runtime: "unity"` and `path`, verify the C# file is package-local and the `entryType` plus `method` match the source.
   - For Skill tools with `runtime: "unity"` and only `typeName`, record the required Unity type as a project dependency.
   - For View scripts, verify each script path is package-local. Record any required project-side type, assembly, asset, or Unity package as a project dependency.
   - Do not approve vague dependencies such as "the current project". Use concrete dependency names.

5. Check project-specific references.
   - Search docs, config, scripts, bindings, and View code for absolute paths, `Assets/...` paths, GUIDs, scene names, prefab names, assemblies, namespaces, Unity package ids, and project-only class names.
   - Keep generic examples in docs when they are clearly examples. Record executable assumptions as dependencies.
   - A dependency on project C# is publishable only when the dependency metadata names the required type, assembly, package, or script.

6. Produce the export brief.
   - Include plugin id, name, version, selected Skill package ids, selected View ids, `compatibility.projectIndependent`, and `dependencies.project`.
   - Report pass/fail checks with concrete evidence from files or tool output.
   - Show the planned archive output path if the user provided one. Ask for the output path when it is missing.
   - Ask the user to approve the exact export brief before calling `plugin_export`.

7. Export only after analysis and approval.
   - Use `plugin_export` only after the audit, dependency classification, structure plan, and user approval are complete.
   - Do not use `plugin_export` as the first step, and do not call it with only component ids.
   - Pass `auditSummary` with the inspected files, completeness checks, dependency decision, plugin-managed source result, and risks.
   - Pass `structurePlan` with the intended plugin directory layout and manifest metadata.
   - Pass `userApproval` as the user instruction or approval text that authorizes this export.
   - If the user asks only for a plan or the audit fails, stop with the export brief and required fixes.
