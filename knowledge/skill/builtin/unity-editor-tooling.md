---
id: kd_skill_unity_editor_tooling
type: skill
path: builtin/unity-editor-tooling.md
title: Unity Editor UI Skill
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /unity-editor-tooling
argumentHint: <target data or workflow>
createdAt: 1775552858000
updatedAt: 1776383574781
---

# Unity Editor UI Skill

## Summary
Use when the task asks for a Unity Editor UI that edits scene data, serialized objects, assets, or project data inside the Unity Editor. This skill chooses the right host surface, defaults to UI Toolkit for new tools, preserves serialization, undo, and prefab correctness, and ships native-looking, production-ready editor workflows.

## Content
Use this skill when the task needs a reliable Unity Editor interface for editing data inside the Unity Editor.

Goal: ship editor tools that are fast to build, hard to misuse, visually native, and correct under undo, multi-object, and prefab workflows.

## When to use

- Build or improve a Unity editor tool that edits data inside the Unity Editor.
- Choose between a custom Inspector, EditorWindow, or PropertyDrawer.
- Choose between UI Toolkit, IMGUI, or an Odin-based editor extension path.
- Improve workflow speed, clarity, validation, preview, batch handling, or visual polish of an editor UI.

## When NOT to use

- Build runtime UI, in-game debug panels, or player-facing menus.
- Build a tool that does not need a Unity Editor UI surface.
- Modify import pipelines, build pipelines, scene handles, or menu-only commands unless the request also needs an Inspector or window UI.

## Core working model

Before coding, determine four things:

- what data is being edited
- where that data lives
- whether the workflow is single-target, multi-target, or batch-driven
- what editor foundations already exist in the project

Read the environment first. Reuse any exposed Unity version, scene state, package/dependency context, and current editor conventions.

Ask the user only when a workflow-critical detail is missing. At minimum, clarify only these when unknown:

- edited data type
- host object or asset
- selection-driven vs batch-driven workflow
- existing editor UI patterns to match

Do not ask for optional style preferences before making the main product decision.

## Version and dependency discipline

- Target the active Unity version first. Write code and choose controls that exist in that version.
- If version is unclear, choose the safest version-compatible path instead of assuming the newest APIs.
- Default to UI Toolkit for new editor UI.
- Use IMGUI when extending a mature IMGUI editor, when the tool is genuinely tiny, or when a required interaction is awkward in UI Toolkit.
- Use `IMGUIContainer` to isolate blocked legacy sections inside a UI Toolkit shell instead of rewriting the whole tool.
- If the project already uses Odin Inspector or Sirenix editor infrastructure, prefer reusing `OdinEditorWindow`, `OdinMenuEditorWindow`, drawers, validators, and `PropertyTree`-based flows instead of rebuilding the same plumbing.
- Ask before migrating a substantial existing IMGUI tool to UI Toolkit.

## Choose the host surface first

- Prefer a custom `Inspector` when the workflow centers on one selected `MonoBehaviour`, `ScriptableObject`, importer, or asset.
- Prefer an `EditorWindow` when the workflow needs search, list management, comparison, preview, cross-asset editing, multi-selection, auditing, or batch apply.
- Prefer a `PropertyDrawer` when the same custom field UI must be reused across multiple inspectors.
- Use a hybrid when both surfaces help: fast object-local edits stay in an Inspector, while heavy browsing or batch actions move into an EditorWindow.

## Default decisions when the user does not care

- One serialized target with mostly per-object edits: `UI Toolkit` + custom `Inspector`.
- Cross-asset editing, search, lists, previews, or batch apply: `UI Toolkit` + `EditorWindow`.
- One reusable field experience repeated across many types: `PropertyDrawer`.
- Existing `OnInspectorGUI()` or `OnGUI()` codebase with small scope: extend with `IMGUI`.
- Existing Odin-heavy project: reuse Odin for inspector-heavy and serialization-heavy flows unless the user explicitly asks for a pure Unity-native implementation.

## Match the request to a proven tool archetype

### 1. Detail inspector
Use for one selected target.

Preferred structure:

- header with title, object summary, and status
- primary settings section
- references or dependencies section
- preview or computed summary section
- warnings and fix hints near the affected fields
- advanced foldout
- actions footer

