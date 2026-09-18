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
    UnityEditorCloseResult,
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
from ._agent_rules import AgentRule
from ._assets import AssetBackend, AssetResult, Assets, asset_integer, asset_unsigned_integer
from ._csv import Csv, CsvColumnView, CsvViewSnapshot
from ._merges import MergeAssets, MergeJob, MergePlan, MergeResult, Merges
from ._worktrees import Worktree, Worktrees, PoolAcquisition
from ._sessions import SessionMessagePage, SessionSearchMatch, SessionSearchPage

__all__ = [
    "csv", "Csv", "CsvColumnView", "CsvViewSnapshot",
    "assets", "AssetBackend", "AssetResult", "Assets", "asset_integer", "asset_unsigned_integer",
    "worktrees", "Worktree", "Worktrees", "PoolAcquisition",
    "close_unity_editor", "UnityEditorCloseResult",
    "merges",
    "MergeJob",
    "MergeAssets",
    "MergePlan",
    "MergeResult",
    "Merges",
    "TaskStatus",
    "TaskMessageDelivery",
    "wait_task",
    "send_message",
    "get_task_status",
    "cancel_task",
    "list_tasks",
    "resume_task",
    "Agent",
    "AgentRule",
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
    "SessionMessagePage",
    "SessionSearchMatch",
    "SessionSearchPage",
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
    "search_sessions",
    "read_session",
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


class _DefaultAssets:
    def __getattr__(self, name: str) -> Any:
        return getattr(_client().assets, name)


assets = _DefaultAssets()


class _DefaultCsv:
    def __getattr__(self, name: str) -> Any:
        return getattr(_client().csv, name)


csv = _DefaultCsv()


class _DefaultMerges:
    async def prepare(self, **kwargs: Any) -> MergeJob:
        return await _client().merges.prepare(**kwargs)

    async def get(self, job_id: str, **kwargs: Any) -> MergeJob:
        return await _client().merges.get(job_id, **kwargs)


merges = _DefaultMerges()


class _DefaultWorktrees:
    def __getattr__(self, name: str) -> Any:
        return getattr(_client().worktrees, name)


worktrees = _DefaultWorktrees()


async def list_agents() -> list[Agent]:
    return await _client().list_agents()


async def list_models(*, available_only: bool = True) -> list[ModelInfo]:
    return await _client().list_models(available_only=available_only)


async def list_tools(*, workspace_ref: Any = None, worktree: Any = None) -> list[ToolInfo]:
    return await _client().list_tools(workspace_ref=workspace_ref, worktree=worktree)


async def get_model(model_id: str, *, include_unavailable: bool = True) -> ModelInfo:
    return await _client().get_model(model_id, include_unavailable=include_unavailable)


async def get_tool(name: str, *, workspace_ref: Any = None, worktree: Any = None) -> ToolInfo:
    return await _client().get_tool(name, workspace_ref=workspace_ref, worktree=worktree)


async def call_tool(
    tool: str | ToolInfo,
    arguments: dict[str, Any] | None = None,
    *,
    timeout: float | None = None,
    workspace_ref: WorkspaceRef | None = None,
    worktree: Any = None,
) -> ToolCallResult:
    return await _client().call_tool(
        tool,
        arguments,
        timeout=timeout,
        workspace_ref=workspace_ref,
        worktree=worktree,
    )


async def get_workspace(*, workspace_ref: Any = None, worktree: Any = None) -> WorkspaceInfo:
    return await _client().get_workspace(workspace_ref=workspace_ref, worktree=worktree)


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


async def get_unity_editor_status(*, project: str | None = None, worktree: Any = None,
    workspace_ref: Any = None) -> UnityEditorStatus:
    return await _client().get_unity_editor_status(project=project, worktree=worktree, workspace_ref=workspace_ref)


async def ensure_unity_editor(
    *,
    project: str | None = None,
    worktree: Any = None,
    workspace_ref: Any = None,
    mode: str = "interactive",
    wait_until: str = "ready",
    timeout: float = 300.0,
) -> UnityEditorEnsureResult:
    return await _client().ensure_unity_editor(
        project=project,
        worktree=worktree,
        workspace_ref=workspace_ref,
        mode=mode,
        wait_until=wait_until,
        timeout=timeout,
    )


async def restart_unity_editor(
    *,
    project: str | None = None,
    worktree: Any = None,
    workspace_ref: Any = None,
    mode: str = "interactive",
    wait_until: str = "ready",
    timeout: float = 300.0,
    force: bool = False,
) -> UnityEditorRestartResult:
    return await _client().restart_unity_editor(
        project=project,
        worktree=worktree,
        workspace_ref=workspace_ref,
        mode=mode,
        wait_until=wait_until,
        timeout=timeout,
        force=force,
    )


async def close_unity_editor(*, project: str | None = None, worktree: Any = None,
    workspace_ref: Any = None, timeout: float = 60.0, force: bool = False) -> UnityEditorCloseResult:
    return await _client().close_unity_editor(project=project, worktree=worktree,
        workspace_ref=workspace_ref, timeout=timeout, force=force)


async def get_unity_dialog(*, project: str | None = None, worktree: Any = None,
    workspace_ref: Any = None) -> UnityModalDialog | None:
    return await _client().get_unity_dialog(project=project, worktree=worktree, workspace_ref=workspace_ref)


async def choose_unity_dialog(
    *,
    project: str | None = None,
    worktree: Any = None,
    workspace_ref: Any = None,
    dialog_id: str,
    choice_id: str,
) -> UnityDialogChoiceResult:
    return await _client().choose_unity_dialog(
        project=project,
        worktree=worktree,
        workspace_ref=workspace_ref,
        dialog_id=dialog_id,
        choice_id=choice_id,
    )


async def wait_unity_execution(
    *,
    project: str | None = None,
    worktree: Any = None,
    workspace_ref: Any = None,
    execution_id: str,
    timeout: float | None = None,
) -> str:
    return await _client().wait_unity_execution(
        project=project,
        worktree=worktree,
        workspace_ref=workspace_ref,
        execution_id=execution_id,
        timeout=timeout,
    )


async def list_sessions(
    *,
    archived: bool = False,
    running_only: bool = False,
    limit: int | None = None,
    worktree: Any = None,
    workspace_ref: Any = None,
) -> list[SessionSummary]:
    return await _client().list_sessions(
        archived=archived,
        running_only=running_only,
        limit=limit,
        worktree=worktree,
        workspace_ref=workspace_ref,
    )


async def list_running_sessions(
    *, limit: int | None = None, worktree: Any = None, workspace_ref: Any = None,
) -> list[SessionSummary]:
    """Return sessions that currently own an active Locus run."""
    return await _client().list_running_sessions(
        limit=limit, worktree=worktree, workspace_ref=workspace_ref,
    )


async def search_sessions(
    query: str,
    *,
    archived: bool = False,
    session_id: str | None = None,
    limit: int = 20,
    cursor: str | None = None,
    worktree: Any = None,
    workspace_ref: Any = None,
) -> SessionSearchPage:
    return await _client().search_sessions(
        query, archived=archived, session_id=session_id, limit=limit, cursor=cursor,
        worktree=worktree, workspace_ref=workspace_ref,
    )


async def read_session(
    session_id: str,
    *,
    before_row_id: int | None = None,
    limit: int = 50,
    worktree: Any = None,
    workspace_ref: Any = None,
) -> SessionMessagePage:
    return await _client().read_session(
        session_id, before_row_id=before_row_id, limit=limit,
        worktree=worktree, workspace_ref=workspace_ref,
    )


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
