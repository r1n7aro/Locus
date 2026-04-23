# Locus for Unity - Open Source Unity Dev Agent

> Scale game development efficiency and free creators from tedious, repetitive work.

[![Docs](https://img.shields.io/badge/DOCS-unity.farlocus.com-f2c230?style=for-the-badge&labelColor=4a4a4a)](https://unity.farlocus.com/en)
[![Release](https://img.shields.io/badge/RELEASE-GitHub-5d7285?style=for-the-badge&labelColor=4a4a4a)](https://github.com/r1n7aro/Locus/releases)
[![License](https://img.shields.io/badge/LICENSE-GPL--3.0--or--later-88b000?style=for-the-badge&labelColor=4a4a4a)](LICENSE)
[![Roadmap](https://img.shields.io/badge/ROADMAP-View-2d6cdf?style=for-the-badge&labelColor=4a4a4a)](https://unity.farlocus.com/en/overview/roadmap)

English | [简体中文](README.zh-CN.md)

## Overview

`Locus for Unity` is an open-source AI Agent for Unity projects.

- **In-editor operations**: write C# code, read and modify Unity objects and assets, and complete the full feature development workflow
- **Automated knowledge system**: automatically summarize conversation requirements into design documents and preserve project understanding in long-term memory
- **Visual version control**: provide a visual version control interface with semantic diff analysis and conflict handling for Unity YAML assets
- **Highly optimized prompts**: optimize prompts for Unity projects, improve Agent capability, and avoid common mistakes
- **Multiple model support**: support subscription account sign-in and compatibility with multiple LLM API capabilities

Locus is currently in early testing (`v0.1.1`). We welcome you to try it and share feedback through Issues. Your input is highly valuable to us.

## Installation

Windows is currently the only supported platform. We plan to add macOS support soon.

We recommend installing from the Releases build. For the post-installation setup flow, see [Quick Start](https://unity.farlocus.com/en/overview/install-and-setup).

## Build from Source

This repository uses `bun` + `Tauri 2`, with Windows as the primary development and build platform.

### Run in Development

```powershell
bun tauri dev
```

This command starts the Vite development server and opens the Tauri desktop app.

### Build

```powershell
bun tauri build
```

This command builds the frontend, generates the third-party license bundle, and packages the desktop app. The default output is a Windows `NSIS` installer under `src-tauri/target/release/bundle/nsis/`.

## Releases

See [GitHub Releases](https://github.com/r1n7aro/Locus/releases) for published installers and release notes.

## License

The main repository source code is released under `GPL-3.0-or-later`. See [LICENSE](LICENSE) for the full text.

## Documentation Build Toolchain

`docs/` contains the documentation source files and the local documentation build toolchain notes. See [docs/BUILD_TOOLCHAIN.md](docs/BUILD_TOOLCHAIN.md).

The desktop app installer does not include `docs/node_modules` or the Mint documentation build toolchain.

## Third-Party Licenses

See [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES) for root-level third-party notices.

For Roslyn and related .NET dependency license and distribution notes inside `locus_unity/Editor/Roslyn`, see [locus_unity/Editor/Roslyn/THIRD_PARTY.md](locus_unity/Editor/Roslyn/THIRD_PARTY.md).

Published installers include the root license file, the root third-party notices, the generated `licenses/third_party/` bundle, and the Roslyn notices under `locus_unity/`.

## Disclaimer

This project is a free and open-source tool for the Unity Editor, and is not affiliated with Unity Technologies.
