# macOS 分支测试收敛记录

执行目录：`F:\AGENT\locus-macos`；日期：2026-09-21。

## 最终结果

| 检查 | 结果 |
| --- | --- |
| `bun run test --reporter=json --outputFile=.tmp/vitest-macos-full-final.json` | 486 个测试文件，3,220 项：3,219 通过，0 失败，1 跳过；进程退出码 0 |
| 12 个换行兼容修复相关测试文件 | 111/111 通过 |
| `bun run typecheck:test` | 通过，退出码 0 |
| `bun scripts/verify-windows-ipc-preservation.mjs` | 12 项 Windows IPC 源代码保护检查通过 |

唯一跳过项是 `unityProfilerApi.test.ts` 的 `runs native Unity capture and lifecycle checks in an isolated project`，属于需真实 Unity 环境的集成检查；本轮未操作真实 Unity 项目。

原始输出保存在本机 ignored 目录：

- `.tmp/vitest-macos-full-final.json`、`.tmp/vitest-macos-full-final.log`：最终全套结果。
- `.tmp/vitest-macos-crlf-fixed.json`、`.tmp/vitest-macos-crlf-fixed.log`：换行修复的 111 项定向结果。
- `.tmp/typecheck-test-macos.log`：测试类型检查结果。

## 首轮失败分类与处理

首轮全套 3,220 项中 3,207 通过、12 失败、1 跳过，结果保留在 `.tmp/vitest-macos-full.json`。分类如下：

- 10 项属于 `core.autocrlf=true` worktree 的 CRLF 源码读取差异：现有测试用多行 LF 字符串、正则或 `indexOf` 匹配原始源码。只在各测试自身的 UTF-8 源码读取处增加 `.replace(/\r\n/g, "\n")`，未修改断言、产品源码或全局文件系统行为。
- 1 项属于独立 worktree 缺少本机 ignored 的 `AGENTS.md`：root 将原工作区文件原样复制作为现有测试所需 fixture，未加入版本控制。
- 1 项属于本轮构建入口分派的测试契约更新：`pythonRuntimeSettings.test.ts` 原断言绑定旧 `build:tauri` 字符串；platform owner 改为验证新平台分派及 Windows 继续执行原 `with_embed_python_git` 构建入口。

在全套测试前，另有 `nativeBridgeMigration.test.ts` 与 `unityBridgeCompatibility.test.ts` 的原始源码 CRLF 断言失败，已采用同样的本地读取归一化修复。

本轮换行修复共涉及以下 12 个测试文件：

```text
src/__tests__/nativeBridgeMigration.test.ts
src/__tests__/unityBridgeCompatibility.test.ts
src/__tests__/chatChangesFileContextMenu.test.ts
src/__tests__/chatResponsiveLayout.test.ts
src/__tests__/customProviderModalLayout.test.ts
src/__tests__/graphReadonlyDrag.test.ts
src/__tests__/knowledgeOverviewLayout.test.ts
src/__tests__/locusUnityCliDriver.test.ts
src/__tests__/onboardingCustomEndpointLayout.test.ts
src/__tests__/unityCaptureViewport.test.ts
src/__tests__/unityTestFrameworkTools.test.ts
src/__tests__/workbenchInteraction.test.ts
```

这些 Vitest 与类型检查结果验证 Windows 主机上的共享逻辑和源码契约，不等同于 macOS 应用实机运行、Unity dylib 加载、签名或公证验收。Rust 编译与 native 测试证据见 [Windows IPC 后端保护记录](windows-backend-preservation.md) 和主实施计划。
