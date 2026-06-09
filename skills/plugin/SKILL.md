---
tools:
  - plugin_list
  - plugin_search
  - plugin_install
  - plugin_uninstall
  - plugin_export
  - skill_list
  - skill_reload
  - view_list
  - view_reload
  - knowledge_read
  - ask_user_question
  - bash
---

# Plugin

## Summary
Search, install, uninstall, create, package, and publish Locus plugins through one natural-language `/plugin` command.

## Instructions

Treat `/plugin <request>` as the only command entry. Interpret words such as search, install, uninstall, remove, create, import, pack, zip, publish, release, registry, PR, Skill, View, and Rule as intent signals inside the natural-language request.

1. Route the request.
   - Discovery: use `plugin_search` for registry search and `plugin_list` for installed plugins.
   - Installation: use `plugin_search` first when the target id is unclear. Use `plugin_install` after the target and scope are clear.
   - Uninstallation: use `plugin_list` first when the installed target or scope is unclear. Use `plugin_uninstall` after the installed plugin id and scope are clear.
   - Creation or packaging: use `skill_list` and `view_list` to locate source Skill packages and Views, then audit before `plugin_export`.
   - Editing: use `plugin_list`, `skill_list`, `view_list`, `skill_reload`, and `view_reload` to locate and validate plugin-managed components after file edits.
   - Publishing: package to a zip, prepare a GitHub release/repository, or create a registry PR according to the user's confirmed target.

2. Use progressive disclosure for specialized editing.
   - Keep `/plugin` focused on plugin lifecycle, ownership, packaging, dependency metadata, and publishing decisions.
   - When the user needs to create or edit a View, load the View skill with `knowledge_read` using `path: "skill/view"` and `part: "body"`, then follow that skill for View source edits and validation.
   - When the user needs to create or edit a Skill, load the Create Skill workflow with `knowledge_read` using `path: "skill/builtin/create-skill.md"` and `part: "body"`, then follow that workflow for Skill source edits and validation.
   - Return to this Plugin workflow after the View or Skill edit is validated, then continue plugin packaging, installation, ownership transfer, or publishing.

3. Ask when a choice changes the result.
   - Use `ask_user_question` when multiple registry matches are plausible, install or uninstall scope is unclear, component selection is unclear, import mode is unclear, publish target is unclear, registry target is unclear, or GitHub authentication is missing.
   - Provide at most 3 options. Make the last option a custom input option.
   - For publishing choices, offer the two most likely concrete paths plus custom input, such as "Zip archive", "Registry PR", and "Custom".
   - Always use `ask_user_question` before creating, exporting, or publishing a new plugin id. The package name is the plugin id, and the user must choose or enter it before export.
   - Show any proposed id explicitly. A proposal may come from an existing plugin id, selected Skill/View slug, repository owner, or authenticated GitHub login, but none of those sources decides the final id.
   - Prefer concise package-manager style ids such as `asset-browser-tools` or `locus-workspace`. Use reverse-DNS or owner-prefixed ids only when the user chooses that naming scheme or a collision needs disambiguation.
   - Prefer lowercase letters, digits, and hyphens for new public ids. The runtime also accepts `_` and `.`; reject slashes, path traversal, leading dot, trailing dot, and empty ids.
   - After the user chooses an id, check for duplicates before continuing.

4. Install plugins.
   - Prefer `pluginId` installs for official registry entries.
   - Use `path`, `url`, `repo`, or `source` installs for local development, private plugins, archives, and GitHub sources.
   - Default to app scope for user-wide plugins. Ask before project scope when the user did not specify it.
   - After install, report installed id, version, scope, root, and included Skill/View/Rule components.

5. Uninstall plugins.
   - Use `plugin_list` first when the request does not provide an exact installed plugin id and scope.
   - If the same plugin id is installed in both app and project scope, ask the user which scope to remove.
   - Use `plugin_uninstall` only after the target is clear. The tool can infer scope when the id is installed in exactly one scope.
   - After uninstall, report removed id and scope.

