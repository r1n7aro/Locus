# macOS 适配扫描与 Windows 本机实施范围

扫描日期：2026-09-21。代码基线：`main@cf70db9636ee61e74c779bec1472111ff389e00d`（v0.8.3）。

本文保留适配前的扫描基线；后续实现和验证进度见 [实施计划](./macos-implementation-plan.md)。

目标架构：Apple Silicon（`aarch64-apple-darwin`）和 Intel（`x86_64-apple-darwin`）。本次只扫描、分析和建立适配分支，不修改产品实现，不参考旧 macOS 分支。

独立分支为 `codex/macos-support`，worktree 为 `F:\AGENT\locus-macos`。原工作目录 `F:\AGENT\locus` 保留在 `main`。

## 1. 结论

**可以沿用现有条件编译体系适配 macOS，但当前版本不能直接通过现有开发、打包入口得到可用的 macOS Unity Dev Agent。**

必须分别看三个层次：

| 层次 | 当前结论 | 证据边界 |
| --- | --- | --- |
| Rust 条件编译基础 | 已有较多 Windows / Unix / macOS 分支，适合继续扩展 | Windows 系统依赖已按 target 隔离；不能据此证明 Darwin 全量编译通过 |
| 默认开发和构建链 | 存在确定的平台阻塞 | ORT 准备脚本固定 Windows DLL / PowerShell；原生插件脚本固定 `.dll`；托管程序集打包直接启动 `ILRepack.exe`；安装器固定 NSIS |
| macOS 实际功能 | Unity 核心闭环尚未实现 | Unity 传输明确返回“不支持 Windows 以外平台”，原生 broker 的非 Windows 初始化返回 `-1`；C# 服务也被平台判断禁用 |

所以工作量不只是添加几个 `#[cfg(target_os = "macos")]`。需要同时处理构建资源、运行时发现、Unity IPC / 进程身份、能力声明、安装包和发布入口。

进一步的改动规模、通信/hook 耦合与 Windows 回归风险分析见 [Windows 回归风险评估](./macos-windows-regression-risk-2026-09-21.md)。首阶段建议保留 Windows 后端实现，仅增加独立 macOS 后端；共享状态机的大规模提取延后为单独重构。

本次没有执行 Darwin `cargo check`、链接或 Mac 实机测试。尚未发现并通过编译器确认的 Darwin Rust 语法/类型错误清单；下文“确定阻塞”主要来自已核对的脚本与运行时分支，不冒充目标平台编译结果。

## 2. 已有的跨平台基础

| 模块 | 已有实现 | 仍需注意 |
| --- | --- | --- |
| Rust 依赖隔离 | [Cargo.toml](../../src-tauri/Cargo.toml) L120 / L130 将 `windows`、`webview2-com`、`winreg`、`junction` 等置于 Windows target，Unix 单独依赖 `libc` | 还需在两个 Darwin target 上核对完整依赖图和 native build scripts |
| 窗口原生增强 | [lib.rs](../../src-tauri/src/lib.rs) 的 `windows_resize_sync` / `windows_window_frame` 模块按 Windows 编译 | 非 Windows 不加载 Win32 增强，不等于窗口体验已经验证 |
| 子进程与 Shell | [process_util.rs](../../src-tauri/src/process_util.rs) L249 / L365 有 Unix 进程组与终止实现；[shell.rs](../../src-tauri/src/tool/builtins/shell.rs) 有 macOS 分支 | Finder 启动后的 PATH、取消任务、子孙进程退出需要 Mac 验收 |
| Unity 安装路径 | [unity_bridge/mod.rs](../../src-tauri/src/unity_bridge/mod.rs) L1404 / L1495 已识别 `Unity.app/Contents/MacOS/Unity` 与 `/Applications/Unity/Hub/Editor` | 能发现/启动编辑器，不代表可以连接、确认进程身份或执行工具 |
| Unity 日志 | [editor_log.rs](../../src-tauri/src/unity_bridge/editor_log.rs) L215 已提供 `~/Library/Logs/Unity/Editor.log` | 需验证多项目、多个 Editor 和日志归属 |
| Git / Python | `process_util.rs` 有非 Windows `git` 发现；[python_runtime.rs](../../src-tauri/src/python_runtime.rs) L981 起支持系统 `python3` / `python` | 内置运行时仍偏 Windows；GUI PATH 不完整时系统发现可能失败 |
| GitHub CLI | [prepare-managed-github-cli.mjs](../../scripts/prepare-managed-github-cli.mjs) L42 / L49 已有 Darwin x64 / arm64 下载项；Rust 端有对应 runtime ID | 脚本按宿主 `process.platform/process.arch` 选择，尚不能直接代表跨目标或 universal 构建 |
| 快捷键 | [useKeyboardShortcuts.ts](../../src/composables/useKeyboardShortcuts.ts) 已区分 Mac 的 Cmd；聊天输入支持 `metaKey` | [App.vue](../../src/App.vue) L386 仍有只判断 `ctrlKey` 的入口，需继续收敛 |
| 文件与存储通用逻辑 | 多处软链接、权限、原子写入已有 Unix 分支，数据库和会话处理并非整体绑定 Win32 | 大小写、符号链接、文件监听、资源目录与可写目录必须单独验收 |

