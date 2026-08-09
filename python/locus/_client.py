from __future__ import annotations

import asyncio
import json
import os
import urllib.error
import urllib.request
import uuid
from typing import Any

_DEFAULT_TOOL_TIMEOUT = 120.0


class LocusSdkError(RuntimeError):
    """Base error raised by the Locus Python SDK."""


class LocusUnavailableError(LocusSdkError):
    """The Python process was not launched by a running Locus instance."""


class LocusRpcError(LocusSdkError):
    """The local Locus bridge rejected an SDK operation."""

    def __init__(self, message: str, *, code: int | None = None) -> None:
        super().__init__(message)
        self.code = code


class LocusToolError(LocusSdkError):
    """A directly invoked Locus tool returned an error result."""

    def __init__(self, tool_name: str, message: str) -> None:
        super().__init__(f"Tool '{tool_name}' failed: {message}")
        self.tool_name = tool_name


class LocusRunError(LocusSdkError):
    """A Locus Agent run reached the error state."""

    def __init__(self, run_id: str, message: str) -> None:
        super().__init__(f"Run '{run_id}' failed: {message}")
        self.run_id = run_id


class Client:
    """Async client for the current local Locus desktop process.

    ``base_url`` and ``token`` default to the ephemeral values injected by
    Locus. Supplying them explicitly is useful for tests; application code
    should rely on the injected connection.
    """

    def __init__(
        self,
        *,
        base_url: str | None = None,
        token: str | None = None,
        request_timeout: float = 35.0,
    ) -> None:
        self.base_url = (base_url or os.environ.get("LOCUS_SDK_URL", "")).strip()
        self.token = (token or os.environ.get("LOCUS_SDK_TOKEN", "")).strip()
        self.request_timeout = request_timeout

    def _require_connection(self) -> None:
        if self.base_url and self.token:
            return
        raise LocusUnavailableError(
            "Locus SDK connection is unavailable. Run this code with the Python "
            "runtime selected inside a running Locus desktop instance."
        )

    def _rpc_sync(
        self,
        method: str,
        params: dict[str, Any] | None,
        timeout: float | None,
    ) -> Any:
        self._require_connection()
        request_id = uuid.uuid4().hex
        body = json.dumps(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params or {},
            },
            ensure_ascii=False,
        ).encode("utf-8")
        request = urllib.request.Request(
            self.base_url,
            data=body,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(
                request,
                timeout=self.request_timeout if timeout is None else timeout,
            ) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace").strip()
            raise LocusRpcError(detail or f"Locus bridge returned HTTP {error.code}") from error
        except (urllib.error.URLError, TimeoutError, OSError) as error:
            raise LocusUnavailableError(f"Could not reach the local Locus bridge: {error}") from error
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise LocusRpcError(f"Locus bridge returned invalid JSON: {error}") from error

        if payload.get("id") != request_id:
            raise LocusRpcError("Locus bridge returned a mismatched request id")
        if error := payload.get("error"):
            raise LocusRpcError(
                str(error.get("message") or "Locus SDK request failed"),
                code=error.get("code"),
            )
        return payload.get("result")

    async def rpc(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
    ) -> Any:
        return await asyncio.to_thread(self._rpc_sync, method, params, timeout)

    async def list_agents(self) -> list["Agent"]:
        from ._models import Agent

        rows = await self.rpc("agents.list")
        return [Agent.from_payload(row, self) for row in rows]

    async def list_models(self, *, available_only: bool = True) -> list["ModelInfo"]:
        from ._models import ModelInfo

        rows = await self.rpc("models.list", {"availableOnly": available_only})
        return [ModelInfo.from_payload(row) for row in rows]

    async def list_tools(self) -> list["ToolInfo"]:
        from ._models import ToolInfo

        rows = await self.rpc("tools.list")
        return [ToolInfo.from_payload(row, self) for row in rows]

    async def get_model(self, model_id: str, *, include_unavailable: bool = True) -> "ModelInfo":
        for model in await self.list_models(available_only=not include_unavailable):
            if model.id == model_id:
                return model
        raise LocusRpcError(f"Unknown model '{model_id}'")

    async def get_tool(self, name: str) -> "ToolInfo":
        for tool in await self.list_tools():
            if tool.name == name:
                return tool
        raise LocusRpcError(f"Unknown tool '{name}'")

    async def call_tool(
        self,
        tool: str | "ToolInfo",
        arguments: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
    ) -> "ToolCallResult":
        from ._models import ToolCallResult, ToolInfo

        if timeout is not None and timeout <= 0:
            raise ValueError("timeout must be positive")
        name = tool.name if isinstance(tool, ToolInfo) else str(tool).strip()
        if not name:
            raise ValueError("tool name cannot be empty")
        effective_timeout = _DEFAULT_TOOL_TIMEOUT if timeout is None else timeout
        payload = await self.rpc(
            "tools.call",
            {
                "name": name,
                "arguments": arguments or {},
                "timeoutMs": None if timeout is None else max(1, int(timeout * 1000)),
            },
            timeout=effective_timeout + 5.0,
        )
        return ToolCallResult.from_payload(payload)

    async def get_workspace(self) -> "WorkspaceInfo":
        from ._models import WorkspaceInfo

        payload = await self.rpc("workspace.get")
        return WorkspaceInfo.from_payload(payload)

    async def list_sessions(
        self,
        *,
        archived: bool = False,
        limit: int | None = None,
    ) -> list["SessionSummary"]:
        from ._models import SessionSummary

        if limit is not None and limit <= 0:
            raise ValueError("limit must be positive")
        rows = await self.rpc("sessions.list", {"archived": archived, "limit": limit})
        return [SessionSummary.from_payload(row, self) for row in rows]

    async def get_session(self, session_id: str) -> "Session":
        from ._models import Session

        payload = await self.rpc("sessions.get", {"sessionId": session_id})
        return Session.from_payload(payload, self)

    def define_agent(
        self,
        agent_id: str,
        *,
        system_prompt: str,
        tools: list[str | Tool] | tuple[str | Tool, ...] = (),
        name: str | None = None,
        description: str | None = None,
        sub_agents: list[str] | tuple[str, ...] = (),
        default_effort: str | None = None,
        model_recommendation: str | None = None,
    ) -> "Agent":
        from ._models import Agent

        return Agent(
            name=name or agent_id,
            id=agent_id,
            description=description or "",
            system_prompt=system_prompt,
            tools=list(tools),
            sub_agents=list(sub_agents),
            default_effort=default_effort,
            model_recommendation=model_recommendation,
            client=self,
        )

    async def get_agent(self, agent_id: str) -> "Agent":
        for agent in await self.list_agents():
            if agent.id == agent_id:
                return agent
        raise LocusRpcError(f"Unknown agent '{agent_id}'")

    async def prompt_agent(
        self,
        agent_id: str,
        prompt: str,
        *,
        agent_spec: dict[str, Any] | None = None,
        session_id: str | None = None,
        title: str | None = None,
        model: str | None = None,
        effort: str | None = None,
        fast_mode: bool | None = None,
        mode: str = "build",
        session_type: str = "chat",
        knowledge_mode: str = "full",
        subagent_models: dict[str, str] | None = None,
    ) -> "Run":
        from ._models import Run

        payload = await self.rpc(
            "agents.prompt",
            {
                "agentId": agent_id,
                "agentSpec": agent_spec,
                "prompt": prompt,
                "sessionId": session_id,
                "title": title,
                "model": model,
                "effort": effort,
                "fastMode": fast_mode,
                "mode": mode,
                "sessionType": session_type,
                "knowledgeMode": knowledge_mode,
                "subagentModels": subagent_models,
            },
        )
        return Run(
            run_id=payload["runId"],
            session_id=payload["sessionId"],
            client=self,
        )


from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from ._models import (
        Agent,
        ModelInfo,
        Run,
        Session,
        SessionSummary,
        ToolCallResult,
        ToolInfo,
        WorkspaceInfo,
    )
    from ._tools import Tool
