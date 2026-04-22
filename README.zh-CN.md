# Locus for Unity - Open Source Unity Dev Agent

[English](README.md) | 简体中文

Locus for Unity 是一个面向 Unity 项目的开源 AI Agent。它不仅能够像常规 Coding Agent 一样编写代码，还能够自主读入 Unity 编辑器上下文、修改场景与资产，并结合内置的自动记忆系统、可视化版本管理完成完整开发流程。

[文档](docs/index.mdx)

## 安装

目前仅支持 Windows 系统，我们很快会完善针对 macOS 的支持。

我们推荐使用 Releases 中的安装包安装，安装后的配置流程见 [快速开始](docs/overview/install-and-setup.mdx)。

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

## 许可证

主仓库源代码采用 `GPL-3.0-or-later` 发布，完整文本见 [LICENSE](LICENSE)。

## 文档构建工具链

`docs/` 保存文档源文件与本地文档构建工具链说明，目录约定见 [docs/BUILD_TOOLCHAIN.md](docs/BUILD_TOOLCHAIN.md)。

桌面应用安装包不包含 `docs/node_modules` 或 Mint 文档构建工具链。

## 第三方许可证

根级第三方说明见 [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES)。

`locus_unity/Editor/Roslyn` 中 Roslyn 与相关 .NET 依赖的许可证和分发说明见 [locus_unity/Editor/Roslyn/THIRD_PARTY.md](locus_unity/Editor/Roslyn/THIRD_PARTY.md)。

发布安装包时会同时携带根级许可证文件、根级第三方说明、生成的 `licenses/third_party/` bundle 与 `locus_unity/` 目录中的 Roslyn notices。
