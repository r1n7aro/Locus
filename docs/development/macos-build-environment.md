# Windows 本机的 macOS 交叉构建环境

本环境运行在本机已有的 WSL `Ubuntu`（Ubuntu 22.04），专用于 `codex/macos-support` 验证。Windows 的 Rust 默认工具链、正式 Locus profile 与 Unity 项目未改变。

## 已安装组件

| 组件 | 版本/位置 |
| --- | --- |
| Rust / Cargo | WSL Rust `1.95.0`，用 `cargo +1.95.0` 显式选择；原默认工具链保留 |
| Rust targets | `aarch64-apple-darwin`、`x86_64-apple-darwin` |
| C/Objective-C/C++ 编译器 | LLVM 官方 apt 仓库的 `clang-19` / `clang++-19`，与原 Clang 14 并存 |
| 链接器 | Rust 自带 LLD 22.1.2，通过 `/opt/locus-macos/bin/ld64.lld` 使用 |
| Apple SDK | Apple CLT 16.4 中的 `MacOSX14.5.sdk`，位于 `/opt/locus-macos/sdk/SDKs/MacOSX14.5.sdk` |
| SDK 下载工具 | xmac `15a873d4b961`，`/opt/locus-macos/bin/xmac` |
| 部署目标 | macOS 14.0，覆盖 arm64 / x64 |

SDK 通过 xmac 从 Apple 官方软件更新 CDN 下载，在本机提取；没有将 SDK 或工具链提交到仓库。xmac Linux 二进制 SHA256：`5b0af73a7d1d79d5944f15fafcae79d61c502f9e9912c877d88841845ea1c1a7`，与 GitHub release 元数据一致。[xmac 项目说明](https://github.com/Jarred-Sumner/xmac)

选定 macOS 14.0 是因为现有 C# sidecar 使用 .NET 10，其官方 macOS 支持范围从 14 开始；这仍不替代实际机器验收。[.NET 10 支持的系统](https://github.com/dotnet/core/blob/main/release-notes/10.0/supported-os.md)

初次使用原 Clang 14 编译 SDK 14.5 时，`TargetConditionals.h` 错误得到 `TARGET_OS_OSX=0`，导致 Foundation 类型缺失。安装并显式选择 Clang 19 后，该宏为 1，Objective-C 依赖能够继续编译。不要去改第三方源码或 Apple 头文件来掩盖工具链不匹配。

另外，本机非交互 WSL 控制台触发 Rust 1.95 的 styled diagnostic renderer 内部崩溃。验证脚本使用 `--message-format=json-diagnostic-short`，保留全部诊断而避开该渲染路径；没有关闭警告、跳过 native 编译或更换产品依赖。

## 使用本机环境

以下命令从 PowerShell 执行，显式选择可用的 `Ubuntu`。本机另一个 `Ubuntu-22.04` 发行版指向不存在的旧 VHDX，本任务未修复或删除它。

```powershell
# native broker 两架构真实链接
wsl -d Ubuntu -- bash /mnt/f/AGENT/locus-macos/scripts/check-macos-cross.sh native all build --release

# 校验生成的 Mach-O，并准备托管 DLL、sidecar、插件和许可证资源
bun run scripts/prepare-macos-resources.mjs --native-prebuilt

# 应用两个目标的检查与链接
wsl -d Ubuntu -- bash /mnt/f/AGENT/locus-macos/scripts/check-macos-cross.sh app all check
wsl -d Ubuntu -- bash /mnt/f/AGENT/locus-macos/scripts/check-macos-cross.sh app all build
```

首次资源准备需要当前源版本的 Json/Roslyn 托管产物；Windows 可以执行 `bun run macos:managed:export` 生成 `.tmp/macos-managed`。Mac 开发机可复制该导出目录并设置 `LOCUS_MACOS_MANAGED_BUNDLE`，导入时会检查源指纹、文件 SHA256 和 AnyCPU/IL-only 属性。

交叉编译脚本使用工作目录自己的默认 Cargo target 路径，不设置额外的 `CARGO_TARGET_DIR`；各 CPU 的产物放在 `target/<triple>/...`。不要同时清理其他任务正在使用的缓存或停止共享进程。

脚本支持通过 `LOCUS_MACOS_CROSS_ROOT`、`LOCUS_MACOS_RUST_TOOLCHAIN`、`SDKROOT`、`XMAC_CLANG`、`XMAC_CLANGXX`、`XMAC_LLD` 指定另一套已准备好的 Linux 工具链。生成的 xmac wrapper 本身绑定 SDK 位置，移动 SDK 时应重新生成 wrapper。

## 证据边界

`cargo check` 验证目标条件编译和类型；`cargo build` 才生成并链接 Mach-O。两者都不会在这台 Windows/WSL 机器上运行 macOS 应用。

`.app` / `.dmg` 打包、签名公证、WKWebView、Keychain、系统通知、Unity Editor 加载和真实 IPC 生命周期仍需 Mac 验收。正式 Mac 环境可使用 `bun tauri dev` / `bun tauri build --target <Darwin triple>` 走仓库新增的平台入口；不要把交叉编译成功写成这些实机项目已经通过。

最终已完成的目标编译、链接和产物哈希见 [交叉编译验收记录](./macos-cross-build-results-2026-09-21.md)。
