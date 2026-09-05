# Locus Python SDK

`locus` 连接当前运行中的 Locus 桌面进程，复用本地登录态、可用模型、Agent 定义、工作区、会话存储，以及 Locus / Skill / MCP 工具链。SDK 只依赖 Python 标准库，由 Locus 自动加入所选 Python 运行时的 `PYTHONPATH`。

```python
import asyncio
import locus


async def main() -> None:
    workspace = await locus.get_workspace()
    models = await locus.list_models()
    tools = {tool.name: tool for tool in await locus.list_tools()}

    print(workspace.path)
    print([model.id for model in models])

    @locus.tool
    def project_policy() -> dict[str, object]:
        """Return project-specific review constraints."""
        return {"require_tests": True, "severity_order": True}

    reviewer = locus.Agent(
        name="Reviewer",
        id="reviewer",
        system_prompt="Review code and return evidence-backed findings.",
        tools=[name for name in ("read", "grep", "list") if name in tools]
        + [project_policy],
    )

    result = await reviewer.run(
        "Review the current project.",
        model=next((model.id for model in models if model.is_default), models[0].id),
    )
    result.raise_for_error()
    print(result.text or "")


asyncio.run(main())
```

## API 覆盖

任务接口直接内置在 SDK 中，无需发现或加载工具。`list_tasks()` 只列出当前会话的任务；
`get_task_status(id)` 返回状态、结果、日志路径和续跑次数。`wait_task(id, timeout=30)`
等待完成，超时返回当前状态且不取消任务；`cancel_task(id)` 请求取消。
`resume_task(id, message=...)` 在失败或取消的 subagent 原子会话中续跑，完成后自动通知父 Agent。
Bash/Python 不支持续跑。查询和等待不会消耗通知。

任务 ID 在会话内分配为 `t1`、`t2` 等短 ID。创建 subagent 时可指定 `name="reviewer"`，
此后返回和使用的 ID 即为 `reviewer`；同会话不能重名。`send_message("reviewer", text)`
给子 Agent 发消息，已结束的 subagent 会自动续跑原子会话。子 Agent 的注入信息包含自身 ID
和父 Agent 地址 `parent`；可用 `send_message("parent", text)` 回报，或
`send_message("parent/tester", text)` 联系同级 Agent。消息在下一次模型请求前注入。
仅执行任务控制的 Python 脚本使用 `readonly=true`。完整契约见 [tasks 帮助](../prompt/python-sdk/tasks.md)。

- 资源发现：`list_models()`、`list_agents()`、`list_tools()`、`get_workspace()`。
- Agent 编排：`Agent(...)`、`define_agent(...)`、`prompt(...)`；支持 Locus 工具与 `@locus.tool` Python 回调混合绑定。
- 工具执行：`call_tool(...)`、`ToolInfo.call(...)`；返回 `ToolCallResult`，可通过 `raise_for_error()` 转为异常。
- Run 生命周期：`status()`、`wait()`、`events()`、`event_stream()`、`cancel()`、`answer()`。
- 会话续接：`list_sessions()`、`get_session()`、`Session.prompt()`、`Session.events()`。
- Unity 生命周期：`get_unity_editor_status(project=...)`、`ensure_unity_editor(project=...)`、`restart_unity_editor(project=...)`；查询进程、连接与语义状态，按需拉起或重启当前项目对应的编辑器并等待就绪。
- Unity 阻塞恢复：`get_unity_dialog(project=...)`、`choose_unity_dialog(...)`、`wait_unity_execution(...)`；弹窗查询与选择由 Locus 原生窗口监听处理，不依赖 Unity 主线程。

`list_models()` 默认只返回当前登录态下可用的模型。`list_models(available_only=False)` 同时返回未登录的内置模型，并通过 `available` 与 `unavailable_reason` 标明状态。模型对象包含上下文窗口、推理强度与速度档位，可供 workflow 自动选择执行配置。

`list_tools()` 返回工具 schema、来源、工作区修改声明和 `agent_only` 标记。常规 Locus、Skill、Unity 与 MCP 工具可以直接调用；`subagent`、`ask_user_question`、`todowrite` 等依赖 Agent 运行状态的工具应绑定到 Agent 使用。

```python
listing = await locus.get_tool("list")
result = await listing.call(
    {"path": ".", "depth": 2, "include_files": True},
    timeout=30,
)
result.raise_for_error()
print(result.output)
```

直接调用属于会话外操作，工作区执行锁继续生效；会话撤销记录由 Agent 回合生成。需要进入 Locus 撤销链的写操作应交给 Agent 调用工具完成。

