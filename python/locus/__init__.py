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
    Agent,
    ModelInfo,
    Run,
    RunEvent,
    RunResult,
    RunStatus,
    Session,
    SessionMessage,
    SessionSummary,
    ToolCallImage,
    ToolCallResult,
    ToolInfo,
    WorkspaceInfo,
)
from ._tools import Tool, tool

__all__ = [
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
    "SessionSummary",
    "ToolCallImage",
    "ToolCallResult",
    "ToolInfo",
    "Tool",
    "WorkspaceInfo",
    "call_tool",
    "define_agent",
    "get_agent",
    "get_model",
    "get_session",
    "get_tool",
    "get_workspace",
    "list_agents",
    "list_models",
    "list_sessions",
    "list_tools",
    "prompt",
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
) -> ToolCallResult:
    return await _client().call_tool(tool, arguments, timeout=timeout)


async def get_workspace() -> WorkspaceInfo:
    return await _client().get_workspace()


async def list_sessions(
    *,
    archived: bool = False,
    limit: int | None = None,
) -> list[SessionSummary]:
    return await _client().list_sessions(archived=archived, limit=limit)


async def get_session(session_id: str) -> Session:
    return await _client().get_session(session_id)


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
