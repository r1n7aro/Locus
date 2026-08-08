# Locus Python SDK

`locus` 连接当前正在运行的 Locus 桌面进程，直接复用本地登录态、当前模型配置、Agent 定义、会话存储以及 Locus/Skill/MCP 工具链。SDK 只依赖 Python 标准库，并由 Locus 自动加入所选 Python 运行时的 `PYTHONPATH`。

```python
import asyncio
import locus


async def main():
    print([agent.id for agent in await locus.list_agents()])

    @locus.tool
    def review_policy() -> dict:
        """Return project-specific review constraints."""
        return {"require_tests": True, "severity_order": True}

    reviewer = locus.Agent(
        name="Reviewer",
        id="reviewer",
        system_prompt="Review code and return evidence-backed findings.",
        tools=["read", "grep", "list", review_policy],
    )

    first = await reviewer.run("Review the current project.")
    second = await reviewer.run("Continue with the test coverage.")
    assert first.session_id == second.session_id
    print(second.text)


asyncio.run(main())
```

主要 API：

- `await locus.list_agents()`：列出 Locus 文件与插件提供的 Agent。
- `await locus.list_tools()`：列出可绑定的内置、Skill 与 MCP 工具。
- `locus.Agent(...)` / `locus.define_agent(...)`：在 Python 内存中定义 Agent，可混合绑定 Locus 工具名与 `@locus.tool` Python 函数。
- `run = await agent.prompt(...)`：首次调用创建 Locus 会话，后续调用默认复用 `agent.session_id`，沿用 Locus 最近使用的模型与推理强度。
- `result = await run` 或 `await run.wait(timeout=...)`：等待 Agent 完成。
- `await run.status()`、`await run.events()`、`await run.cancel()`：查询、观察和取消运行。
- `await run.answer(question_id, answer)`：回答 Agent 暂停等待的提问或确认。

Python Agent 定义只保存在脚本对象中，每次 prompt 随请求发送，不写入 Locus Agent 注册表。Locus 会话、消息和模型端点的续接状态继续使用常规持久化存储。同一 Agent 对象保持稳定的 system prompt、工具 schema 与 session ID，从而保留 LLM 会话续接和 prompt cache；`new_session=True` 可显式开始新会话。

`@locus.tool` 从函数签名与类型标注生成 JSON Schema。模型调用时，Locus 通过受临时令牌保护的回环地址把参数传回原 Python 事件循环，支持同步与异步函数、超时和异常回传。

桥接服务仅监听 `127.0.0.1`，每次 Locus 启动生成新的临时令牌。令牌通过 Locus 启动的 Python 进程环境注入，SDK 代码无需读取或保存桌面端凭据。
