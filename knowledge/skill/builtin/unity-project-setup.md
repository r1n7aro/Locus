---
id: kd_skill_unity_project_setup
type: skill
path: builtin/unity-project-setup.md
title: Unity Project Setup
injectMode: none
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /unity-project-setup
argumentHint: <focus area>
tools:
  - web_fetch
createdAt: 1775735250000
updatedAt: 1779840000000
---

# Unity Project Setup

## Summary
Interactively plan and scaffold a Unity project's code infrastructure, package choices, and initial code layout by combining targeted questions with the Unity environment already exposed in Locus.

## Content
## When to use

- Bootstrap a new Unity project with production-grade code infrastructure.
- Evaluate and install community best-practice third-party packages for a specific need (async, serialization, networking, etc.).
- Redesign or upgrade an existing project's foundational systems.
- The user says "set up my Unity project", "recommend packages", "I need an event system", or similar.

## When NOT to use

- The project already has mature infrastructure and the user just wants to add a feature or fix a bug.
- The request is about runtime gameplay code, shaders, or visual content, and the target is not infrastructure.
- The request is specifically about Editor tooling. Use `unity-editor-tooling` for that workflow.

## Instructions

### Phase 0 - Role & Skill Level (ask_user_question)

Before anything else, determine who you are talking to and adapt all subsequent communication.

**Step 1: Role**

```text
ask_user_question:
  question: "What is your role on the team?"
  options:
    - label: "Programmer"
      description: "Writes gameplay logic, system architecture, and tools"
    - label: "Designer"
      description: "Owns game design, tuning data, and level editing"
    - label: "Artist"
      description: "Owns models, animation, VFX, and UI visuals"
```

Adapt subsequent phases based on the role:

- **Programmer** -> proceed to Step 2 (skill level), then run all phases.
- **Designer** -> skip architecture questions. Focus on designer-friendly infrastructure: ScriptableObject-based events, configuration workflows, scene management. Use non-technical language throughout. Skip asmdef and IL2CPP topics.
- **Artist** -> focus on art-pipeline infrastructure: Addressables for asset management, shader variant management, animation system setup (DOTween / PrimeTween). Skip code architecture topics.

**Step 2: Skill Level (programmer role only)**

```text
ask_user_question:
  question: "How familiar are you with Unity and C#?"
  options:
    - label: "Beginner"
      description: "Just getting started with Unity"
    - label: "Experienced"
      description: "Has shipped or completed projects and understands basic architecture"
    - label: "Senior"
      description: "Knows asmdef, IL2CPP, Package Manager, and production constraints"
```

Adapt based on level:

- **Beginner** -> explain each concept in simple terms, recommend the minimum set of packages, skip asmdef layout.
- **Experienced** -> normal flow with recommendations and alternatives.
- **Senior** -> streamline Q&A and jump straight to package lists and install commands.

### Phase 1 - Project Profile

Do not ask about Unity version or input system when the environment already exposes them through project settings.

Ask about:

- **Project type**: mobile game, PC or console game, VR or XR app, tool or simulation, or prototype.
- **Team size**: solo, small team (2-5), or larger.
- **Render pipeline**: URP, HDRP, or Built-in. If the environment already detects one, present it as the default and let the user change it.
- **Architecture preferences** for programmer role: preferred pattern (MVC, MVP, ECS or DOTS, or none), existing frameworks in use, and asmdef usage.

Present infrastructure needs as a checklist and let the user pick:

- [ ] Event or messaging system
- [ ] Object pooling
- [ ] State machine
- [ ] Async or task utilities
- [ ] Serialization
- [ ] UI framework
- [ ] Addressables or asset management
- [ ] Scene management
- [ ] Input system
- [ ] Inspector enhancement
- [ ] Networking or multiplayer
- [ ] Tween animation
- [ ] Pathfinding
- [ ] Other

### Phase 2 - Recommendation & Implementation

There are two categories of infrastructure: third-party packages to install and subsystems the agent implements directly.

#### 2A. Third-party package recommendations

Only recommend third-party packages for the categories listed below. For each, provide:

- Source (UPM, OpenUPM, GitHub, or Asset Store)
- Exact install command or UPM URL
- Why this is the recommended choice in 2-3 sentences
- Tradeoffs or gotchas
- Current compatibility or pricing checks when the recommendation depends on a paid tier or store listing

Recommendation matrix:

