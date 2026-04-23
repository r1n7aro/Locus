---
name: create-releases
description: Draft and execute a GitHub release workflow for this repository. Use when Codex needs to detect the last published release on main, analyze the commits since that release, write a GitHub-style changelog grouped into Added, Fixed, and Removed, present a release plan for approval, sync the required app and docs version files after the user provides the target version, validate localized release documents, and then create and push the GitHub release only after explicit approval.
---

# Create Releases

## Overview

Use this skill as a gated release operator. Establish the release baseline first, write the changelog second, edit versioned files only after the user approves the plan, and publish only after the user approves the final diff and validation results.

## Workflow

### 1. Establish the release baseline

Inspect the repository state before making assumptions.

- Read `git status --short` and keep unrelated workspace changes intact.
- Resolve `origin/main` and use it as the release reference branch unless the user explicitly says otherwise.
- Detect the latest published release from tags reachable from `origin/main`.
- Prefer SemVer-like tags such as `v0.1.0`, `v1.2.3`, or prerelease tags derived from the same pattern.
- Resolve the release commit with `git rev-list -n 1 <tag>`.

Use this fallback order:

1. Latest SemVer-like tag merged into `origin/main`
2. Latest GitHub release tag if tag metadata is available but missing locally
3. Initial-release mode when neither exists

When initial-release mode is triggered, report that no previous published release could be detected and ask the user to confirm that the upcoming release should be treated as the first public release before drafting final notes.

### 2. Analyze the change range

Build the comparison range from the detected release commit to the target commit.

- Default target commit to `HEAD`.
- Collect both commit-level and file-level evidence.
- Use `git log --reverse <base>..HEAD` to understand intent chronologically.
- Use `git diff --stat <base>..HEAD` and targeted file inspection to verify what actually changed.
- Avoid inferring product behavior from commit subjects alone.

For changelog grouping:

- Put user-visible capabilities and workflows under `Added`.
- Put corrections, regressions, stability work, and polish under `Fixed`.
- Put removed workflows, removed UI, removed commands, or removed compatibility paths under `Removed`.

Keep the changelog factual. Do not invent benefits that are not supported by the diff.

### 3. Draft the release plan and changelog

Before touching any version file, report a concise plan in list form.

The report should include:

- Detected previous release tag and commit hash
- Target commit hash
- Diff range
- Draft GitHub-style changelog with `Added`, `Fixed`, and `Removed`
- Planned files to update
- Planned validation commands
- Planned publish actions

Then stop and wait for the user to approve the plan and provide the target version number.

### 4. Sync the required release files after approval

Once the user approves and specifies the version, update the required files for this repository.

Required version sync targets:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`
- `docs/overview/latest-version.mdx`
- `docs/en/overview/latest-version.mdx`

Generated output:

- `docs/update.txt`

Apply these rules while editing:

- Keep `version`, `releasedAt`, and `channel` aligned across localized `latest-version.mdx` documents.
- Keep `changelogUrl` locale-correct.
- Update both localized changelog bodies to reflect the same release facts.
- Generate `docs/update.txt` from the docs script instead of hand-maintaining divergent JSON when the repository already provides a generator.

For this repository, use the existing release commands when possible:

```powershell
bun run release
```

That command is expected to regenerate `docs/update.txt`, verify version consistency, and validate docs metadata.

### 5. Validate before publish

After editing, validate the release state and summarize the outcome.

Minimum validation:

- `bun run release`

Recommended additional checks when preparing the public release:

- `bun test`
- `bun tauri build`

When `bun tauri build` succeeds, collect the release artifact paths from:

- `src-tauri/target/release/bundle/nsis/`

If validation fails, stop before publish and report the failure clearly.

### 6. Present the final release summary and wait for approval

After edits and validation, report:

- Final version number
- Modified files
- Validation results
- Final changelog body to use for GitHub Release
- Release assets that are ready to upload

Then wait for explicit user approval before creating the release, pushing commits, or pushing tags.

### 7. Publish the release

After final approval:

- Commit the release changes intentionally
- Create or update the release tag using the approved version, normally `v<version>`
- Push the branch and the tag
- Create the GitHub Release using the validated changelog body
- Upload the built installer assets

Prefer the GitHub plugin or `gh` CLI when available. If both are available, prefer the workflow that can attach assets reliably in the current environment.

### 8. Safeguards

- Do not overwrite unrelated user changes.
- Do not publish from an unvalidated state.
- Do not guess the target version.
- Do not create the GitHub Release before the version files and localized docs are aligned.
- Do not push a release tag until the user has approved the post-edit summary.
- Prefer exact dates in `releasedAt`.

## Repo-specific notes

This repository currently uses these release-related mechanics:

- App runtime version is surfaced from Tauri first, with `package.json` as fallback.
- Docs release metadata is parsed from `docs/overview/latest-version.mdx` and `docs/en/overview/latest-version.mdx`.
- `docs/update.txt` is generated from docs release notes, not authored independently.
- The updater checks `https://unity.farlocus.com/update.txt`, so publishing docs metadata and GitHub Release assets should stay coordinated.

## Example prompts

- `Use $create-releases to draft the next release plan from the last published tag on main.`
- `Use $create-releases to prepare version 0.1.2, update the release files, and wait for my approval before publishing.`
- `Use $create-releases to compare the previous release on main with HEAD and write the changelog first.`
