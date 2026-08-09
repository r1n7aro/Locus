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

- 资源发现：`list_models()`、`list_agents()`、`list_tools()`、`get_workspace()`。
- Agent 编排：`Agent(...)`、`define_agent(...)`、`prompt(...)`；支持 Locus 工具与 `@locus.tool` Python 回调混合绑定。
- 工具执行：`call_tool(...)`、`ToolInfo.call(...)`；返回 `ToolCallResult`，可通过 `raise_for_error()` 转为异常。
- Run 生命周期：`status()`、`wait()`、`events()`、`event_stream()`、`cancel()`、`answer()`。
- 会话续接：`list_sessions()`、`get_session()`、`Session.prompt()`、`Session.events()`。

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
