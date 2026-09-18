# 无头 Unity Editor 生命周期

受管 worktree 的 Unity 工具按需获取 Editor。已有交互式 Editor 时直接连接使用；没有运行实例时，Locus 准备插件并启动 `-batchmode -nographics` Editor。Agent 不需要在每个回合重复启动或关闭它。

- 交互式 Editor 由用户管理。自动管理不关闭、不重启，也不将它转换成无头模式。显式 `ensure_unity_editor(mode="headless")` 遇到已有交互式实例时同样复用，并返回其实际模式。
- Locus 记录自己启动的无头进程及创建时间、checkout materialization epoch。不会仅凭相同路径或 PID 接管别的进程。
- Editor 跨工具调用和 Agent 回合复用。空闲时间沿用 `workspace_service_resource_limits.serviceIdleTimeoutSecs`，默认 3600 秒；设置 → 通用 → 工作区资源使用分钟配置。
- 活动任务和工具执行持有租约，阻止回收。仅打开 Locus worktree 面板不阻止无头 Editor 到期释放。
- 回收通过 Unity 主线程检查未保存场景、Prefab、资产及编译、导入、Play Mode 状态，再请求正常退出。自动回收不调用强制结束；阻塞原因显示在运行实例列表。
- TTL 释放后，下一次 Unity 工具调用重新启动并连接该 checkout。已有前台 Editor 仍然优先。
- 设置页显示所有可观测的运行 Editor、模式、PID 和工作集内存。内存包含主 Editor 与同项目 Unity 资产导入进程，不含显存；无法获取时显示 `—`。当前进程观测实现支持 Windows。

测试工具仍要求目标 checkout 启用 Unity Test 工具并安装受支持的 `com.unity.test-framework`。新测试或热重载后的测试修改必须先通过 `unity_recompile` 完成编译收敛。

## CLI 验收

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --example unity_test_driver
bun run locus:test:unity -- --driver-binary .\src-tauri\target\debug\examples\unity_test_driver.exe --project C:\Projects\UnitySample --suite headless-development --connect-timeout-ms 900000 --timeout-ms 2400000 --output-dir artifacts/headless-development/validation
```

`--project` 只提供 Unity 版本参考。suite 在 `E:/LocusTemp/headless-loop-<id>` 创建最小源仓库和受管 worktree，预置 Unity Test Framework 1.7.0 及测试开关。可用 `LOCUS_HEADLESS_TEST_ROOT` 指定父目录。原项目源码及交互式 Editor 不参与写入或关闭。

每个阶段通过独立 Python SDK 进程执行，模拟多个 Agent 回合。脚本不调用 `ensure_unity_editor`：创建 worktree → 首次 Unity 工具自动启动 → 添加 C# 测试 → 重编译 → 捕获预期失败 → 修改代码 → 重编译并通过 EditMode → 通过 PlayMode → 验证活动测试租约和未保存场景保护 → 保存并等待 TTL → 再次工具调用自动启动 → 等待最终释放。报告同时记录每阶段 PID、内存和原交互式进程存活验证。

以 `LOCUS_DRIVER_JSON` 的 `suite_result`、`finished.ok` 和输出目录中的 `driver.log` 为准；`fixtureRoot/acceptance.json` 保留详细结果。测试文件和 Unity 日志保留供复核。

## 2026-09-09 验证结果

Unity `6000.5.8f1` 的两轮真实验收均为 `finished.ok=true`。最终轮次完成 8 个阶段：首次 PID `54776` 跨开发、修复和测试阶段保持不变；TTL 释放后自动启动 PID `62800`，最终也正常释放。预期失败用例为 `failed=1`，修复后 EditMode 和 PlayMode 均为 `passed=1, failed=0`。

活动测试、未保存场景保护、可见 Locus 面板不阻止空闲释放、内存可观测均通过。既有交互式 Editor PID `56836` 保持存活。测试使用 2 秒 TTL 加速验证；设置页验证默认 60 分钟，修改为 15 分钟后持久化为 900 秒，随后恢复默认。

- [最终验收 JSON](../../artifacts/headless-development/acceptance.json)
- [最终 CLI 日志](../../artifacts/headless-development/attempt2/driver.log)
- [设置页截图](../../artifacts/headless-development/ui/settings.png)

相关 Vitest 92 项、Rust 生命周期与原生内存单测 4 项、应用与测试 TypeScript 类型检查通过。设置页复用现有资源配置布局、worktree 表格排版、BaseButton 与全局颜色/字体 token，实例模式使用普通文本。
