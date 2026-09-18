# Python SDK 多 worktree 管理与双 Editor 合并分析

本轮将已有 worktree / Unity 项目池能力接入 Python SDK。单个 Agent 保持原工作目录，通过每次调用的 checkout 句柄控制多个 Editor；不切换会话绑定，也不修改全局工作区选择。

## 接口

| 需求 | 接口 |
| --- | --- |
| 创建、查询、导入 | `locus.worktrees.create/list/get/discover/import_worktree` |
| 删除与操作记录 | `locus.worktrees.remove/operations` |
| 申请、归还可复用项目槽位 | `locus.worktrees.acquire/release` |
| 指定 checkout 调用工具 | `locus.call_tool(..., worktree=wt)`、`ToolInfo.call(..., worktree=wt)` |
| 指定 checkout 发现工具 | `list_tools/get_tool(..., worktree=wt)`；返回的工具对象保留该绑定 |
| 编辑器生命周期 | `get_unity_editor_status/ensure_unity_editor/close_unity_editor/restart_unity_editor(worktree=wt)` |
| 弹窗与异步执行结果 | `get_unity_dialog/choose_unity_dialog/wait_unity_execution(worktree=wt)` |
| 自研合并 | `locus.merges.prepare/get(..., worktree=wt)`；后续操作继续绑定 `job.workspace_ref` |

`Worktree` 包含根路径、分支、HEAD、checkout/project ID、池槽归属及 `workspace_ref`。后者携带 runtime generation 与 materialization epoch。创建/导入/申请使用已有后台命令完成注册；显式 `get(checkout_id)` 可在应用重启后重新注册活动 checkout。列举不会启动 Editor 或服务，包含空闲、异常和已移除记录。

“预算池”在这里按用户确认的含义处理为 Unity 项目池槽位。`max_slots` 限制指定池中的物理槽位数量，不修改全局 Editor/session 并发设置。申请返回 `PoolAcquisition`，含 `.worktree`、`.reused` 和 `.preserved_library`。同一物理目录换任务后 epoch 增加，旧句柄和旧 assignment 不能继续操作新任务。

旧 `workspace_ref=` 与生命周期 `project=` 入口保留。新流程优先使用有代次信息的句柄；不能同时传多个目标选择器。省略工具目标时使用 Python 工具注入的 checkout；不根据当前 UI 选择猜测目标。

## 与 merge API 的协作

推荐流程是先把所选来源 commit 申请到独立池槽，在来源与目标 Editor 中采集运行时证据，再用自研 API 的冻结快照作合并决策。目标由 `worktree=` 指定，prepare 若创建新 checkout，则后续 Unity 调用使用返回的 `job.workspace_ref`。

```python
import asyncio

target = await locus.worktrees.get(target_checkout_id)
source_slot = await locus.worktrees.acquire(
    pool_root="F:/UnityPool", commit=source_oid, max_slots=2,
)
source = source_slot.worktree
await asyncio.gather(*(
    locus.ensure_unity_editor(worktree=wt, timeout=600)
    for wt in (source, target)
))
observations = await asyncio.gather(*(
    locus.call_tool("unity_execute", {
        "readonly": True, "request_editor_status": "editing",
        "code": "print(UnityEngine.Application.dataPath);",
    }, worktree=wt)
    for wt in (source, target)
))
for result in observations:
    result.raise_for_error()

job = await locus.merges.prepare(worktree=target, sources=[{"commits": [source_oid]}])
catalog = await job.changes()
plan = await job.new_plan(default="keep_target")
# 根据冻结 catalog 显式选择字段、对象或文件，再 preview/apply。
# 应用后调用目标 Editor：
# await locus.call_tool("unity_execute", arguments, workspace_ref=job.workspace_ref)
# await plan.validate(level="unity")
```

这套接口能直接衔接现有 `inspect_asset → new_plan → preview → apply → validate` 流程。需要区分以下证据边界：