### 2. Collection manager
Use for assets, entries, or records that benefit from browsing and selection.

Preferred structure:

- toolbar with search, filters, refresh, and create actions
- `TwoPaneSplitView`
- `ListView` on the left
- detail, preview, or embedded inspector on the right
- footer or bottom action bar with selection count and apply actions

### 3. Batch audit or fixer
Use for validation, cleanup, migration, or repair workflows.

Preferred structure:

- scope and filters at the top
- explicit `Scan` action
- result list with counts and clear issue labels
- preview of affected objects or changes
- `Apply` action separated from `Scan`
- success, skipped, and failed result reporting

### 4. Reusable micro-editor
Use when a single field or nested type needs a better editor everywhere.

Preferred structure:

- compact single-row editing by default
- inline validation or mini-help when needed
- foldout or expanded mode only for advanced details

### 5. Wizard or guided flow
Use for sequential setup tasks.

Preferred structure:

- step list or tab bar
- one main action per step
- no deep foldout nesting
- clear next-state guidance after completion

## Build in this order

1. Write the UI tree in text before coding.
   - sections
   - lists and detail panes
   - validation surfaces
   - action zones
2. Decide file layout early.
   - `Editor/FooEditor.cs`
   - optional `FooEditor.uxml`
   - optional `FooEditor.uss`
   - optional row-template UXML for list items
3. Build the shell first.
4. Bind serialized data.
5. Wire validation and preview.
6. Wire actions and undo.
7. Add persistence and all non-happy-path states.
8. Polish spacing, alignment, labels, and iconography last.

## UXML, USS, and C# division of responsibility

- Put stable structure in UXML.
- Put stable styling in USS.
- Put dynamic behavior, data binding, and tool logic in C#.
- Use pure C# UI Toolkit for very small or highly dynamic tools.
- Keep custom USS semantic and restrained.
- Avoid large inline style blocks in C# except for trivial one-offs.
- Name key elements that code must query.
- Keep layout, styling, and logic separable when the tool is expected to live beyond a quick prototype.

## Serialization and data flow rules

- Prefer `SerializedObject`, `SerializedProperty`, `PropertyField`, and binding paths over direct field mutation.
- In custom inspectors, prefer `CreateInspectorGUI()`.
- In inspector UIs, let the Inspector perform its implicit binding after you return the visual tree.
- In editor windows that edit serialized targets, bind the root to a `SerializedObject` or host an `InspectorElement`.
- Prefer `PropertyField` for standard serialized members. Reach for custom controls only where they materially improve clarity, workflow, or validation.
- For lists, prefer `ListView` bound to a serialized list or a backing collection. Use row templates and `makeItem`/`bindItem` for custom visuals. Do not instantiate a full row tree per item manually for large lists.
- When polishing an existing inspector, consider embedding the default inspector inside a `Foldout` or a dedicated section with `InspectorElement.FillDefaultInspector()` before rewriting every field.
- Avoid ad hoc reflection traversal when a serialized path or an Odin property tree already exists.
- Query visual elements once and keep references; avoid repeated string queries inside high-frequency callbacks.

## Undo, prefab, and multi-object correctness

- SerializedObject-based editing is the first choice because it naturally supports undo, multi-object workflows, and prefab overrides.
- If direct mutation is unavoidable, call `Undo.RecordObject()` or `Undo.RegisterCompleteObjectUndo()` before changing data.
- Use the dedicated `Undo` APIs for creation, destruction, parenting, and component add/remove flows.
- Add `[CanEditMultipleObjects]` whenever the semantics are valid.
- When a UI supports multiple targets, make mixed states obvious and avoid pretending one target represents all of them.
- When direct edits touch prefab instances, record prefab instance property modifications where required.
- Do not bypass serialization for convenience unless the tradeoff is explicit and acceptable.

## Validation and user guidance

- Validate close to the field that caused the issue.
- Also keep one small top-level status summary when the tool benefits from a quick read.
- Use reactive tracking for warnings, badges, computed summaries, previews, and button enabled states.
- Disable apply or destructive actions when the current state is invalid, unchanged, or selectionless.
- For destructive or large batch operations, show a preview and affected count before execution.
- Separate `Scan` from `Apply` in audit-style tools.
- After execution, report exact counts: succeeded, skipped, failed.
- Prefer concise guidance and fix hints over long explanatory paragraphs.

