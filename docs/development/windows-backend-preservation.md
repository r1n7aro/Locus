# Windows IPC 后端保护证据

基线提交：`cf70db9636ee61e74c779bec1472111ff389e00d`。

macOS 通信后端为独立新增模块。Windows 原始函数体、命名管道与 MMF 布局、FFI 消息语义、hook 和 overlay 模块不做公共重构。

## 可重复检查

仓库根目录运行：

```powershell
bun scripts/verify-windows-ipc-preservation.mjs
cargo test --manifest-path locus_native_plugin/Cargo.toml
cargo check --manifest-path locus_native_plugin/Cargo.toml --tests --target aarch64-apple-darwin
cargo check --manifest-path locus_native_plugin/Cargo.toml --tests --target x86_64-apple-darwin
```

保护脚本直接读取基线 Git 对象与当前文件，比较：

- native 的完整 `imp`、`hook`、`overlay` 模块、命名管道规范化和所有 Windows FFI 分支。
- desktop 的完整 `windows_impl`、请求 ACK 管理源文件和 Windows 调用分派。
- Windows MMF 完整读取模块和命名管道 endpoint 计算函数。
- C# `LocusBridge.Native.cs` 排除 `UNITY_EDITOR_OSX` 分支后的完整代码。

截至 2026-09-21，上述源代码比较均通过，Windows native 单测 8/8 通过。macOS native 新增 10 项行为/安全测试通过双 Darwin 目标编译检查，包括 ACK/完成、reload、中断与旧 generation、断线恢复、超时/重复 ID、hot reload 拒绝、真实 Unix socket 重连、独立状态读取和清理、错误路径、有界消息、陈旧 PID/权限/符号链接拒绝。

## 验证边界

源文件相等检查针对上述 Windows IPC 模块，不覆盖仓库其它改动。macOS 测试在 Windows 上仅完成编译检查，尚未执行；需要 macOS 执行 native 测试并验证 Unity 加载和生命周期。Windows native 单测不覆盖真实 Unity 注入、domain reload 或应用完整回归。

另外使用 `.tmp/macos-transport-check` 临时 harness 直接引用真实 transport、Mac 请求管理与 IPC 源文件，双 Darwin 目标 `cargo check --tests` 通过；其中仅 `AppHandle` 和事件发射入口为类型替身，此结果不等同于整个 Tauri 应用编译或链接。
