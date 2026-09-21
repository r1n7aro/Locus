# macOS 并列后端：交叉编译验收记录

日期：2026-09-21。开发分支 `codex/macos-support`，工作目录 `F:\AGENT\locus-macos`。起点为 `cf70db9636ee61e74c779bec1472111ff389e00d`；本次修改尚未提交或推送。

## 编译与链接结果

| 组件 / 目标 | 检查 | 实际产物 / 结果 |
| --- | --- | --- |
| Locus / `aarch64-apple-darwin` | `cargo check` 通过 | `cargo build` 通过；Mach-O 64-bit arm64 executable |
| Locus / `x86_64-apple-darwin` | `cargo check` 通过 | `cargo build` 通过；Mach-O 64-bit x86_64 executable |
| native broker / `aarch64-apple-darwin` | 含 Mac 测试源码的目标检查通过 | release build 链接成功；Mach-O arm64 dylib |
| native broker / `x86_64-apple-darwin` | 含 Mac 测试源码的目标检查通过 | release build 链接成功；Mach-O x86_64 dylib |
| Locus / `x86_64-pc-windows-msvc` | `cargo check --locked --offline` 通过 | Windows 原 Unity bundle 构建流程成功，native 单测 8/8 |

应用目标检查和链接的 Cargo JSON 均返回 `build-finished: success=true`；验证命令退出 0。`file` 额外检查了两份应用的 Mach-O CPU 类型，结果与目标一致。应用本次使用 dev profile，native plugin 使用 release profile；这不是已打包、签名、公证的发行安装器。

构建环境为 Windows 本机 WSL Ubuntu + Rust 1.95.0 + Clang 19 + Rust LLD 22.1.2 + MacOSX14.5 SDK，部署目标 macOS 14.0。未修改日常 Windows 工具链配置，未参考旧 macOS 分支。安装及复用说明见 [构建环境](./macos-build-environment.md)。

## 产物

所有路径相对于本 worktree：

| 文件 | SHA256 |
| --- | --- |
| `src-tauri/target/aarch64-apple-darwin/debug/locus` | `e24c3a0947133577d63bb462b309bc699f38b01d1b6162e10940a6677ed7b8aa` |
| `src-tauri/target/x86_64-apple-darwin/debug/locus` | `c9a5e823b69f9f71d5a0cb6eb856f2b6f77e6be50c9f95707af3b7d5ecf40183` |
| `locus_native_plugin/target/aarch64-apple-darwin/release/liblocus_native.dylib` | `c5c6874f3ec341ade38b2b343d456b05c4c491da2330255511ced9f9eb29d5b9` |
| `locus_native_plugin/target/x86_64-apple-darwin/release/liblocus_native.dylib` | `3f1b6f1b7fbe7ec11366748ccb96fb8f6911ddcb577c249f88fb4999b96e3358` |

生成的 Mac native 文件位于 `src-tauri/gen/macos/native`，与 `src-tauri/macos/unity-native` 中的静态 importer 配置一起进入 Mac 专属 plugin staging。两种架构始终一同分发给 Unity，由 importer 匹配 Editor 进程架构；Windows 原 `locus_unity` 资源树不包含这些 Mac 文件。

## 本机测试和 Windows 保护

- 全套 Vitest：486 文件，3219 通过、0 失败、1 项真实 Unity 测试跳过。
- 应用 TypeScript 与测试 TypeScript 类型检查通过。
- 前端以 `VITE_LOCUS_TARGET_OS=macos`、`VITE_LOCUS_TARGET_ARCH=arm64` 构建成功，保留现有的大 chunk 提示；日志为 `.tmp/macos-frontend-build.log`。
- Windows native broker 单测 8/8；Mac 进程参数/身份/枚举返回值的可移植回归测试 6/6。
- 官方 .NET 10.0.9 的 osx-arm64 / osx-x64 包在 Windows 和 WSL 均成功提取；WSL 4 项测试包含 Unix 可执行权限、路径逃逸、链接拒绝和不完整包检查。
- Mac Unity 托管源码以 `UNITY_EDITOR_OSX` 条件和 Unity 6000.5.6f1 API 编译通过，不依赖 Detour / HotReload.Runtime。
- Windows IPC 保护脚本 12 项通过：原 named-pipe、native broker、hook、MMF、FFI/dispatch 和 C# Windows 预处理分支保留。
- Windows 平台保护脚本检查 24 个源/配置文件，并验证原命令、默认 installer flavor、更新选择、ORT 初始化和资源隔离。
- `git diff --check` 通过。没有关闭警告、使用伪造 Darwin cfg 或通过跳过依赖构建来获得通过结果。

原始 Cargo 日志位于本机 `F:\AGENT\.tmp\locus-macos-toolchain\app-all-check.{jsonl,err}`、`app-all-build.{jsonl,err}`；Windows 检查日志位于 worktree `.tmp/windows-app-check.{jsonl,err}`。Vitest 原始结果为 `.tmp/vitest-macos-full-final.json`。

## 功能边界与未执行的验收

已实现 macOS 独立 socket/broker/状态快照/进程管理、构建资源、.NET 与系统程序发现及更新选择。Windows 后端内部实现保留，公共入口只增加必要的平台分派和能力信息。

Mac 暂不开放后台 hook、热更、原生内存/栈探针、Unity 嵌入增强和本地 ORT embedding；相关入口明确返回不支持，远程 embedding 路径保留。禁用原生探针不影响 Mac broker 的状态观察 actor。

尚未执行 Mac 上的 WKWebView、Keychain、通知、Unity native plugin 实际加载、真实 socket 生命周期、domain reload、Rosetta Editor、完整工具闭环及签名公证。新增 Mac native 10 项行为/安全测试完成目标编译检查，但尚未在 Mac 执行。Windows 真实 Unity 集成也未在本轮运行。

进程终止采用逐 PID、UID、启动时间、映像和项目身份复核，拒绝不确定结果；普通 `kill(pid)` API 的检查与调用之间仍有时间间隙，不宣称原子防 PID 复用。
