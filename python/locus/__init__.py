"""Python workflow SDK for the currently running Locus desktop instance.

The SDK reuses Locus's local login state, selected model, agent definitions,
Skill/MCP inventory, session persistence, and tool execution pipeline.
"""

from __future__ import annotations

from typing import Any

from ._client import (
    Client,
    LocusRpcError,
    LocusRunError,
    LocusSdkError,
    LocusToolError,
    LocusUnavailableError,
)
from ._models import (
    TaskStatus,
    TaskMessageDelivery,
    Agent,
    ModelInfo,
    Run,
    RunEvent,
    RunResult,
    RunStatus,
    Session,
    SessionMessage,
    SessionMessageDelivery,
    SessionSummary,
    ToolCallImage,
    ToolCallResult,
    ToolInfo,
    UnityEditorEnsureResult,
    UnityEditorLaunchInfo,
    UnityEditorRestartResult,
    UnityEditorStatus,
    UnityDialogChoice,
    UnityDialogChoiceResult,
    UnityModalDialog,
    WorkspaceInfo,
    WorkspaceRef,
)
from ._tools import Tool, tool

__all__ = [
    "TaskStatus",
    "TaskMessageDelivery",
    "wait_task",
    "send_message",
    "get_task_status",
    "cancel_task",
    "list_tasks",
    "resume_task",
    "Agent",
    "Client",
    "LocusRunError",
    "LocusRpcError",
    "LocusSdkError",
    "LocusToolError",
    "LocusUnavailableError",
    "ModelInfo",
    "Run",
    "RunEvent",
    "RunResult",
    "RunStatus",
    "Session",
    "SessionMessage",
    "SessionMessageDelivery",
    "SessionSummary",
    "ToolCallImage",
    "ToolCallResult",
    "ToolInfo",
    "Tool",
    "UnityEditorEnsureResult",
    "UnityEditorLaunchInfo",
    "UnityEditorRestartResult",
    "UnityEditorStatus",
    "UnityDialogChoice",
    "UnityDialogChoiceResult",
    "UnityModalDialog",
    "WorkspaceInfo",
    "WorkspaceRef",
    "call_tool",
    "define_agent",
    "get_agent",
    "get_model",
    "get_session",
    "get_tool",
    "get_unity_editor_status",
    "get_unity_dialog",
    "get_workspace",
    "list_agents",
    "list_models",
    "list_running_sessions",
    "list_sessions",
    "list_tools",
    "prompt",
    "choose_unity_dialog",
    "ensure_unity_editor",
    "restart_unity_editor",
    "send_session_message",
    "wait_unity_execution",
    "tool",
]

_default_client: Client | None = None


def _client() -> Client:
    global _default_client
    if _default_client is None:
        _default_client = Client()
    return _default_client


async def list_agents() -> list[Agent]:
    return await _client().list_agents()


async def list_models(*, available_only: bool = True) -> list[ModelInfo]:
    return await _client().list_models(available_only=available_only)


async def list_tools() -> list[ToolInfo]:
    return await _client().list_tools()


async def get_model(model_id: str, *, include_unavailable: bool = True) -> ModelInfo:
    return await _client().get_model(model_id, include_unavailable=include_unavailable)


async def get_tool(name: str) -> ToolInfo:
    return await _client().get_tool(name)


async def call_tool(
    tool: str | ToolInfo,
    arguments: dict[str, Any] | None = None,
    *,
    timeout: float | None = None,
    workspace_ref: WorkspaceRef | None = None,
) -> ToolCallResult:
    return await _client().call_tool(
        tool,
        arguments,
        timeout=timeout,
        workspace_ref=workspace_ref,
    )


async def get_workspace() -> WorkspaceInfo:
    return await _client().get_workspace()


async def get_task_status(task_id: str) -> TaskStatus:
    return await _client().get_task_status(task_id)


async def cancel_task(task_id: str) -> TaskStatus:
    return await _client().cancel_task(task_id)


async def list_tasks() -> list[TaskStatus]:
    return await _client().list_tasks()


async def wait_task(task_id: str, *, timeout: float = 30.0) -> TaskStatus:
    return await _client().wait_task(task_id, timeout=timeout)


async def send_message(task_id: str, message: str) -> TaskMessageDelivery:
    return await _client().send_message(task_id, message)


async def resume_task(task_id: str, *, message: str | None = None) -> TaskStatus:
    return await _client().resume_task(task_id, message=message)


async def get_unity_editor_status(*, project: str) -> UnityEditorStatus:
    return await _client().get_unity_editor_status(project=project)


async def ensure_unity_editor(
    *,
    project: str,
    mode: str = "interactive",
    wait_until: str = "ready",
    timeout: float = 300.0,
) -> UnityEditorEnsureResult:
    return await _client().ensure_unity_editor(
        project=project,
        mode=mode,
        wait_until=wait_until,
        timeout=timeout,
    )


async def restart_unity_editor(
    *,
    project: str,
    mode: str = "interactive",
    wait_until: str = "ready",
    timeout: float = 300.0,
    force: bool = False,
) -> UnityEditorRestartResult:
    return await _client().restart_unity_editor(
        project=project,
        mode=mode,
        wait_until=wait_until,
        timeout=timeout,
        force=force,
    )


async def get_unity_dialog(*, project: str) -> UnityModalDialog | None:
    return await _client().get_unity_dialog(project=project)


async def choose_unity_dialog(
    *,
    project: str,
    dialog_id: str,
    choice_id: str,
) -> UnityDialogChoiceResult:
    return await _client().choose_unity_dialog(
        project=project,
        dialog_id=dialog_id,
        choice_id=choice_id,
    )


async def wait_unity_execution(
    *,
    project: str,
    execution_id: str,
    timeout: float | None = None,
) -> str:
    return await _client().wait_unity_execution(
        project=project,
        execution_id=execution_id,
        timeout=timeout,
    )


async def list_sessions(
    *,
    archived: bool = False,
    running_only: bool = False,
    limit: int | None = None,
) -> list[SessionSummary]:
    return await _client().list_sessions(
        archived=archived,
        running_only=running_only,
        limit=limit,
    )


async def list_running_sessions(*, limit: int | None = None) -> list[SessionSummary]:
    """Return sessions that currently own an active Locus run."""
    return await _client().list_running_sessions(limit=limit)


async def get_session(session_id: str) -> Session:
    return await _client().get_session(session_id)


async def send_session_message(
    session_id: str,
    message: str,
    *,
    source_session_id: str | None = None,
) -> SessionMessageDelivery:
    """Insert a source-labelled user message into another active session."""
    return await _client().send_session_message(
        session_id,
        message,
        source_session_id=source_session_id,
    )


async def get_agent(agent_id: str) -> Agent:
    return await _client().get_agent(agent_id)


def define_agent(
    agent_id: str,
    *,
    system_prompt: str,
    tools: list[str | Tool] | tuple[str | Tool, ...] = (),
    name: str | None = None,
    description: str | None = None,
    sub_agents: list[str] | tuple[str, ...] = (),
    default_effort: str | None = None,
    model_recommendation: str | None = None,
) -> Agent:
    return _client().define_agent(
        agent_id,
        system_prompt=system_prompt,
        tools=tools,
        name=name,
        description=description,
        sub_agents=sub_agents,
        default_effort=default_effort,
        model_recommendation=model_recommendation,
    )


async def prompt(agent: Agent | str, text: str, **kwargs: Any) -> Run:
    if isinstance(agent, Agent):
        return await agent.prompt(text, **kwargs)
    return await _client().prompt_agent(agent, text, **kwargs)


__version__ = "0.2.0"