6. Work with editable plugin components.
   - Local path/source installs are suitable for iterative local plugin work. Edit files inside the returned plugin root, then validate with `skill_reload` or `view_reload`.
   - `skill_reload` source may be `pluginApp` or `pluginProject` for plugin Skill packages.
   - `view_list` and `view_reload` include plugin Views by id. Use the returned `packageRoot` for edits.
   - Optional plugin Rules live under the plugin root, usually `rules/<rule-name>.md`, and are declared in `locus.plugin.json` under `components.rules`. Agent settings control whether each Agent enables them.
   - Registry-installed plugins are treated as managed components. Ask before replacing, forking, or editing them in place.

7. Create or package from existing components.
   - Use `skill_list` and `view_list` to locate the requested components.
   - Only Skill packages are exportable. A Markdown-only Skill must be converted to a Skill package before packaging.
   - Plugin-managed components are exported through their owning plugin. Report the owning plugin id and ask before creating a fork or replacement package.
   - Inspect manifests, root docs, source files, scripts, bindings, Unity C# files, package-local assets, and referenced files.

8. Audit portability before export.
   - A project-independent plugin carries all runtime code, docs, scripts, View files, and Unity C# it needs inside exported component packages.
   - A project-dependent plugin records each required Unity package, assembly, script, asset path, generated type, bridge type, or project convention.
   - For each dependency, record `kind`, `name`, optional `version`, and optional `notes`.
   - Set `compatibility.projectIndependent = true` only after the audit finds no project dependencies.
   - Set `compatibility.projectIndependent = false` and fill `dependencies.project` when project dependencies exist.

9. Export only after approval.
   - Present plugin id, name, version, selected Skill package ids, selected View ids, output path, dependency metadata, checked files, and remaining risks.
   - Use `ask_user_question` when the output path, version, package id, or publish target is missing.
   - For new plugins, force the package name question even when a high-confidence id is available. Record the user's selected plugin id in `userApproval`.
   - Before export, run `plugin_list` and check installed app/workspace plugins for the selected id. If it already exists, ask whether to update that plugin, choose another id, or stop.
   - Call `plugin_export` only after the audit, structure plan, and explicit user approval are complete.
   - For plugin creation from existing components, install the created plugin locally and transfer ownership by passing `installAfterExport: true` and `transferOwnership: true`.
   - Use `installScope: "app"` by default. Use `installScope: "project"` when the user wants the plugin stored in the current workspace or the plugin should stay project-local.
   - After export, report the installed plugin root and the transferred Skill/View component ids. Continue future edits inside the installed plugin root.
   - Pass detailed `auditSummary`, `structurePlan`, and `userApproval`.

