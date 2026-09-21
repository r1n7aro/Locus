# Windows 构建与运行时保护证据

基线：`cf70db9636ee61e74c779bec1472111ff389e00d`。本记录补充 `windows-backend-preservation.md` 中的通信后端保护检查。

## 可重复检查

```powershell
bun run scripts/verify-windows-platform-preservation.mjs
bun run test src/__tests__/appUpdate.test.ts src/__tests__/appUpdateStore.test.ts src/__tests__/appUpdateLayout.test.ts src/__tests__/macosBuildTarget.test.ts src/__tests__/pythonRuntimeSettings.test.ts
bun run typecheck:test
```

保护脚本从 Git 基线直接读取原文，统一换行后比较 24 个源文件或配置文件。对于公共入口，仅移除列明的新增 macOS 条件分派再比较完整原文件；任何原候选路径、优先级或 Windows 函数体变化均会导致失败。

- Windows `.NET` 下载、ZIP 提取、缓存与系统探测实现全文保持不变。
- Tauri 原配置及完整/精简安装包 flavor 保持不变；既有构建命令均保持原值。`build:tauri` 新增分派，Windows 仍运行原 `build:tauri:with_embed_python_git`。
- Unity 托管依赖、Windows native 构建、Compile Server、ORT、Python、Git、GitHub CLI、许可证和 release installer 原生成器保持不变。
- Agent、Knowledge、Skills、Python SDK 和 Compile Server 的 Windows 资源候选顺序保持不变。
- Git/gh 原发现实现、Windows 更新包选择函数和 Windows ORT 初始化函数保持不变。
- macOS dylib 和 Unity importer metadata 均位于独立目录，不进入 Windows 配置整体打包的 `locus_unity` 源树。

## macOS 平台边界核对

Finder 启动时，Git/gh 除环境覆盖与 PATH 外增加 `/opt/homebrew/bin`、`/usr/local/bin`、`/opt/local/bin`，Git另检查 `/usr/bin/git`；候选仍通过现有版本探测验证。Python 额外检查 Homebrew、`/usr/local/bin`、Python.framework 和系统路径。.NET 额外检查官方安装目录及 Homebrew 路径，未找到受支持的系统运行时才下载所需 Darwin 架构。

静态 native metadata 保存于 `src-tauri/macos/unity-native`，生成 dylib 保存于 `src-tauri/gen/macos/native`，Mac 准备脚本把二者组装到独立插件 staging；每个 Mac 包均包含 ARM64 和 Intel 两份 broker，由 Unity importer 选择 Editor 架构。

Windows 宿主发布 Compile Server 时可能生成一个 `.exe` apphost。两种平台的 Compile Server 和 C# LSP 启动路径均明确执行所解析的 `dotnet`，将对应 `.dll` 作为参数；macOS 不执行这个 Windows apphost。

## 验证边界

2026-09-21：上述源保护脚本通过；平台、更新及 Python 构建入口的 37 项 Vitest 测试通过，测试类型检查通过。Mac 资源准备使用已交叉编译的真实双架构 dylib 验证并完成 staging。

这些检查证明原实现保留及分派契约成立，不等同于完整 Windows Unity 集成回归。Finder 下的工具发现、macOS 应用运行和 Unity 加载仍需要 macOS 实机验证。
