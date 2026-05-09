# Locus for Unity - Open Source Unity Dev Agent

> 规模化地提升游戏开发的效率，将创作者从繁琐的事务性工作中解放

[![文档](https://img.shields.io/badge/DOCS-unity.farlocus.com-f2c230?style=for-the-badge&labelColor=4a4a4a)](https://unity.farlocus.com/)
[![发布](https://img.shields.io/badge/RELEASE-GitHub-5d7285?style=for-the-badge&labelColor=4a4a4a)](https://github.com/r1n7aro/Locus/releases)
[![许可证](https://img.shields.io/badge/LICENSE-GPL--3.0--or--later-88b000?style=for-the-badge&labelColor=4a4a4a)](LICENSE)
[![路线图](https://img.shields.io/badge/ROADMAP-View-2d6cdf?style=for-the-badge&labelColor=4a4a4a)](https://unity.farlocus.com/overview/roadmap)
[![Bilibili](https://img.shields.io/badge/BILIBILI-Watch-00a1d6?style=for-the-badge&labelColor=4a4a4a)](https://www.bilibili.com/video/BV1H4ReBNELD/)
[![X](https://img.shields.io/badge/X-@farlocus-000000?style=for-the-badge&labelColor=4a4a4a)](https://x.com/farlocus)
![QQ群](https://img.shields.io/badge/QQ_Group-1104932978-12b7f5?style=for-the-badge&labelColor=4a4a4a)

[English](README.md) | 简体中文

[![在 Bilibili 观看演示](https://img.youtube.com/vi/xoApXZMon9M/maxresdefault.jpg)](https://www.bilibili.com/video/BV1H4ReBNELD/)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=r1n7aro/Locus&type=Date)](https://www.star-history.com/#r1n7aro/Locus&Date)

## 概览

`Locus for Unity`是一个面向Unity项目的**开源**AI Agent。

- **编辑器内操作**：编写C#代码、读入并修改Unity对象与资产，完成完整功能开发流程
- **运行时分析与调试**：自主操作并捕获运行时状态，协助你修复BUG、优化性能
- **自动化知识系统**：自动将对话需求总结成设计文档，并将项目理解保存在长期记忆中
- **可视化版本管理**：提供可视化的版本管理界面，支持Unity YAML资产的语义化差异分析与冲突处理
- **多种模型支持**：支持订阅帐号登录，并兼容多种LLM API能力

Locus 目前仍然处于早期测试状态（v0.2.8），欢迎您试用并通过Issue提出反馈，您的意见对我们非常重要！

## 从技术上讲，Locus有什么独特之处？

Locus是一个Rust + Tauri + Vue.js的独立进程应用程序。

- 我们设计了专有的中间表示，以让Agent渐进地读入大型场景与资产，并相应设计了检索工具，让agent能够快速定位目标对象
- 我们通过Roslyn库，实现了在Unity编辑器内JIT编译并执行C#代码，以此实现对资产的语义化修改；并在agent侧的版本管理做了特定处理，能够review/revert agent在对话中的资产/代码修改
- 我们基于Rust优秀并行生态系统，实现了高度并行化的资产数据库扫描，以此实现了对大型场景的高速语义解析与任意资产的引用关系查询（Unity Editor API仅提供依赖关系查询）
- 我们实现了自动化的知识系统，agent会把每次接到的零散对话需求总结成设计文档，并把工作中的理解保存到memory中，无需重复大量explore项目
- 知识系统内的文档支持配置AI维护模式、维护规则，并且支持调整在上下文内部的L0/L1/L2的注入方式，用户可以高度定制化渐进式展开的方式，并且原生支持大量文档的词法/语法检索，支持选择并下载嵌入运行时
- 我们通过编写C#状态机工具，Agent得以在运行时对某些特定帧数/事件上通过反射采样内部状态，并输出成逐帧表格，进行多帧行为的动态调试
- 我们提供图形化的版本管理界面，并且支持对Unity YAML文件语义化的修改查看与冲突解决
- 我们基于Vue.js实现了用户体验更好的现代前端界面，而非基于Unity Editor API的有限控件，并且通过Windows API将其嵌入到Unity窗口中

如果选择在 Unity 编辑器内部实现 Locus，或将 Locus 设计为一个 MCP 服务器，上述多数特性将难以落地，甚至在技术上几乎不可实现。

## 安装

目前仅支持 Windows 系统，我们很快会完善针对 macOS 的支持。

我们推荐使用 Releases 中的安装包安装，安装后的配置流程见 [快速开始](https://unity.farlocus.com/overview/install-and-setup)。

## 兼容性

Locus 当前支持 Windows 系统上的 Unity 2021 或更高版本。

如果您在更低 Unity 版本中发现兼容性问题，欢迎通过 Issue 反馈。我们会尽可能修复；涉及较大修改的兼容性修复，可能会作为分支方案处理。

## 从源代码构建

当前仓库使用 `bun` + `Tauri 2`，目前以 Windows 作为主要开发与构建平台。

### 开发时运行

```powershell
bun tauri dev
```

该命令会启动 Vite 开发服务器，并打开 Tauri 桌面应用。

### 构建

```powershell
bun tauri build
```

该命令会重新生成合并后的 Roslyn DLL、完成前端构建、生成第三方许可证 bundle，并打包桌面应用。当前默认输出 Windows `NSIS` 安装包，产物位于 `src-tauri/target/release/bundle/nsis/`。

## 发布版本

发布安装包与版本说明见 [GitHub Releases](https://github.com/r1n7aro/Locus/releases)。

本地构建两个 Windows 发布安装包：

```powershell
bun run release:installers
```

默认安装包保持标准命名，例如 `locus_0.2.5_x64-setup.exe`。无内嵌版本使用 `locus_0.2.5_x64-without_embed_python_git-setup.exe`。

## 许可证

主仓库源代码采用 `GPL-3.0-or-later` 发布，完整文本见 [LICENSE](LICENSE)。

## 文档构建工具链

`docs/` 保存文档源文件与本地文档构建工具链说明，目录约定见 [docs/BUILD_TOOLCHAIN.md](docs/BUILD_TOOLCHAIN.md)。

桌面应用安装包不包含 `docs/node_modules` 或 Mint 文档构建工具链。

## 第三方许可证

根级第三方说明见 [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES)。

`locus_unity/Editor/Roslyn` 中 Roslyn 与相关 .NET 依赖的许可证和分发说明见 [locus_unity/Editor/Roslyn/THIRD_PARTY.md](locus_unity/Editor/Roslyn/THIRD_PARTY.md)。

发布安装包时会同时携带根级许可证文件、根级第三方说明、生成的 `licenses/third_party/` bundle 与 `locus_unity/` 目录中的 Roslyn notices。

## 免责声明

本项目是一个面向 Unity Editor 的免费开源工具，与 Unity Technologies 无关联。