10. Publish.
   - For zip-only publishing, create the approved archive with `plugin_export` and report the path.
   - For GitHub release or repository publishing, use `bash` with `gh` only after confirming GitHub CLI authentication. If authentication is missing, ask the user to run `gh auth login -h github.com -s repo,read:org`.
   - A public plugin GitHub repository must contain an installable plugin source tree at the repository root. Commit `locus.plugin.json`, component directories such as `views/`, `skills/`, `agents/`, and `rules/`, plus README/LICENSE files. Do not create a repository that only contains README/LICENSE and a release asset.
   - Build the GitHub repository source tree from the same exported plugin root used for the release asset. The repository root, the GitHub source archive, and the release zip must all contain a root `locus.plugin.json` with the same plugin id and version.
   - Do not hide the installable plugin under `release/`, `dist/`, or another nested folder in the plugin repository. GitHub repo installs clone the repository and locate `locus.plugin.json`; the repository itself must be directly installable.
   - Before publishing a release or registry PR, validate repository-source installation separately from release-asset installation: inspect or install from the GitHub repo/default branch, and inspect the release zip. Both must resolve to the same id, version, and component tree.
   - The official Locus plugin registry is always `r1n7aro/locus-plugin-registry`.
   - When the user asks to publish to the official registry, create an official registry PR, or does not name a registry repository, use `r1n7aro/locus-plugin-registry` directly. Do not ask the user to choose a registry repository.
   - Ask for the registry repository only when the user explicitly requests a custom registry or names more than one possible registry repository.
   - Do not infer the registry repository from plugin id, plugin repository name, GitHub login, package namespace, organization name, or common names such as `farlocus_registry`.
   - Use the official registry default branch for real publishing unless the user explicitly names another base branch, such as `test` for validation.
   - Before a registry release PR, verify the release archive after the final package is built: download or inspect the exact archive, compute SHA-256 and byte size, and read root `locus.plugin.json` to confirm `id` and `version` match the registry entry. Also verify the plugin repository source archive or clone has the same root manifest id/version and component tree.
   - After publishing or replacing a GitHub release asset, list the latest release assets and resolve the intended `downloadSource` rule. If `downloadSource.assetPattern` is used, it must match exactly one asset. If it matches zero or multiple assets, fix the release assets or selector before opening a registry PR and before declaring a version-only release complete.
   - For version-only updates of an already registered plugin, update and commit the plugin repository root source tree, publish the GitHub release asset from that committed source, and let registry CI refresh `public/v1`. Do not open a registry PR unless metadata, compatibility, dependency metadata, description, icon, tags, repo, license, or download source rules change.
   - For official registry publishing, prepare only the source plugin entry `entries/v1/plugins/<bucket>/<plugin-id>.json`, where bucket is the first two hex chars of the SHA-256 of the plugin id.
   - Every registry source entry must include user-facing metadata: `author`, `repo`, `license`, `summary`, `description`, `tags`, compatibility, and `stats`.
   - Use the current registry fields. Do not use legacy or npm-style fields such as `schema`, `version`, `homepage`, `repository`, or `components` in a registry source entry.
   - Include standard stats definitions for GitHub-hosted plugins: `githubStars` with label `Stars`, and `releaseDownloads` with label `Release downloads`. Registry CI refreshes these stat values when it generates `public/v1`.
   - For GitHub release-hosted plugins, the source entry should store `downloadSource`, not generated `download`, `latestVersion`, or release SHA fields. Registry CI resolves `latestVersion`, `download.url`, `download.sha256`, `download.sizeBytes`, `updatedAt`, and `downloadSource.version` into `public/v1`.
   - Use `downloadSource.type: "latestRelease"` for normal GitHub release publishing. If each release has exactly one plugin zip, omit `asset`. If release asset names include the version, use `assetPattern`, such as `locus-workspace-*.zip`. Do not set `asset` to a versioned filename such as `locus-workspace-0.1.0.zip` for latest-release sources.
   - Registry CI validates source entries on PRs. Pushes, scheduled runs, and manual dispatches rebuild `public/v1` from `entries/v1` and GitHub release assets, so downstream version-only releases are picked up without a registry PR.
   - If a latest GitHub release contains multiple zip assets, set either `downloadSource.asset` for a stable asset name or `downloadSource.assetPattern` for versioned names. Without one of those selectors, CI should fail instead of guessing.
   - Treat `public/v1/manifest.json`, `public/v1/shards/<bucket>.json`, `public/v1/plugins/<bucket>/<plugin-id>.json`, and `public/v1/search/summaries.json` as generated registry index files owned by the registry repository CI. Registration PRs should leave them unchanged.
   - Before opening a registry PR, confirm target repository, plugin id, author, release source rule, description source, license, compatibility, dependency metadata, plugin purpose, usage instructions, and stats definitions.
   - Treat the confirmed plugin id as stable release identity. Future registry entries and archive names keep that id until the user explicitly chooses a replacement id and handles migration.
   - The registry repository owner does not determine plugin id. The GitHub account login is only a visible default proposal for repository owner or optional owner-prefixed naming.
   - Check registry duplicates after the user chooses the id: compute the bucket and read `entries/v1/plugins/<bucket>/<plugin-id>.json` on the target base branch. If the id exists, ask whether this is a new version of that plugin, a replacement/fork with a new id, or a stop.
   - Check GitHub repository availability for the confirmed plugin repo with `gh repo view <repo-owner>/<repo-name>`. If it exists, ask before reusing it for release assets.

