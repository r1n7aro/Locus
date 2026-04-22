# Docs Build Toolchain

`docs/` contains the Locus documentation source files and the local documentation build toolchain.

- The documentation source files are distributed as part of the main repository under `GPL-3.0-or-later`.
- `docs/package.json` and `docs/bun.lock` are used only for local preview, validation, and documentation site builds.
- The desktop application bundle does not include the `mint` CLI, `docs/node_modules`, or documentation build toolchain dependencies.
- Documentation build dependencies continue to follow their own respective licenses. The dependency set is defined by `docs/package.json` and `docs/bun.lock`.
