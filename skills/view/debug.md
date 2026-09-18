---
title: View Runtime Debugging
tools:
  - execute_typescript
---

# View Runtime Debugging

Use the single `execute_typescript` TypeScript tool. Read [frontend-sdk.md](frontend-sdk.md) for the shared native frontend SDK.

```typescript
const panel = await locus.views.open("asset-editor");
await panel.wait({ condition: "runtimeReady" });
const snapshot = await panel.snapshot();
await panel.capture();
return { snapshot, logs: await panel.logs(20) };
```

Use locators from the snapshot to interact, then wait for the resulting state. Use `locus.ui` and `locus.workbench` for the native Locus interface. There are no separate View snapshot, action, wait, capture, console or eval tools.
