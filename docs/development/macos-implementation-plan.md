# macOS 并列后端实施计划

基线：`cf70db9636ee61e74c779bec1472111ff389e00d`。分支：`codex/macos-support`；工作目录：`F:\AGENT\locus-macos`。

## 已确认边界

- 同时支持 Apple Silicon 与 Intel。
- 保留 Windows 后端内部实现、协议、hook、热更及默认行为；只在必要的公共入口增加条件编译分派。
- 新增并列 macOS 构建/资源/运行时/更新入口，以及 Unity 基础通信、native broker、独立状态通道和进程管理。
- macOS 暂不开放后台 hook、热更、Windows 原生状态探针、Unity 嵌入控制及相关 Win32 增强；明确暴露支持能力，不伪造成功。
- 不参考旧 macOS 分支，不操作正式 Locus profile 或现有 Unity 项目，不提交/推送。

## 实施步骤

1. [已完成] 建立计划与 Windows 保护基线；安装 Darwin 交叉编译工具链。
2. [已完成本机实施与验证] 新增 macOS 构建配置、目标资源准备、.NET/程序发现、资源路径和更新选择。
3. [已完成本机实施与验证] 新增独立 Unix IPC 客户端和 native broker 后端，保持既有 C ABI/消息语义，提供独立状态通道与 reload 生命周期。
4. [已完成本机实施与验证] 新增 macOS Editor 进程身份、发现/关闭逻辑、插件安装规则和平台能力门控；UID 枚举、空结果错误处理与 PID 复用问题经修复和复审通过。
5. [已完成静态/单元验证] Windows IPC 保留检查、平台源码/配置保留检查、前端类型检查与全套 Vitest 通过；真实 Unity 回归仍未执行。
6. [已完成] 应用 ARM64/Intel 两个 Darwin 目标 `cargo check` 和 `cargo build` 均通过，生成两份 Mach-O 可执行文件；native broker 双架构 release 已真实链接，Windows 应用 `cargo check` 通过。

## 验收要求

- Windows backend 的函数体、patch 常量、消息/共享内存协议及热更依赖保持原样；新增必要分派需审查。
- Mac IPC 覆盖请求/ACK、并发、超时/取消、重连、generation、重载中断及独立状态读取；仅限当前用户本机访问。
- Mac 进程管理保留 PID+进程身份验证，不能把 unknown 作为安全关闭/替换依据。
- 构建目标与宿主架构分开，资源不覆盖 Windows 产物；Unity 插件架构匹配 Editor。
- 交叉编译记录确切命令、目标、结果和环境阻塞；Windows 编译与单测不代表 Mac 编译通过，目标检查也不代表实机验证。
- 实机运行、Unity 加载与签名/公证如无法在当前环境验证，明确保留为待验收事项。

## 执行记录

- 已建立独立 worktree；已有扫描/风险报告保留为基线。
- 根据用户授权复用可用的 WSL `Ubuntu`，安装 Rust 1.95.0 与两个 Darwin targets、独立 LLVM 19，并通过固定 SHA256 的 xmac 从 Apple CDN 提取 MacOSX14.5 SDK；环境位于 WSL `/opt/locus-macos`。Windows 编译默认配置不变。
- macOS 最低版本按 .NET 10 官方支持范围取 14.0；具体实机兼容性仍待验证。
- 原生 broker 的 arm64/x86_64 Mach-O 动态库已真实链接；安全审查修改后已重新生成最终资源，并核对 plugin staging 与最终 release 产物 SHA256 一致。
- Windows IPC 保留审查脚本 12 项通过；Windows native 单测 8 项通过。macOS runtime 官方两架构 tar 包已在 Windows/WSL 提取验证，WSL 的 4 项测试包含执行位检查。
- Mac 条件编译的完整 Unity 插件托管源码已用 Unity 6000.5.6f1 API 编译通过；不将此结果当成 Mac Editor 实机加载验证。
- 全套 Vitest：486 文件、3219 通过、1 项真实 Unity 集成测试跳过；应用与测试 TypeScript 类型检查通过。12 个源码断言测试的读取入口仅做 CRLF→LF 归一化，未弱化断言。
- 前端使用 macOS/arm64 目标环境变量完成生产构建，命令退出 0。
- Windows IPC 12 项及平台源码/配置 24 项保护检查通过；Windows Unity bundle 原流程成功，native Windows 8 项单测及进程身份 6 项回归测试通过。
- Mac Git/gh 发现补齐 Finder 常见路径；Mac native 文件与 metadata 已移出 Windows 整包源目录。Mac 不带 ORT，因此本地嵌入入口明确返回不支持，避免依赖内部 panic，远程服务路径保留。
- 完整应用的两个 Darwin `cargo check` 均产生 `build-finished: success=true` 且验证命令退出 0。Rust 1.95 的 WSL 诊断渲染崩溃通过 short JSON 诊断格式避开，未关闭警告或修改依赖来掩盖错误。
- 详细结果见 [交叉编译验收记录](./macos-cross-build-results-2026-09-21.md)、[前端及保护测试记录](./macos-test-results-2026-09-21.md) 和 [构建环境](./macos-build-environment.md)。本阶段未执行 Mac 实机/Unity 加载、正式安装包、签名公证或 Windows 真实 Unity 集成；这些是后续验收，不计作本机编译已验证项目。