| Need | Recommended | License | Alternative | Notes |
|------|-------------|---------|-------------|-------|
| Async / Tasks | UniTask (Cysharp) | Free | - | Strong default for async gameplay and tooling flows |
| Serialization | MessagePack-CSharp (Cysharp) | Free | Odin Serializer | Prefer Odin when the project already needs Odin-driven editor workflows |
| Addressables | Unity Addressables (official) | Free | YooAsset | Addressables is the default unless the project already standardized on YooAsset |
| Input | Unity Input System (official) | Free | Unity Legacy Input | Legacy Input fits only legacy projects or strict backward-compatibility constraints |
| Inspector Enhancement | Odin Inspector & Serializer | Paid | - | Recommend only when the editor productivity gain justifies the dependency |
| Networking | Mirror | Free | - | Good default for common Unity multiplayer setups |
| Tween Animation | DOTween / DOTween Pro | Free / Paid | PrimeTween | PrimeTween favors runtime performance and a smaller API surface |
| Pathfinding | A* Pathfinding Project | Free / Paid | Unity NavMesh | NavMesh is sufficient for many standard 3D projects |

Notes:

- Addressables and YooAsset are optional for simple projects.
- DOTween base is free. DOTween Pro adds paid editor tooling.
- A* Pathfinding has both free and paid tiers.
- PrimeTween has stronger raw performance and a smaller ecosystem.
- Verify the current package version, distribution channel, and paid tier details before presenting them as a final recommendation.

#### 2B. Agent-implemented subsystems

For infrastructure needs outside the table above, design and implement a lightweight solution directly.

- **Event System** -> implement with C# `event`/`delegate`, a ScriptableObject event bus, or a lightweight generic `EventBus<T>`.
- **Object Pooling** -> implement with `UnityEngine.Pool.ObjectPool<T>` (Unity 2021+) or a small generic wrapper.
- **State Machine** -> implement a lightweight FSM with a state interface, state machine class, and 2-3 example states.
- **Scene Management** -> implement an async scene loader with loading-screen support.
- **UI Framework** -> implement a simple view management layer on top of UI Toolkit.
- **Other** -> design and implement according to the user request.

Prefer concise self-implemented solutions. Use third-party packages only for the categories in the recommendation matrix.

### Phase 3 - Assembly Definition Layout (programmer only)

If the user opts into asmdef usage, propose a folder and asmdef structure:

```text
Assets/
├── _Project/
│   ├── Scripts/
│   │   ├── Runtime/
│   │   │   ├── Core/           (Core.asmdef - event bus, utilities)
│   │   │   ├── Gameplay/       (Gameplay.asmdef -> refs Core)
│   │   │   ├── UI/             (UI.asmdef -> refs Core)
│   │   │   └── Infrastructure/ (Infra.asmdef -> refs Core, third-party)
│   │   ├── Editor/
│   │   │   └── EditorTools/    (EditorTools.asmdef, Editor-only)
│   │   └── Tests/
│   │       ├── EditMode/       (Tests.EditMode.asmdef)
│   │       └── PlayMode/       (Tests.PlayMode.asmdef)
│   ├── Art/
│   ├── Audio/
│   ├── Prefabs/
│   ├── Scenes/
│   ├── Resources/
│   └── StreamingAssets/
├── Plugins/
└── Settings/
```

Adjust the structure to project scale. Solo projects can use a flatter layout.

### Phase 4 - Installation & Implementation Plan

Generate a numbered checklist that covers:

1. Third-party packages with exact install commands.
2. Agent-implemented subsystems with target file paths and short descriptions.
3. Assembly definition files and their references, when applicable.

### Phase 5 - Bootstrap Code

For each infrastructure system:

- Third-party packages: provide a minimal integration example to verify it works.
- Agent-implemented subsystems: provide the full implementation code in the correct project folder.

Place generated code inside the asmdef structure from Phase 3 when applicable.

### Phase 6 - Verification Checklist

- [ ] All packages resolve without console errors.
- [ ] Assembly definitions compile without circular references.
- [ ] A test scene runs with the bootstrap code wired up.
- [ ] Editor play mode enters and exits without errors.
- [ ] IL2CPP build, when applicable, completes without stripping issues. Generate a `link.xml` when needed.

## General Principles

- Prefer UPM or OpenUPM over manual `.unitypackage` imports.
- Prefer self-implemented concise systems for needs outside the recommendation matrix.
- Keep prototypes minimal and note what can be added later.
- Respect existing project choices and frameworks.
- Check Unity version compatibility before recommending packages.
- Clearly mark paid packages and always offer free alternatives when they exist.