Unity 编辑器生命周期由 Locus 托管。`ensure_unity_editor()` 会复用已运行的当前项目编辑器，仅在进程状态明确为 `not_running` 时拉起 `ProjectVersion.txt` 对应的 Unity 或团结引擎版本，并等待指定目标：

```python
project = r"F:\Project"
status = await locus.get_unity_editor_status(project=project)
print(
    status.process_state,
    status.semantic_phase,
    status.ready,
    status.main_thread_blocked,
    status.blocking_dialog,
)

editor = await locus.ensure_unity_editor(
    project=project,
    mode="interactive",  # interactive | headless
    wait_until="ready",  # process | connected | ready
    timeout=300,
)
print(editor.launched, editor.status.process_id, editor.status.editor_path)

restarted = await locus.restart_unity_editor(
    project=project,
    mode="headless",
    wait_until="ready",
    timeout=300,
    force=False,  # 先请求正常关闭，超时后再强制结束残留进程
)
print(restarted.closed_process_ids, restarted.forced_process_ids)
```

同一 checkout 的 ensure 与 restart 调用会串行执行，避免重复启动。进程探测为 `unknown` 时 ensure 会返回错误并保留现场；`status.semantic_phase` 可区分 `starting`、`reloading`、`crashed`、`quit` 与 `unresponsive`。restart 结果中的 `forced_process_ids` 可判断关闭阶段是否使用了强制结束。无头编辑器会带有 `status.headless=True` 与 `status.launch_mode="headless"`，可在 Locus 的 Unity 状态面板中手动关闭。

从状态结果启动新 Agent 会话时，将 checkout scope 一并传入：

```python
result = await reviewer.run(
    "Review the connected Unity project.",
    workspace_ref=editor.status.workspace_ref,
    model="mock/tool",
)
```

Unity 模态弹窗阻塞主线程时，失败的 Unity 操作会返回 `dialog_id` 与可选的 `choice_id`。恢复接口按需调用，Agent 工具列表无需增加低频工具：

```python
dialog = await locus.get_unity_dialog(project=r"F:\Project")
if dialog is not None:
    print(dialog.title, dialog.message)
    for choice in dialog.choices:
        print(choice.id, choice.label)
    await locus.choose_unity_dialog(
        project=dialog.project,
        dialog_id=dialog.dialog_id,
        choice_id=dialog.choices[0].id,
    )
    # unity_execute 返回 request_state=detached 时，用错误里的 request_id
    # 获取原执行结果；该调用不会再次运行 snippet。
    output = await locus.wait_unity_execution(
        project=dialog.project,
        execution_id="exec-...",
    )
```

选择接口只接受当前快照返回的不透明 id，并在执行前重新验证 Unity PID、owner 窗口、弹窗指纹与按钮集合。按钮调用后，接口会等待原弹窗关闭或被新弹窗替换再返回，后续 Unity 调用不会与原弹窗关闭过程发生竞态。用户已经手动处理弹窗时，接口正常返回 `invoked=False`、`status="dialog_not_found"`；出现新弹窗时返回 `status="dialog_changed"`。未知 choice、重复并发选择和系统调用失败仍会返回错误。

## 自定义 workflow

自定义 workflow 是普通异步 Python 代码，可以组合串行步骤、并行分支、条件判断、直接工具调用和持久化会话。完整示例见 `examples/custom_workflow.py`。

```python
analysis, tests = await asyncio.gather(
    analyst.run("Inspect the implementation."),
    tester.run("Inspect test coverage."),
)
analysis.raise_for_error()
tests.raise_for_error()

final = await coordinator.run(
    f"Merge these reports:\n\n{analysis.text}\n\n{tests.text}"
)
final.raise_for_error()
```

同一个 `Agent` 对象会复用首次 prompt 创建的 `session_id`，保留模型端会话与 prompt cache。`new_session=True` 创建新会话；进程重启后可通过 `get_session(session_id)` 加载历史。文件型 Agent 可直接调用 `Session.prompt(...)` 续接；Python 内联 Agent 需要重新创建定义并通过 `Session.prompt(..., agent=agent)` 传入。

`Run.event_stream()` 按序产出持久化事件。遇到 `waiting_input` 状态时，可从 `RunStatus.runtime` 读取待回答问题，并调用 `run.answer(question_id, answer)`；无人值守 workflow 可以选择取消、超时或把问题转交给外部审批系统。

Python Agent 定义保存在脚本进程内，每次 prompt 随请求发送。Locus 会话、消息、模型续接状态与工具事件继续使用桌面端持久化存储。

桥接服务仅监听 `127.0.0.1`，每次 Locus 启动生成临时令牌。令牌由 Locus 注入启动的 Python 进程，无需写入脚本或配置文件。
