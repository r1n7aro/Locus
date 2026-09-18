# Locus Python SDK

`locus` 连接当前运行中的 Locus 桌面进程，复用本地登录态、可用模型、Agent 定义、工作区、会话存储，以及 Locus / Skill / MCP 工具链。核心 SDK 只依赖 Python 标准库，由 Locus 自动加入所选 Python 运行时的 `PYTHONPATH`。CSV 工作表入口使用真实的 openpyxl 3.1.5；打包的托管 Python 自带依赖，外部 Python 可运行 `python -m pip install -r python/requirements-csv.txt`。

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

`wb = await locus.csv.load_workbook(path)` 将 CSV 加载为真实的 openpyxl 工作簿，`ws = wb.active` 后直接使用 `ws["A1"]`、`Font`、`PatternFill`、`Border`、`Alignment`、`NamedStyle`、`number_format`、行高列宽、合并和条件格式，最后 `await wb.save()`。CSV 保持文本，样式写入配套 `.csv.view`；仅改样式不改 CSV 字节，值编辑只替换改变的字段。版本检查与批量持久化由 SDK 处理。旧 `read_view` / `patch_view` 保留，用于稀疏规则和显示筛选。能力范围及与 Excel 的差异见 [CSV 帮助](../prompt/python-sdk/csv.md)。

`locus.worktrees` 支持创建、发现、导入、删除 worktree，以及 Unity 项目池槽位的申请、释放和私有 Library 复用。返回的句柄可直接作为 `worktree=` 传入程序化工具调用、Unity 编辑器状态/启动/关闭/重启、弹窗处理和 merge API；单个 Agent 的工作目录保持不变。详见 [worktrees 帮助](../prompt/python-sdk/worktrees.md)。

`locus.merges` 提供选择性 Unity 合并：多个 commit 可直接集成到脏工作区，Agent 选择字段、对象或完整文件；应用、Unity 验证、暂存和提交分别执行。目标可以是已有 checkout 或继承脏状态的新分支/worktree。完整契约和示例见 [merges 帮助](../prompt/python-sdk/merges.md)。

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
- Agent 规则：通过 `await locus.get_agent("unity")` 获取内置 Agent，再调用 `list_rules()`、`read_rule(key)`、`save_rule(file_name, content)`、`set_rule_enabled(key, enabled)`；默认使用当前 Python 会话工作区，也支持 `workspace_ref=` 或 `worktree=`。新增规则与启停配置仅保存到该工作区，下一次模型请求生效，无需手动打开设置。详见 [Agent 帮助](../prompt/python-sdk/agents.md)。
- 工具执行：`call_tool(...)`、`ToolInfo.call(...)`；返回 `ToolCallResult`，可通过 `raise_for_error()` 转为异常。
- Run 生命周期：`status()`、`wait()`、`events()`、`event_stream()`、`cancel()`、`answer()`。
- 会话历史：`list_sessions(archived=...)` 按归档状态列出当前工作区会话，`search_sessions()` 搜索，`read_session()` 分页读取；`get_session()`、`Session.prompt()`、`Session.events()` 用于完整加载、续接和事件读取。
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

Unity 模态弹窗阻塞主线程时，失败的 Unity 操作会返回 `dialog_id` 与可选的 `choice_id`。`ensure_unity_editor` 和 `restart_unity_editor` 也会立即以 `LocusRpcError` 返回 `code=unity_modal_dialog_blocked`、标题、正文和全部选项，包括尚未连接时的场景备份恢复弹窗。`request_state=editor_starting` 表示已启动，选择后用相同参数的 `ensure_unity_editor` 继续等待，不要再次重启；`editor_closing` 表示正常关闭被弹窗打断，尚未启动替代进程，选择后先查询状态再决定是否继续原操作。恢复接口按需调用，Agent 工具列表无需增加低频工具：

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

查询历史时，默认使用 Python 进程注入的当前工作区（checkout），也可传 `workspace_ref=workspace_ref` 或 `worktree=handle`：

```python
sessions = await locus.list_sessions(archived=False, limit=20)
archived = await locus.list_sessions(archived=True, limit=20)

hits = await locus.search_sessions("编译失败", archived=True, limit=10)
for hit in hits.matches:
    print(hit.session_id, hit.session_title, hit.message_id, hit.field, hit.excerpt)

# session_id 可将搜索范围限定为单个会话；next_cursor 用于继续相同搜索。
hits = await locus.search_sessions("Shader", session_id="session-id", archived=False)
if hits.has_more:
    hits = await locus.search_sessions(
        "Shader", session_id="session-id", archived=False, cursor=hits.next_cursor,
    )

# 先读最近一页，再按需读取更早内容；已归档会话同样可读。
page = await locus.read_session("session-id", limit=30)
for message in page.messages:
    print(message.role, message.content)
if page.has_more_history:
    page = await locus.read_session(
        "session-id", before_row_id=page.oldest_message_row_id, limit=30,
    )
```

搜索覆盖标题、消息正文、思考内容和工具调用，使用字面子串匹配（ASCII 不区分大小写，中文直接匹配），返回命中摘要；不会扫描另存的大型工具输出文件或图片。搜索每页默认最多 20 条，`limit` 范围 1–100。**空结果不代表搜索结束**：`has_more=True` 时继续传入 `cursor=hits.next_cursor`，只有 `next_cursor=None` 才表示扫描完成。续查须保持查询词、归档状态和会话/工作区范围一致，`scanned_messages`、`scanned_bytes` 可查看本次扫描量。

大会话搜索使用独立只读连接和后台线程，按现有会话消息索引分批读取。每次最多检查 512 条消息，以 8 MiB 文本 / 100 ms 为扫描预算，在字段间检查并返回；单个超长字段或磁盘等待可能超过目标。会话按最近活动排序，先查标题，再从新到旧查消息；游标定位到消息字段，后续页不会重扫已查文本，也不对全部命中排序。首次搜索后新追加的消息不进入本轮查询；重新搜索可包含最新内容。完整无命中查询仍需线性扫描，分批返回用于控制单次开销。

读取每页默认 50 条，`limit` 范围 1–1000；为保留完整工具调用组，实际数量可能超过目标值。页内按时间顺序展示，游标向更早历史移动，新消息追加不会导致已读历史重复。以 `has_more_history` 判断是否继续，工具调用等完整字段可从 `message.raw` 读取。`get_session()` 和 `SessionSummary.load()` 保留原有完整加载行为。

搜索和分页读取会校验会话属于所选工作区。没有注入工作区时须显式传入选择器；列表接口则保留旧版的“未绑定工作区会话”行为。

`Run.event_stream()` 按序产出持久化事件。遇到 `waiting_input` 状态时，可从 `RunStatus.runtime` 读取待回答问题，并调用 `run.answer(question_id, answer)`；无人值守 workflow 可以选择取消、超时或把问题转交给外部审批系统。

Python Agent 定义保存在脚本进程内，每次 prompt 随请求发送。Locus 会话、消息、模型续接状态与工具事件继续使用桌面端持久化存储。

桥接服务仅监听 `127.0.0.1`，每次 Locus 启动生成临时令牌。令牌由 Locus 注入启动的 Python 进程，无需写入脚本或配置文件。
