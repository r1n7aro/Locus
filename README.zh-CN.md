# Locus for Unity - Open Source Unity Dev Agent

> 规模化地提升游戏开发的效率，将创作者从繁琐的事务性工作中解放

[![文档](https://img.shields.io/badge/DOCS-unity.farlocus.com-f2c230?style=for-the-badge&labelColor=4a4a4a)](https://unity.farlocus.com/)
[![发布](https://img.shields.io/badge/RELEASE-GitHub-5d7285?style=for-the-badge&labelColor=4a4a4a)](https://github.com/r1n7aro/Locus/releases)
[![许可证](https://img.shields.io/badge/LICENSE-GPL--3.0--or--later-88b000?style=for-the-badge&labelColor=4a4a4a)](LICENSE)
[![路线图](https://img.shields.io/badge/ROADMAP-View-2d6cdf?style=for-the-badge&labelColor=4a4a4a)](https://unity.farlocus.com/overview/roadmap)

[English](README.md) | 简体中文

## 概览

`Locus for Unity`是一个面向Unity项目的**开源**AI Agent。

- **编辑器内操作**：编写C#代码、读入并修改Unity对象与资产，完成完整功能开发流程
- **自动化知识系统**：自动将对话需求总结成设计文档，并将项目理解保存在长期记忆中
- **可视化版本管理**：提供可视化的版本管理界面，支持Unity YAML资产的语义化差异分析与冲突处理
- **高度优化的提示词**：针对于Unity项目优化提示词，提高Agent能力，规避常见错误
- **多种模型支持**：支持订阅帐号登录，并兼容多种LLM API能力

Locus 目前仍然处于早期测试状态（v0.1.1），欢迎您试用并通过Issue提出反馈，您的意见对我们非常重要！

## 安装

目前仅支持 Windows 系统，我们很快会完善针对 macOS 的支持。

我们推荐使用 Releases 中的安装包安装，安装后的配置流程见 [快速开始](https://unity.farlocus.com/overview/install-and-setup)。

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

该命令会完成前端构建、生成第三方许可证 bundle，并打包桌面应用。当前默认输出 Windows `NSIS` 安装包，产物位于 `src-tauri/target/release/bundle/nsis/`。

## 发布版本

发布安装包与版本说明见 [GitHub Releases](https://github.com/r1n7aro/Locus/releases)。

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
