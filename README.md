# Locus for Unity - Open Source Unity Dev Agent

[![Docs](https://img.shields.io/badge/docs-unity.farlocus.com-5d7285)](https://unity.farlocus.com/en)
[![Release](https://img.shields.io/github/v/release/r1n7aro/Locus?display_name=tag)](https://github.com/r1n7aro/Locus/releases)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-4b6375)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows-2d6cdf)](https://github.com/r1n7aro/Locus/releases)

English | [简体中文](README.zh-CN.md)

Scale game development efficiency and free creators from tedious, repetitive work.

## Overview

`Locus for Unity` is an open-source AI Agent for Unity projects.

- **In-editor operations**: write C# code, read and modify Unity objects and assets, and complete the full feature development workflow
- **Automated knowledge system**: automatically summarize conversation requirements into design documents and preserve project understanding in long-term memory
- **Visual version control**: provide a visual version control interface with semantic diff analysis and conflict handling for Unity YAML assets
- **Highly optimized prompts**: optimize prompts for Unity projects, improve Agent capability, and avoid common mistakes
- **Multiple model support**: support subscription account sign-in and compatibility with multiple LLM API capabilities

![Locus for Unity](docs/images/home.png)

## Quick Links

- [WIKI](https://unity.farlocus.com/en)
- [Quick Start](https://unity.farlocus.com/en/overview/install-and-setup): Install Locus quickly and complete the full setup in minutes
- [Roadmap](https://unity.farlocus.com/en/overview/roadmap): See the features we are implementing and planning

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