注意：`Locus.Roslyn.dll`、`Locus.Json.dll`、`Locus.Detour.dll`、`LocusCompileServer.dll` 是托管程序集，不能因为扩展名是 `.dll` 就统一判定为 Windows 原生库。当前明确绑定 Windows 的是 `locus_native.dll`、ORT/DirectML 等原生产物及部分启动方式。

## 3. 确定的阻塞与功能缺口

优先级含义：P0 阻塞构建或 Unity 基础闭环；P1 影响安装后完整使用；P2 是可单独规划的原生增强。P2 不表示无需处理，而是应明确报告能力，不能静默声称成功。

### 3.1 构建与打包

| 优先级 | 位置 | 当前问题 | 适配方向 |
| --- | --- | --- | --- |
| P0 | [run-tauri.mjs](../../scripts/run-tauri.mjs) L32；[prepare-ort-runtime.mjs](../../scripts/prepare-ort-runtime.mjs) L10–15 / L245 | `bun tauri dev` 必跑 `ort:bundle`；脚本固定 `win-x64`、DirectML、Windows DLL，并用 `powershell.exe` 解包。干净 Mac 环境无法按此流程准备依赖 | 统一平台/目标架构映射；Windows 保留 DirectML，macOS 准备匹配版本的 CPU ORT 动态库或明确禁用本地嵌入功能 |
| P0 | [build-locus-native-plugin.mjs](../../scripts/build-locus-native-plugin.mjs) L18；[插件 meta](../../locus_unity/Editor/Native/x86_64/locus_native.dll.meta) | 构建后仅查找 `target/release/locus_native.dll`，复制到 `Native/x86_64`；Unity importer 限定 Windows / x86_64 | 支持 Darwin 原生产物与 Unity importer，且必须实现 broker，不能只生成现有空壳 |
| P0 | [Roslyn 打包](../../scripts/build-locus-roslyn-bundle.mjs) L178；[JSON 打包](../../scripts/build-locus-json-bundle.mjs) L176；[Detour 打包](../../scripts/build-locus-detour-bundle.mjs) L194 | 解包有 Unix 分支，但随后仍直接 `execFileSync` 启动 `ILRepack.exe`，未配置非 Windows 托管运行方式 | 选定可跨平台的合并执行方式；也可将受控、可复现的托管产物构建与目标 native 产物构建拆开 |
| P0 | [tauri.conf.json](../../src-tauri/tauri.conf.json) L34；[build-release-installers.mjs](../../scripts/build-release-installers.mjs) | 目标固定 `nsis`，安装器查找和重命名固定 `x64-setup.exe` | 拆分 Windows/macOS 配置，加入 `.app` / `.dmg` 构建与架构产物命名 |
| P1 | [prepare-managed-python.mjs](../../scripts/prepare-managed-python.mjs) L148；[prepare-managed-git.mjs](../../scripts/prepare-managed-git.mjs) L208；[两个 flavor 配置](../../src-tauri/tauri.with_embed_python_git.conf.json) | 非 Windows 跳过内置 Python/Git；默认 build wrapper 仍追加内置 flavor 配置，其资源路径假定产物存在 | 首阶段可选择系统 Git/Python 路线，但要同步调整资源配置、检测和产品能力；内置版另做两个架构的运行时供应 |
| P1 | [generate-third-party-bundle.mjs](../../scripts/generate-third-party-bundle.mjs) L24 | 原生依赖清单固定记录 Windows ORT/DirectML 文件 | 按实际分发的目标产物生成 notices / manifest，避免错误包含或漏记 |

