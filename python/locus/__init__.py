"""Python workflow SDK for the currently running Locus desktop instance.

The SDK reuses Locus's local login state, selected model, agent definitions,
Skill/MCP inventory, session persistence, and tool execution pipeline.
"""

from __future__ import annotations

from typing import Any

from ._client import Client, LocusRpcError, LocusSdkError, LocusUnavailableError
from ._models import Agent, Run, RunResult, RunStatus, ToolInfo
from ._tools import Tool, tool

__all__ = [
    "Agent",
    "Client",
    "LocusRpcError",
    "LocusSdkError",
    "LocusUnavailableError",
    "Run",
    "RunResult",
    "RunStatus",
    "ToolInfo",
    "Tool",
    "define_agent",
    "get_agent",
    "list_agents",
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


async def list_tools() -> list[ToolInfo]:
    return await _client().list_tools()


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


__version__ = "0.1.0"