11. Use the reusable official registry PR flow.
   - Treat the official registry repository as the PR base repository. All writable branches for registry changes belong to the user's fork.
   - Set `<registry-owner>/<registry-repo>` to `r1n7aro/locus-plugin-registry` for official registry publishing. This value is fixed; skip registry-repository discovery and registry-repository confirmation.
   - Resolve only the base branch. Use the official registry default branch for real publishing. Use a user-confirmed branch such as `test` for validation.
   - Run one GitHub preflight: `gh auth status -h github.com`, `gh api user --jq .login`, and `gh repo view r1n7aro/locus-plugin-registry --json nameWithOwner,defaultBranchRef`.
   - Ensure a fork with `gh repo fork r1n7aro/locus-plugin-registry --clone=false --remote=false`, then verify `<viewer>/locus-plugin-registry` exists.
   - Create a unique fork branch from the upstream base SHA, such as `register/<plugin-id>/<version>-<timestamp>`.
   - Write exactly one registry source file for a new plugin: `entries/v1/plugins/<bucket>/<plugin-id>.json`.
   - For an existing plugin, update `entries/v1/plugins/<bucket>/<plugin-id>.json` only when registry metadata or download source rules change. Version-only releases do not need a registry PR.
   - With the GitHub contents API, creating a new entry uses `message`, `branch`, and `content`. Updating an existing entry must first read the file on the target branch and include its current blob `sha` in the PUT payload.
   - Do not modify `public/v1/**` in the registration PR. The registry index refresh is handled by registry repository automation after the plugin entry is accepted.
   - Prefer `gh api repos/<fork-owner>/<registry-repo>/contents/<path> -X PUT --input -` for registry file writes. This avoids cloning and keeps the workflow stable in constrained network environments.
   - Create a Markdown PR body file and pass it with `--body-file <body.md>` for every multi-line PR body.
   - Write PR body files as literal Markdown. Avoid escaped `\n` strings and shell strings that treat Markdown backticks as escape characters. In PowerShell, prefer a single-quoted here-string (`@' ... '@`) or another literal file-writing method.
   - Include these PR body sections:
     - `## Summary`: registration target, plugin id, version, and release link.
     - `## What This Plugin Does`: user-facing purpose and the included Skill/View/Rule components.
     - `## How To Use`: install path plus the natural way to invoke included Skills or open included Views.
     - `## Registry Metadata`: plugin repo, author, stats definitions, download source rule, current release link, bucket, compatibility, dependencies, license, description source, and the single source entry path.
     - `## Validation`: export, local install and ownership transfer, plugin repository source installability, release archive inspection, source entry update, registry CI resolves generated version/download metadata, and `public/v1/**` left unchanged.
   - Create the PR with `gh pr create --repo <registry-owner>/<registry-repo> --base <base-branch> --head <viewer>:<fork-branch> --title "Register <plugin-id> <version>" --body-file <body.md>`.
   - Verify the PR with `gh pr view --repo <registry-owner>/<registry-repo> <number-or-url> --json url,state,baseRefName,headRefName,headRepositoryOwner`.
   - Verify changed files before merge: the PR should contain only `entries/v1/plugins/<bucket>/<plugin-id>.json` for that plugin.
   - On PR checks, expect registry validation to pass and generated-index build to be skipped for pull requests.
   - After merge, watch the base-branch registry workflow. The follow-up generated commit should touch only the expected generated files: `public/v1/manifest.json`, `public/v1/search/summaries.json`, `public/v1/shards/<bucket>.json`, and `public/v1/plugins/<bucket>/<plugin-id>.json`.
   - Validate the generated index with the GitHub contents API as the immediate source of truth: source entry, public detail entry, bucket shard, manifest `entryCount` and `availableBuckets`, and `search/summaries.json` must all agree on id, version, URL, SHA-256, size, compatibility, and dependency metadata.
   - Validate the public raw URLs after the workflow completes. GitHub raw branch URLs can lag briefly after a generated commit; if raw and contents API disagree right after merge, wait and retry before treating it as a registry index failure.
   - Download the public entry archive URL after merge and verify SHA-256, byte size, and root `locus.plugin.json` id/version against the generated public entry.
   - Download the plugin repository source archive or install from the GitHub repo after publishing. It must import as the same plugin id/version as the release asset.
   - To verify a version-only update after a plugin release, trigger or wait for registry CI, then inspect only `public/v1` generated outputs. Do not create or update `entries/v1` for that release unless registry metadata changed.
