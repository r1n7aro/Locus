---
tools:
  - plugin_list
  - plugin_search
  - plugin_install
  - plugin_set_enabled
  - plugin_uninstall
  - plugin_export
  - skill_list
  - skill_reload
  - view_list
  - view_reload
  - knowledge_read
  - ask_user_question
  - sheet
---

# Plugin

## L1
Load when plugin or 插件 means a Locus app extension: search, install, enable, disable, uninstall, create, package, publish. Ignore Unity project plugins.

## Instructions

Treat `/plugin <request>` as the only command entry. Interpret words such as search, install, uninstall, remove, create, import, pack, zip, publish, release, registry, PR, Skill, View, and Rule as intent signals inside the natural-language request.

1. Route the request.
   - Discovery: `plugin_search` for the registries configured in Locus, `plugin_list` for installed plugins.
   - Install, enable, disable, uninstall: resolve the target with step 2, then call the matching tool.
   - Creation or packaging: locate source components with `skill_list` and `view_list`, then follow steps 8-10.
   - Editing: locate plugin-managed components with `plugin_list`, `skill_list`, and `view_list`; validate edits with `skill_reload` or `view_reload`.
   - Publishing beyond a local zip archive: follow step 11.

2. Resolve the target before any state change.
   - Use `plugin_search` first when an install target id is unclear. Use `plugin_list` first when an installed target or scope is unclear.
   - `plugin_set_enabled` and `plugin_uninstall` infer scope when the id is installed in exactly one scope. If the same id is installed in both app and project scope, ask which scope to change.
   - Default to app scope for user-wide plugins. When project scope seems intended but was not stated, ask before using it.

3. Use progressive disclosure for specialized editing.
   - Keep `/plugin` focused on plugin lifecycle, ownership, packaging, dependency metadata, and publishing decisions.
   - To create or edit a View, load the View skill with `knowledge_read` using `path: "skill/view"` and `part: "body"`, then follow that skill.
   - To create or edit a Skill, load the Create Skill workflow with `knowledge_read` using `path: "skill/create-skill.md"` and `part: "body"`, then follow that workflow.
   - Return to this workflow after the component edit is validated, then continue packaging, installation, ownership transfer, or publishing.

4. Ask when a choice changes the result.
   - Use `ask_user_question` when multiple registry matches are plausible, install or uninstall scope is unclear, component selection is unclear, import mode is unclear, publish target is unclear, or GitHub authentication is missing.
   - Use `sheet` to confirm collected multi-field metadata in one pass (export step 10, registry entry metadata in the publishing workflow) instead of asking field by field.
   - Always ask before creating, exporting, or publishing a new plugin id. The package name is the plugin id, and the user must choose or enter it before export.
   - Show any proposed id explicitly. A proposal may come from an existing plugin id, selected Skill/View slug, repository owner, or authenticated GitHub login, but none of those sources decides the final id.
   - Prefer concise package-manager style ids such as `asset-browser-tools` or `locus-workspace`. Use reverse-DNS or owner-prefixed ids only when the user chooses that naming scheme or a collision needs disambiguation.
   - Prefer lowercase letters, digits, and hyphens for new public ids. The runtime also accepts `_` and `.`; reject slashes, path traversal, leading dot, trailing dot, and empty ids.
   - After the user chooses an id, check for duplicates before continuing.

5. Install plugins.
   - Prefer `pluginId` installs for registry entries; the id resolves across the configured registries in order, and `plugin_search` results carry the matching `registryBaseUrl`. Use `path`, `url`, `repo`, or `source` installs for local development, private plugins, archives, and GitHub sources.
   - After install, report installed id, version, scope, root, and included Skill/View/Rule components.

6. Enable, disable, or uninstall plugins.
   - Disabling a plugin keeps it installed and listed, while its Agent definitions, Rules, Skill packages, tools, and Views stop loading from that plugin.
   - Plugin Rules follow the plugin state. They are visible as read-only Agent rules while the plugin is enabled.
   - After the change, report the affected id and scope.

7. Work with editable plugin components.
   - Local path/source installs are suitable for iterative local plugin work. Edit files inside the returned plugin root, then validate with `skill_reload` or `view_reload`.
   - `skill_reload` source may be `pluginApp` or `pluginProject` for plugin Skill packages. `view_list` and `view_reload` include plugin Views by id; use the returned `packageRoot` for edits.
   - Optional plugin Rules live under the plugin root, usually `rules/<rule-name>.md`, and are declared in `locus.plugin.json` under `components.rules`. They are enabled by default while the plugin is enabled.
   - Registry-installed plugins are treated as managed components. Ask before replacing, forking, or editing them in place.

8. Create or package from existing components.
   - Locate the requested components with `skill_list` and `view_list`. Only Skill packages are exportable; convert a Markdown-only Skill to a Skill package before packaging.
   - Plugin-managed components are exported through their owning plugin. Report the owning plugin id and ask before creating a fork or replacement package.
   - Inspect manifests, root docs, source files, scripts, bindings, Unity C# files, package-local assets, and referenced files.

9. Audit portability before export.
   - A project-independent plugin carries all runtime code, docs, scripts, View files, and Unity C# it needs inside exported component packages.
   - A project-dependent plugin records each required Unity package, assembly, script, asset path, generated type, bridge type, or project convention as a dependency with `kind`, `name`, optional `version`, and optional `notes`.
   - Set `compatibility.projectIndependent = true` only when the audit finds no project dependencies; otherwise set it to `false` and fill `dependencies.project`.

10. Export only after approval.
   - Confirm the export metadata with the `sheet` tool: one sheet titled with the plugin id and version, with fields for plugin id, name, version, selected Skill package ids, selected View ids, output path, install scope, and dependency metadata. Summarize checked files and remaining risks in the sheet description. Mark values the user already fixed as readonly and leave the rest editable.
   - The sheet confirms collected values; it does not collect missing ones. Ask with `ask_user_question` first when the output path, version, package id, or publish target is still unknown.
   - For new plugins, force the package name question even when a high-confidence id is available.
   - When the sheet returns a change request, revise the proposal and present an updated sheet. Never call `plugin_export` while the latest sheet outcome is a change request.
   - Before export, run `plugin_list` and check installed app/workspace plugins for the selected id. If it already exists, ask whether to update that plugin, choose another id, or stop.
   - Call `plugin_export` only after the audit, structure plan, and sheet confirmation are complete. Pass detailed `auditSummary` and `structurePlan`, and record the confirmed sheet values, including user edits, in `userApproval`.
   - For plugin creation from existing components, pass `installAfterExport: true` and `transferOwnership: true` so the created plugin installs locally and takes ownership of the components. Use `installScope: "app"` by default; use `"project"` when the plugin should stay in the current workspace.
   - After export, report the installed plugin root and the transferred Skill/View component ids. Continue future edits inside the installed plugin root.

11. Publish.
   - For zip-only publishing, the approved `plugin_export` archive is the deliverable; report the output path.
   - For GitHub release, plugin repository, or registry publishing, load the [publishing workflow](publish.md) with `knowledge_read` using `path: "skill/plugin/publish.md"` and `part: "body"`, then follow it end to end. Do not improvise GitHub or registry mechanics from memory.
