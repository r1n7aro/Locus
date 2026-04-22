# Locus for Unity - Open Source Unity Dev Agent

English | [简体中文](README.zh-CN.md)

Locus for Unity is an open source AI agent for Unity projects. It can write code like a standard coding agent, read Unity Editor context, modify scenes and assets, and combine built-in automatic memory with visual version control to support the full development workflow.

[Documentation](docs/en/index.mdx)

## Installation

Windows is currently the only supported platform. We plan to add macOS support soon.

We recommend installing from the Releases build. For the post-installation setup flow, see [Quick Start](docs/en/overview/install-and-setup.mdx).

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

## License

The main repository source code is released under `GPL-3.0-or-later`. See [LICENSE](LICENSE) for the full text.

## Documentation Build Toolchain

`docs/` contains the documentation source files and the local documentation build toolchain notes. See [docs/BUILD_TOOLCHAIN.md](docs/BUILD_TOOLCHAIN.md).

The desktop app installer does not include `docs/node_modules` or the Mint documentation build toolchain.

## Third-Party Licenses

See [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES) for root-level third-party notices.

For Roslyn and related .NET dependency license and distribution notes inside `locus_unity/Editor/Roslyn`, see [locus_unity/Editor/Roslyn/THIRD_PARTY.md](locus_unity/Editor/Roslyn/THIRD_PARTY.md).

Published installers include the root license file, the root third-party notices, the generated `licenses/third_party/` bundle, and the Roslyn notices under `locus_unity/`.