- Editor 反映当前已导入状态，`inspect_asset` 反映冻结的 target/source/base/result。运行时观察不能替代快照，也不会自动转成字段选择。
- 来源池槽应检出准确的 commit。非连续多 commit 的 delta 仍分别相对其 parent，一个最新分支 Editor 无法代表所有历史版本。
- prepare 后更改目标文件会触发现有 stale 检查。需要实际编辑的分析应在 prepare 前完成，或重新准备计划。
- 普通 Unity 验证检查应用后的目标；精确部分提交验证还会使用候选池槽。若两个分析 Editor 已占满容量，应先关闭来源 Editor，再运行候选验证。
- 未保存场景/Prefab stage、Play Mode、弹窗、编译与导入仍按现有门控处理。关闭 Editor 后，先保存或明确提交源码修改，才能释放/删除槽位。

## 并发与写锁

本轮不扩大普通工具的写锁范围。SDK 与 MCP 入口复用和 Agent 前台一致的判定：关闭会话文件撤回后，普通不透明 Python/bash 写任务不持有整工作区写锁；已知路径写入保留路径协调，Unity 写执行/编译屏障与 merge 事务维持已有规则。只读跨 Editor 分析不新增写锁。

当外层可写 Python 已持有真实写锁时，嵌套 SDK 工具与 merge 共用委托机制。委托检查项目、Git common directory、checkout、generation、epoch；同 checkout 可借用现有锁，不同 checkout 分别排队，避免将两个 Editor 串行化。交叉等待返回可重试 busy 错误。非 Git 工作区内的同 checkout 调用仍可委托。

普通 worktree 创建、导入和池申请只沿用已有 journal / 目录租约；包含本地修改且启用文件撤回的创建使用已有写入协调。关闭/启动/重启 Editor 按 checkout 共用生命周期互斥，scope 租约防止过程中回收该目录。关闭 API 在确认进程退出后清理已停止 Editor 的短期 marker，沿用已有 Windows 共享句柄重试规则。

## 验证

新增 `worktree-sdk` CLI suite 使用公开 Python SDK 和嵌套 Python 工具，创建独立 worktree 与池槽，运行两个真实 Editor，验证调用路由、写锁委托、merge 应用/恢复、进程关闭、Library 复用、旧句柄拒绝及删除。测试源码位于 `src-tauri/src/cli_driver/worktree_sdk_acceptance.rs` 和同名 `.py.txt`。

```powershell
bun run locus:test:unity -- --project <Unity6.5基线> --suite worktree-sdk --connect-timeout-ms 600000 --timeout-ms 1800000 --no-progress-timeout-ms 600000 --output-dir artifacts/worktree-sdk-validation
```

`LOCUS_WORKTREE_SDK_TEST_ROOT` 可指定新测试目录的父目录；默认 Windows 使用 `E:/LocusTemp`。测试在失败时保留目录与 `acceptance.json`，只关闭本次创建目录中的 Editor。源码/index/HEAD 的保留检查与最终 suite 结果分开记录。

2026-09-07 实测结果：

- Python SDK 38 项、Rust SDK/委托锁/marker/连接缓存 28 项、相关 Vitest 16 项全部通过。
- Unity `6000.5.8f1` 最小独立工程：开启会话文件撤回时 18/18；关闭时 18/18，均收到 `finished.ok=true`。关闭配置的日志中没有 `tools=[python]` 写锁记录。
- 两个 Editor 并行路由、嵌套可写 Python 同时调用当前/兄弟 Editor、冻结 merge 应用与恢复、关闭状态、Library 保留、epoch 递增、旧句柄拒绝及受管目录删除均通过。测试后原先两个 Editor 继续运行。
- 首轮暴露测试参数名错误；第二轮暴露关闭后仍回退到旧连接缓存。已修正参数并增加关闭缓存失效、过期探测结果拒绝回填的回归测试；失败日志保留。

通过证据：[开启撤回](../../artifacts/worktree-sdk-validation-r3/acceptance.json)、[关闭撤回](../../artifacts/worktree-sdk-undo-disabled/acceptance.json)、[验证汇总](../../artifacts/worktree-sdk-verification.json)。新 CLI 的 merge 部分验证普通文本路径的冻结 plan/apply/abort；完整复杂 YAML 资产验收仍以[原实施记录](worktree-implementation-and-validation.md)为准，本轮没有重跑该全量套件。