### 3.2 Unity 核心闭环

| 优先级 | 位置 | 当前行为 | 影响 |
| --- | --- | --- | --- |
| P0 | [transport.rs](../../src-tauri/src/unity_bridge/transport.rs) L1191–1254 | 非 Windows 的发送入口全部报 `Unity bridge is only supported on Windows (named pipes)` | 无法通过桌面应用连接 Unity 并执行工具 |
| P0 | [native broker](../../locus_native_plugin/src/lib.rs) L89 / L2137；[Cargo.toml](../../locus_native_plugin/Cargo.toml) L17 | 真正的 broker、队列、心跳、共享状态和传输均在 Windows 模块内；非 Windows `locus_init` 返回 `-1` | 给 dylib 换文件名或补构建目标不会产生有效通信能力 |
| P0 | [LocusBridge.Native.cs](../../locus_unity/Editor/LocusBridge.Native.cs) L39 / L673；[mod.rs](../../src-tauri/src/unity_bridge/mod.rs) L1267 / L1338 | endpoint 固定 Windows named pipe；路径归一化包含转反斜杠、转小写；C# 侧明确要求 native broker | 必须同步改 Rust 客户端、native broker、C# 启动参数和项目身份规则；不能假定存在可直接启用的 managed fallback |
| P0 | [process.rs](../../src-tauri/src/unity_bridge/process.rs) L285 / L320 / L646 | 非 Windows 进程活性/身份报不支持，查询为 unknown；关闭项目 Editor 返回空列表 | 连接管理、旧进程识别、插件替换、取消/重启流程都缺乏完整依据 |
| P0 | [plugin.rs](../../src-tauri/src/unity_bridge/plugin.rs) L35 / L49 | 必需文件及 DLL 更新判断固定包含 Windows native DLL | 安装完整性校验、升级判断及锁定处理需支持 Mac 原生插件，不能只改复制脚本 |
| P2 | [state_probe.rs](../../src-tauri/src/unity_bridge/state_probe.rs) L486 / L3019；[background_hook.rs](../../src-tauri/src/unity_bridge/background_hook.rs) | 原生状态探针、Windows 内存/模块/符号读取、后台 hook 在非 Windows 上不可用 | 状态可观测性和后台行为不能按 Windows 等价宣称 |
| P2 | [unity_embed.rs](../../src-tauri/src/commands/unity_embed.rs) L2722；[shared_workbench_window.rs](../../src-tauri/src/shared_workbench_window.rs) L136 | Mac 不启动嵌入控制 pipe；跨窗口原生拖动跟踪只是等待循环，不发送位置事件 | Unity 嵌入面板、浮窗吸附/拖动等需要单独的平台实现或明确降级 |

建议首阶段保留 Windows named-pipe 后端的内部实现，通过现有入口为 macOS 增加独立本机 IPC 后端，优先评估 Unix domain socket。请求队列、超时、取消、接收确认、generation、domain reload 生命周期的公共提取应作为后续独立重构，而不与首次平台适配合并进行。共享状态面必须一起设计，不能只替换主连接。此为实施建议，尚未落地。

IPC 至少需要覆盖：同用户访问限制、短 endpoint 路径、项目与进程唯一性、残留 endpoint 清理、并发连接、请求上限、domain reload 期间状态、重连与取消。不要直接开放无认证的网络监听来替代本地桥接。