## Visual polish rules

- Aim for native Unity Editor polish, not flashy runtime-game styling.
- Build strong hierarchy: header, content sections, warnings, actions.
- Do not dump raw fields into one flat form unless the data is truly trivial.
- Use one visual emphasis system only: a status chip, a restrained accent, or a clearly primary button. Keep the rest quiet.
- Favor clean spacing, strong labels, and consistent alignment over decorative styling.
- Use Unity-native controls first: `PropertyField`, `Foldout`, `HelpBox`, `Toolbar`, `ListView`, `TwoPaneSplitView`, `ScrollView`, and `InspectorElement`.
- Keep custom styling restrained. Avoid thick borders, gradients, saturated fills, giant cards, and game-menu aesthetics.
- Align custom controls with the native inspector rhythm. When custom fields sit beside native fields, use the same alignment classes and spacing conventions.
- Use `HelpBox` for warnings and guidance by default.
- Keep primary actions easy to find. Keep destructive actions visually separated.
- Test both editor themes before calling the tool polished.

## States every editor tool should handle

- empty state
- invalid selection state
- mixed multi-selection state
- loading or scanning state
- no-results state
- success and failure feedback after actions

## Persistence and reopen behavior

- Persist the UI state that matters to returning users.
- Use `viewDataKey` for foldouts, selections, tabs, trees, list selection, and scroll positions when supported.
- Use `SessionState` for session-only filters, search strings, and last-opened tabs when appropriate.
- Use `EditorPrefs` only for durable user preferences that should survive editor restarts and are not project data.
- On domain reload or window reopen, restore a useful working state instead of a blank reset whenever possible.

## Performance rules

- Do not rescan assets or rebuild expensive previews on every repaint or keystroke.
- Debounce search and filtering in windows that can touch many assets.
- Rebuild only the regions that changed; prefer data refresh over full visual-tree reconstruction.
- Keep `ListView` item creation cheap and bind logic deterministic.
- Delay expensive previews until the selection changes or the preview area becomes visible.
- Avoid expensive `AssetDatabase` work in high-frequency UI callbacks.
- For long scans or migrations, show progress and make reruns safe.

## Practical speed shortcuts

- Existing inspector, mostly default fields, but needs better summary and actions: keep default fields and wrap them in a stronger shell before rebuilding everything.
- Selection-driven browser or manager: start with `EditorWindow` + `Toolbar` + `TwoPaneSplitView` + `ListView` + detail pane.
- Reusable nested type editor: start with a `PropertyDrawer` before considering a full custom inspector.
- Tiny action tool: build a small native-looking window; avoid overengineering a framework-heavy solution.

## Common mistakes to avoid

- Writing direct field changes when a `SerializedProperty` path exists.
- Rebuilding an entire UI just to add one or two actions.
- Migrating mature IMGUI tools for style reasons alone.
- Hiding critical warnings inside collapsed advanced sections.
- Styling the tool heavily before workflow, validation, and undo are correct.
- Creating long flat inspectors when the data clearly wants hierarchy.
- Building large list tools without virtualization-friendly controls.
- Mixing layout, style, and behavior in one giant method for a tool that will be maintained.

## Escalate when the product choice changes workflow meaningfully

- Ask before migrating a substantial IMGUI tool to UI Toolkit.
- Ask before choosing Inspector vs EditorWindow when both are plausible and the wrong choice would reshape the workflow.
- Ask before replacing familiar project patterns with a new visual language.
- Ask before removing or collapsing advanced controls that users may depend on.

## Delivery contract

Every delivered editor UI should end with:

- chosen host surface and UI technology, with a one-line rationale
- target files added or changed
- edited data flow summary
- validation and preview behavior
- undo and redo behavior
- multi-object behavior
- prefab behavior
- persistence behavior across reload or reopen
- a concise manual verification checklist

## Verification checklist

- selection, empty, invalid, and mixed states render correctly
- undo and redo work for every mutation path
- serialized values survive domain reload
- multi-object editing works or is explicitly disabled
- prefab overrides are preserved
- window reopen restores a sensible working state
- lists remain usable with realistic data volume
- layout stays readable at narrow inspector widths and common window widths
- light and dark themes both read clearly
- no null-reference or missing-reference path silently breaks the UI