### 3.3 运行时与安装后的资源

| 优先级 | 位置 | 当前问题 | 适配方向 |
| --- | --- | --- | --- |
| P0 | [dotnet_runtime.rs](../../src-tauri/src/dotnet_runtime.rs) L41 / L244 / L284 | RID 只有 `win-x64` / `win-arm64`；系统 `dotnet` 探测前就要求有效 RID | 增加 `osx-arm64` / `osx-x64`，支持 tar.gz、执行权限和系统运行时优先；当前“Mac 已装 .NET”也绕不过门控 |
| P0 | [csharp_lsp/mod.rs](../../src-tauri/src/csharp_lsp/mod.rs) L675；[csharp_compile/manager.rs](../../src-tauri/src/csharp_compile/manager.rs) L137 | LSP 和编译服务依赖上述 supported 判断 | 代码分析、编译以及依赖它们的执行/热更流程必须一起恢复 |
| P1 | [lib.rs](../../src-tauri/src/lib.rs) L821；[knowledge.rs](../../src-tauri/src/commands/knowledge.rs) L537；[skill.rs](../../src-tauri/src/commands/skill.rs) L1105；[plugin.rs](../../src-tauri/src/unity_bridge/plugin.rs) L69；[manager.rs](../../src-tauri/src/csharp_compile/manager.rs) L67 | Agent、knowledge、skills、Unity 插件和编译服务多处只查可执行文件旁边及开发目录，缺少统一 Tauri resource root | 将 `app.path().resource_dir()` 传入统一资源解析；开发目录 fallback 保持仅开发态启用 |
| P1 | [storage.rs](../../src-tauri/src/commands/storage.rs) L88 | 带资源的 exe 目录会被视为 portable 根，尝试用其下 `data` | macOS 明确区分应用包只读资源与用户可写数据；当前不应把 bundle 内目录当数据迁移目标 |
| P1 | [embedding.rs](../../src-tauri/src/knowledge_index/embedding.rs) L498 / L5080 | 非 Windows ORT 初始化是空操作；CPU 构建路径存在，但未准备/定位 Mac 动态库；GPU 计划无 Mac 实现 | 可以先完成 CPU ORT，远程 embedding 仍单独验证；不能将 DirectML 选项照搬为 Mac GPU 支持 |
| P1 | [python_runtime.rs](../../src-tauri/src/python_runtime.rs) L906 / L934 | managed 路径固定 `windows-x64`，非 Windows 平台 ID 只有 `default` | 系统 Python 路线先可用，内置版另补目标目录、解释器与 package cache 规则 |
| P1 | `process_util.rs` / `python_runtime.rs` / `dotnet_runtime.rs` | 大量程序发现依赖当前 PATH，未见统一 macOS GUI 启动路径恢复 | 覆盖 Finder 启动与命令行启动，确认 Git/Python/dotnet/外部 CLI 的实际发现结果 |

资源目录问题是根据现有路径解析与 Tauri 的 macOS 应用包布局作出的静态判断：应用二进制在 `Contents/MacOS`，打包资源在 `Contents/Resources`。此外，GUI 应用不会自动继承用户 Shell dotfiles 中的 PATH。[Tauri 官方应用包说明](https://v2.tauri.app/distribute/macos-application-bundle/)

### 3.4 调试、界面与分发

| 优先级 | 位置 | 当前问题 | 适配方向 |
| --- | --- | --- | --- |
| P1 | [appUpdate.ts](../../src/services/appUpdate.ts) L280；[release-notes.mjs](../scripts/release-notes.mjs) L223 | 更新选择固定优先 Windows，发布下载项也是 Windows x64 | 按 OS / CPU 选择，支持 universal 元数据；找不到匹配产物时不能回退为错误平台安装器 |
| P1 | [tauri.conf.json](../../src-tauri/tauri.conf.json)；`lib.rs` 窗口/托盘事件 | 无专用 macOS 配置；自绘无边框窗口、关闭到托盘按现有通用逻辑运行 | 在已有桌面风格下处理 Cmd 快捷键、菜单、Dock 激活、关闭/退出、Retina、多显示器与中文输入 |
| P1 | [runtime_paths.rs](../../src-tauri/src/runtime_paths.rs) L11；[run-tauri.mjs](../../scripts/run-tauri.mjs) L116 | 隔离 WebView 数据设置仍使用 `WEBVIEW2_USER_DATA_FOLDER`，临时目录只设置 TEMP/TMP | macOS 需要实际有效的 WebKit profile / 数据隔离及临时目录规则，不能只输出“isolated”日志 |
| P2 | [cdp_debug.rs](../../src-tauri/src/cdp_debug.rs) L1821；[view.rs](../../src-tauri/src/view.rs) L7750 | 非 Windows CDP reconcile 返回 None；前端截图报 requires WebView2 | 单独规划 macOS 调试/截图；现有 `dev-mcp` / WebView2 测试入口不能作为 Mac 验收通道 |
| P2 | [prepare-renderdoc-runtime.mjs](../../scripts/prepare-renderdoc-runtime.mjs) L171；[locus_renderdoc.py](../../skills/graphics-debugger/scripts/locus_renderdoc.py) L501；[skill_runtime_context.rs](../../src-tauri/src/skill_runtime_context.rs) L190 | RenderDoc bundle 为 Windows x64，worker 查找 `qrenderdoc.exe`；原生调试发现面向 CDB/WinDbg | capability 要反映实际可用性；Mac 图形/原生调试是独立适配项 |

macOS 的 Tauri WebView 是 WKWebView，现有 Chromium/WebView2 CDP 实现不能仅更换端口复用。[Tauri WebView 平台说明](https://v2.tauri.app/reference/webview-versions/)

补充核查：Unity viewport 截图在 Windows 原生实现外仍有 `ReadScreenPixel` / RenderTexture 路径，不能把所有截图能力都判为 Windows 专用；但屏幕权限、遮挡、Retina 坐标和结果正确性需要 Mac 实测。热更的 MonoMod detour 同样需要分别验证 Intel 与 ARM64 的 Unity/Mono 行为。

## 4. Windows 本机能推进到什么程度

**本机可以完成平台抽象、绝大部分共享逻辑与脚本改造，并验证 Windows 不回归；不能独立完成 macOS 原生验收。**

| 工作包 | Windows 本机可以完成 | 本机可提供的有效证据 | 仍需 Mac 的部分 |
| --- | --- | --- | --- |
| 构建目标解析 | 显式区分 host / target / architecture，统一下载和资源矩阵，修正 Tauri flavor 合并、installer 命名 | Vitest 测试目标组合、资源清单、错误输入；Windows 真实 dev/build 回归 | Darwin native build scripts、链接、`.app` / `.dmg` 产物 |
| 资源解析 | 统一 resource root；Agent/skill/knowledge/plugin/sidecar 使用同一入口 | 临时目录模拟 `Contents/MacOS` 与 `Contents/Resources`，测试开发/安装布局选择 | 从 Finder 启动签名应用后的路径、权限和资源加载 |
| .NET / Python / Git / gh | RID 映射、下载校验、tar.gz 解包逻辑、缓存/manifest、程序发现、能力状态 | 测试 archive fixture、缓存命中、下载错误、目标路径；Windows 运行时回归 | Unix 可执行位、动态库、GUI PATH、ARM64 / x64 程序实际执行 |
| 更新下载 | 按 OS/arch 选择安装包，修正发布 JSON 结构与测试 | 三平台与两种 Mac 架构的输入矩阵，缺少产物时的行为 | Mac 安装/更新、实际替换与首次启动 |
| Unity broker 重构 | 提取平台无关协议、队列、生命周期；保留 Windows backend；设计 Mac endpoint 和状态面 | Windows named-pipe 集成测试；内存/模拟流测试超时、取消、重连、generation 和限额 | Unix socket、共享状态、native dylib 加载、domain reload、异常退出 |
| Unity 进程管理 | 抽象进程身份接口，保持 PID + 创建时间校验；整理启动/关闭/升级状态机 | fake process facts 的状态转换测试；现有 Windows Unity CLI driver 回归 | Mac 真实进程枚举、身份与项目绑定、正常退出/强制退出、插件锁定 |
| UI 能力与键盘 | 复用既有快捷键及设置控件，按 backend capability 展示；处理只认 Ctrl 的入口 | 参数化平台测试和现有前端类型检查 | WKWebView、输入法、窗口行为、拖放、通知、菜单与 Dock |
| 发布与 CI 配置 | 编写两个 Darwin target 的流程、资源校验、测试与签名步骤定义 | 配置和脚本测试，Windows job 回归 | 真正执行 macOS runner、签名、公证与安装验收 |

这里的“可以完成”指实现与本机能覆盖的验证，不把模拟测试记成真实 macOS 支持。尤其是 `#[cfg(target_os = "macos")]` 内部代码，Windows 常规构建不会检查它；`cargo fmt`、前端 typecheck 或 mock 通过也无法弥补这一点。

Rust 支持 Darwin 交叉编译，但涉及 C/native 库和链接时仍需要合适的 Clang / Apple SDK。本仓库还依赖 Tauri、WebKit、系统库和原生产物；只执行 `rustup target add` 不构成完整 macOS 工具链，也不产生 Mac 运行结果。因此不建议把本轮工程前置工作变成 Windows 上搭建整套 Apple 交叉工具链。[Rust 官方 Darwin target 说明](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html)

如果随后使用远程 Mac / macOS CI，可继续从这台 Windows 编写和发起构建；应明确记录验证实际发生在 Mac。WSL/Linux 可辅助验证可移植协议或 Unix 分支，但不能替代 Darwin、WKWebView 和 Apple 原生 API 的验证。

## 5. 两种 Mac 架构的边界

| 对象 | Apple Silicon | Intel | 要点 |
| --- | --- | --- | --- |
| Locus Rust binary | `aarch64-apple-darwin` | `x86_64-apple-darwin` | 先独立构建和验收两者，之后再决定是否合并 universal 包 |
| .NET runtime | `osx-arm64` | `osx-x64` | 下载、解包、执行、缓存要按实际运行架构区分 |
| ORT / gh / 可选 Python | arm64 原生产物 | x64 原生产物 | 不能只根据打包机器的 `process.arch` 选资源 |
| Unity native broker | 与 Unity Editor 进程架构一致 | 与 Unity Editor 进程架构一致 | 插件载入 Unity 进程，不能按 Locus 自身架构盲选 |
| C# managed bundles | 共享产物候选 | 共享产物候选 | 依赖 native detour / Unity API 的行为仍需分别验证 |

特别注意 Apple Silicon 上运行 Intel Unity 的情况：Locus 可能为 arm64，而 Editor 为 x64。建议评估 universal native plugin，或显式按 Editor 架构安装；不能只把 Locus 主程序做成 universal 就认为它携带的所有运行时都已覆盖双架构。Unity 官方支持 Mac native plugin 的 universal 或分架构产物，具体 importer 配置仍需按支持的 Unity 版本实测。[Unity 桌面原生插件说明](https://docs.unity3d.com/cn/6000.0/Manual/plug-ins-for-desktop.html)

最低 macOS 版本、最低 Unity 版本及是否承诺 Rosetta 组合尚未确定，应在选定 native 依赖版本和开始 Mac 验收前固定。不要只依据 Rust 或 Tauri 单个组件的最低系统版本作产品承诺。

## 6. 建议实施顺序与完成标准

### M1：构建、启动与基础桌面能力

先在 Windows 做目标矩阵、构建脚本、资源目录、.NET、系统 Git/Python、更新选择和能力声明。保留 Windows 既有入口与行为，减少平台判断散落。

随后在 Mac 完成两种架构的首次 `cargo check` / build，修复由编译器确认的问题，再验证安装包启动、Chat、工作区、Git、Python、C# 服务和资源完整性。M1 只代表基础桌面壳与通用能力，不是完整 Unity 支持。

### M2：Unity Dev Agent 最小完整闭环

实现并验证双端 IPC、native broker、插件安装/升级、进程身份与生命周期。Mac 上至少覆盖：连接已打开项目、启动未打开项目、执行 C#、读取状态/日志、编辑资产、取消任务、断线重连、domain reload、退出后重启和多工作区隔离。

原生状态面不可用时要明确呈现已降级的能力；进程身份未知不能被当作可安全替换插件或终止进程的依据。完成 M2 后才适合称为可试用的 macOS Unity Dev Agent。

### M3：原生增强与公开分发

分别处理热更/ARM64 detour、Unity 内嵌窗口、跨窗拖动、后台 hook、原生状态探针、截图/调试和 Mac 图形工具。根据已验证的能力确定首发范围。

双架构安装包需完成嵌套 native 资源签名、必要权限、签名公证、全新用户安装、升级、卸载后数据策略，以及实际下载链接验证。保持现有会话 schema；若后续确实改动持久化结构，按仓库要求同时验证迁移与旧会话导出。

## 7. 后续验证计划

以下是适配后的验收入口，不表示当前代码已能在 Mac 执行成功。

Windows 共享逻辑和回归：

```powershell
bun run test
bun run typecheck
bun run typecheck:test
cargo check --manifest-path src-tauri/Cargo.toml
```

Unity 修改后继续使用仓库规定的 `bun run locus:test:unity -- --project <独立测试项目> --suite connect --install-plugin`，再按受影响模块增加 `native-bridge` / `state-probe` / `hot-reload` 等 suite。共享 Cargo 缓存和正式 Unity/应用进程不因本次适配清理或迁移。

Mac 的首次原生检查：

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo check --manifest-path src-tauri/Cargo.toml --target aarch64-apple-darwin
cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-apple-darwin
```

准备好目标资源与平台构建配置后，再分别使用 `bun tauri build --target aarch64-apple-darwin --bundles app,dmg` 与 `bun tauri build --target x86_64-apple-darwin --bundles app,dmg`。此处命令需要 M1 中的平台配置与脚本修复，不能直接作为当前版本的一键适配命令。Mac 原生开发需要 Apple 开发工具；Tauri 官方建议使用 Mac 执行应用包构建。[Tauri 前置要求](https://v2.tauri.app/start/prerequisites/)、[应用包构建](https://v2.tauri.app/distribute/macos-application-bundle/)

两套产物都需要实际运行。ARM Mac 上的 x64 运行可以覆盖 Rosetta 场景，但不能单独代表所有 Intel 硬件、系统版本和 Unity 组合。

## 8. 本次实际完成与验证

- 读取当前 main 的构建入口、Tauri/Cargo 配置、关键 Rust 平台分支、Unity broker/C# 接入、runtime/resource 解析、更新和快捷键逻辑；补充核对官方平台文档。
- 建立新的 `codex/macos-support` 分支及独立 worktree，仅新增本报告；未合并、读取或移植旧 macOS 分支代码。
- 本机工具：Bun `1.3.11`，Rust `1.95.0`，.NET SDK `10.0.303`。已安装 Rust targets 为 `x86_64-pc-windows-msvc`、`aarch64-unknown-linux-musl`；未配置或执行本次 Darwin 构建。
- 在原 worktree 的相同基线执行 `bun run test src/__tests__/keyboardShortcuts.test.ts src/__tests__/chatInputSettings.test.ts src/__tests__/appUpdate.test.ts`：**3 个测试文件、24 项测试通过**。这些结果验证当前共享逻辑基线，其中更新测试仍覆盖现有选择规则，不代表 Mac 更新已适配。
- 未启动或操作正式 Locus / Unity，未修改数据库、产品代码、依赖版本或旧分支；未提交或推送。

下一轮最适合先实施 M1 中 Windows 可验证的工程基础，同时尽早接入 Mac 构建反馈。Unity native broker 的实际移植应作为独立工作包，避免把“应用能打开”误当成“Unity 能正常工作”。
