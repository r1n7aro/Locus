from __future__ import annotations

import asyncio
import builtins
import hashlib
import re
import uuid
from dataclasses import dataclass, field
from typing import Any, AsyncIterator, Iterator, TYPE_CHECKING

from ._callbacks import callbacks
from ._tools import Tool

if TYPE_CHECKING:
    from ._client import Client


def _agent_id(name: str) -> str:
    slug = re.sub(r"[^a-z0-9_-]+", "-", name.strip().lower()).strip("-_")
    if slug:
        return slug[:64]
    digest = hashlib.sha256(name.encode("utf-8")).hexdigest()[:12]
    return f"agent-{digest}"


@dataclass(frozen=True, slots=True)
class ModelInfo:
    id: str
    name: str
    provider: str
    available: bool
    context_window: int | None = None
    default_effort: str | None = None
    supported_efforts: tuple[str, ...] = ()
    additional_speed_tiers: tuple[str, ...] = ()
    is_default: bool = False
    unavailable_reason: str | None = None
    custom_provider_id: str | None = None
    custom_provider_name: str | None = None
    custom_model_name: str | None = None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "ModelInfo":
        return cls(
            id=payload["id"],
            name=payload.get("name", payload["id"]),
            provider=payload.get("provider", "unknown"),
            available=bool(payload.get("available", True)),
            context_window=payload.get("contextWindow"),
            default_effort=payload.get("defaultEffort"),
            supported_efforts=tuple(payload.get("supportedEfforts") or ()),
            additional_speed_tiers=tuple(payload.get("additionalSpeedTiers") or ()),
            is_default=bool(payload.get("isDefault")),
            unavailable_reason=payload.get("unavailableReason"),
            custom_provider_id=payload.get("customProviderId"),
            custom_provider_name=payload.get("customProviderName"),
            custom_model_name=payload.get("customModelName"),
        )


@dataclass(frozen=True, slots=True)
class WorkspaceInfo:
    path: str | None
    workspace_id: str | None
    unity_connected: bool

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "WorkspaceInfo":
        return cls(
            path=payload.get("path"),
            workspace_id=payload.get("workspaceId"),
            unity_connected=bool(payload.get("unityConnected")),
        )


@dataclass(frozen=True, slots=True)
class ToolCallImage:
    data: str
    mime_type: str

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "ToolCallImage":
        return cls(data=payload["data"], mime_type=payload.get("mimeType", "image/png"))


@dataclass(frozen=True, slots=True)
class ToolCallResult:
    name: str
    output: str
    is_error: bool
    images: tuple[ToolCallImage, ...] = ()
    workspace_path: str | None = None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "ToolCallResult":
        return cls(
            name=payload["name"],
            output=payload.get("output", ""),
            is_error=bool(payload.get("isError")),
            images=tuple(
                ToolCallImage.from_payload(image) for image in payload.get("images") or ()
            ),
            workspace_path=payload.get("workspacePath"),
        )

    def raise_for_error(self) -> "ToolCallResult":
        if self.is_error:
            from ._client import LocusToolError

            raise LocusToolError(self.name, self.output)
        return self


@dataclass(frozen=True, slots=True)
class ToolInfo:
    name: str
    description: str
    input_schema: dict[str, Any]
    source: str
    mutates_workspace: bool = False
    agent_only: bool = False
    client: "Client | None" = field(default=None, repr=False, compare=False)

    @classmethod
    def from_payload(cls, payload: dict[str, Any], client: "Client | None" = None) -> "ToolInfo":
        return cls(
            name=payload["name"],
            description=payload.get("description", ""),
            input_schema=payload.get("inputSchema") or {},
            source=payload.get("source", "unknown"),
            mutates_workspace=bool(payload.get("mutatesWorkspace")),
            agent_only=bool(payload.get("agentOnly")),
            client=client,
        )

    async def call(
        self,
        arguments: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
    ) -> ToolCallResult:
        if self.client is None:
            raise RuntimeError("ToolInfo is not attached to a Locus client")
        return await self.client.call_tool(self.name, arguments or {}, timeout=timeout)


class Agent:
    """A file-backed Locus agent or a Python-memory inline agent.

    The first prompt creates a Locus session. Later prompts on the same object
    reuse that session by default, preserving provider conversation state and
    prompt-cache continuity.
    """

    def __init__(
        self,
        name: str,
        *,
        system_prompt: str,
        tools: list[str | Tool] | tuple[str | Tool, ...] = (),
        id: str | None = None,
        description: str = "",
        sub_agents: list[str] | tuple[str, ...] = (),
        default_effort: str | None = None,
        model_recommendation: str | None = None,
        client: "Client | None" = None,
    ) -> None:
        if not name.strip():
            raise ValueError("Agent name cannot be empty")
        if not system_prompt.strip():
            raise ValueError("Agent system_prompt cannot be empty")
        self.id = (id or _agent_id(name)).strip()
        self.name = name.strip()
        self.description = description.strip()
        self.system_prompt = system_prompt.strip()
        self.tools = tuple(tools)
        self.sub_agents = tuple(sub_agents)
        self.is_default = False
        self.default_effort = default_effort
        self.model_recommendation = model_recommendation
        self.source = "python"
        self.session_id: str | None = None
        self._client = client
        self._inline = True
        self._prompt_lock: asyncio.Lock | None = None
        self._callback_keys = {
            builtins.id(tool): f"{self.id}:{tool.name}:{uuid.uuid4().hex}"
            for tool in self.tools
            if isinstance(tool, Tool)
        }

    @classmethod
    def from_payload(cls, payload: dict[str, Any], client: "Client") -> "Agent":
        agent = cls.__new__(cls)
        agent.id = payload["id"]
        agent.name = payload.get("name", payload["id"])
        agent.description = payload.get("description", "")
        agent.system_prompt = None
        agent.tools = tuple(payload.get("tools") or ())
        agent.sub_agents = tuple(payload.get("subAgents") or ())
        agent.is_default = bool(payload.get("isDefault"))
        agent.default_effort = payload.get("defaultEffort")
        agent.model_recommendation = payload.get("modelRecommendation")
        agent.source = payload.get("source", "unknown")
        agent.session_id = None
        agent._client = client
        agent._inline = False
        agent._prompt_lock = None
        agent._callback_keys = {}
        return agent

    def _resolved_client(self) -> "Client":
        if self._client is None:
            from ._client import Client

            self._client = Client()
        return self._client

    def _agent_spec(self) -> dict[str, Any] | None:
        if not self._inline:
            return None
        loop = asyncio.get_running_loop()
        locus_tools: list[str] = []
        python_tools: list[dict[str, Any]] = []
        for binding in self.tools:
            if isinstance(binding, str):
                locus_tools.append(binding)
                continue
            callback_key = self._callback_keys[builtins.id(binding)]
            callbacks.register(callback_key, binding, loop)
            python_tools.append(binding.callback_spec(callback_key))
        return {
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "systemPrompt": self.system_prompt,
            "locusTools": locus_tools,
            "pythonTools": python_tools,
            "callbackUrl": callbacks.url if python_tools else None,
            "callbackToken": callbacks.token if python_tools else None,
            "subAgents": list(self.sub_agents),
            "defaultEffort": self.default_effort,
            "modelRecommendation": self.model_recommendation,
        }

    async def prompt(
        self,
        prompt: str,
        *,
        session_id: str | None = None,
        new_session: bool = False,
        title: str | None = None,
        model: str | None = None,
        effort: str | None = None,
        fast_mode: bool | None = None,
        mode: str = "build",
        session_type: str = "chat",
        knowledge_mode: str = "full",
        subagent_models: dict[str, str] | None = None,
    ) -> "Run":
        if new_session and session_id is not None:
            raise ValueError("new_session and session_id cannot be used together")
        if self._prompt_lock is None:
            self._prompt_lock = asyncio.Lock()
        async with self._prompt_lock:
            effective_session = None if new_session else (session_id or self.session_id)
            run = await self._resolved_client().prompt_agent(
                self.id,
                prompt,
                agent_spec=self._agent_spec(),
                session_id=effective_session,
                title=title,
                model=model,
                effort=effort,
                fast_mode=fast_mode,
                mode=mode,
                session_type=session_type,
                knowledge_mode=knowledge_mode,
                subagent_models=subagent_models,
            )
            self.session_id = run.session_id
            return run

    async def run(self, prompt: str, **kwargs: Any) -> "RunResult":
        run = await self.prompt(prompt, **kwargs)
        return await run

    def use_session(self, session_id: str | None) -> None:
        self.session_id = session_id.strip() if session_id else None

    def close(self) -> None:
        callbacks.unregister(tuple(self._callback_keys.values()))


@dataclass(frozen=True, slots=True)
class SessionMessage:
    id: str
    role: str
    content: str
    created_at: int
    raw: dict[str, Any] = field(repr=False, compare=False)

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "SessionMessage":
        return cls(
            id=payload["id"],
            role=payload["role"],
            content=payload.get("content", ""),
            created_at=int(payload.get("createdAt", 0)),
            raw=payload,
        )


@dataclass(frozen=True, slots=True)
class SessionSummary:
    id: str
    title: str
    agent_id: str | None
    session_type: str
    parent_session_id: str | None
    updated_at: int
    runtime_status: str | None = None
    client: "Client | None" = field(default=None, repr=False, compare=False)

    @classmethod
    def from_payload(
        cls,
        payload: dict[str, Any],
        client: "Client | None" = None,
    ) -> "SessionSummary":
        return cls(
            id=payload["id"],
            title=payload.get("title", ""),
            agent_id=payload.get("agentId"),
            session_type=payload.get("sessionType", "chat"),
            parent_session_id=payload.get("parentSessionId"),
            updated_at=int(payload.get("updatedAt", 0)),
            runtime_status=payload.get("runtimeStatus"),
            client=client,
        )

    async def load(self) -> "Session":
        if self.client is None:
            raise RuntimeError("SessionSummary is not attached to a Locus client")
        return await self.client.get_session(self.id)


@dataclass(frozen=True, slots=True)
class RunEvent:
    session_id: str
    run_id: str
    seq: int
    type: str
    payload: dict[str, Any]
    created_at: int

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "RunEvent":
        return cls(
            session_id=payload["sessionId"],
            run_id=payload["runId"],
            seq=int(payload["seq"]),
            type=payload.get("eventType", "unknown"),
            payload=payload.get("payload") or {},
            created_at=int(payload.get("createdAt", 0)),
        )


@dataclass(frozen=True, slots=True)
class Session:
    id: str
    title: str
    agent_id: str | None
    last_model_id: str | None
    last_effort: str | None
    session_type: str
    parent_session_id: str | None
    latest_completed_run_id: str | None
    created_at: int
    updated_at: int
    messages: tuple[SessionMessage, ...]
    pending_inputs: tuple[dict[str, Any], ...]
    runtime: dict[str, Any] | None
    client: "Client" = field(repr=False, compare=False)

    @classmethod
    def from_payload(cls, payload: dict[str, Any], client: "Client") -> "Session":
        return cls(
            id=payload["id"],
            title=payload.get("title", ""),
            agent_id=payload.get("agentId"),
            last_model_id=payload.get("lastModelId"),
            last_effort=payload.get("lastEffort"),
            session_type=payload.get("sessionType", "chat"),
            parent_session_id=payload.get("parentSessionId"),
            latest_completed_run_id=payload.get("latestCompletedRunId"),
            created_at=int(payload.get("createdAt", 0)),
            updated_at=int(payload.get("updatedAt", 0)),
            messages=tuple(
                SessionMessage.from_payload(message) for message in payload.get("messages") or ()
            ),
            pending_inputs=tuple(payload.get("pendingInputs") or ()),
            runtime=payload.get("runtime"),
            client=client,
        )

    async def events(self, *, after_seq: int = 0, limit: int = 500) -> list[RunEvent]:
        payload = await self.client.rpc(
            "sessions.events",
            {"sessionId": self.id, "afterSeq": after_seq, "limit": limit},
        )
        return [RunEvent.from_payload(event) for event in payload]

    async def prompt(
        self,
        text: str,
        *,
        agent: Agent | str | None = None,
        **kwargs: Any,
    ) -> "Run":
        target = agent or self.agent_id
        if target is None:
            raise ValueError("Session has no agent; pass agent= to continue it")
        if isinstance(target, Agent):
            return await target.prompt(text, session_id=self.id, **kwargs)
        return await self.client.prompt_agent(target, text, session_id=self.id, **kwargs)


@dataclass(frozen=True, slots=True)
class RunStatus:
    run_id: str
    session_id: str
    status: str
    completed: bool
    text: str | None = None
    message_id: str | None = None
    error: str | None = None
    runtime: dict[str, Any] | None = None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "RunStatus":
        return cls(
            run_id=payload["runId"],
            session_id=payload["sessionId"],
            status=payload["status"],
            completed=bool(payload.get("completed")),
            text=payload.get("text"),
            message_id=payload.get("messageId"),
            error=payload.get("error"),
            runtime=payload.get("runtime"),
        )

    def raise_for_error(self) -> "RunStatus":
        if self.status == "error" or self.error:
            from ._client import LocusRunError

            raise LocusRunError(self.run_id, self.error or "Locus run failed")
        return self


@dataclass(frozen=True, slots=True)
class RunResult(RunStatus):
    @classmethod
    def from_status(cls, status: RunStatus) -> "RunResult":
        return cls(
            run_id=status.run_id,
            session_id=status.session_id,
            status=status.status,
            completed=status.completed,
            text=status.text,
            message_id=status.message_id,
            error=status.error,
            runtime=status.runtime,
        )


@dataclass(frozen=True, slots=True)
class Run:
    run_id: str
    session_id: str
    client: "Client" = field(repr=False, compare=False)

    async def status(self) -> RunStatus:
        payload = await self.client.rpc("runs.get", {"runId": self.run_id})
        return RunStatus.from_payload(payload)

    async def wait(self, timeout: float | None = None) -> RunResult:
        loop = asyncio.get_running_loop()
        started = loop.time()
        while True:
            remaining = None if timeout is None else max(0.0, timeout - (loop.time() - started))
            if remaining == 0.0:
                raise TimeoutError(f"Locus run '{self.run_id}' did not finish within {timeout}s")
            wait_seconds = 30.0 if remaining is None else min(30.0, remaining)
            payload = await self.client.rpc(
                "runs.wait",
                {"runId": self.run_id, "timeoutMs": max(1, int(wait_seconds * 1000))},
                timeout=wait_seconds + 5.0,
            )
            status = RunStatus.from_payload(payload)
            if status.completed:
                return RunResult.from_status(status)

    def __await__(self) -> Iterator[Any]:
        return self.wait().__await__()

    async def events(self, *, after_seq: int = 0, limit: int = 500) -> list[dict[str, Any]]:
        return await self.client.rpc(
            "runs.events",
            {"runId": self.run_id, "afterSeq": after_seq, "limit": limit},
        )

    async def event_stream(
        self,
        *,
        after_seq: int = 0,
        limit: int = 500,
        poll_interval: float = 0.25,
    ) -> AsyncIterator[RunEvent]:
        """Yield persisted run events until the run reaches a terminal state."""
        if limit <= 0:
            raise ValueError("limit must be positive")
        if poll_interval <= 0:
            raise ValueError("poll_interval must be positive")
        cursor = after_seq
        while True:
            rows = await self.events(after_seq=cursor, limit=limit)
            for row in rows:
                event = RunEvent.from_payload(row)
                cursor = max(cursor, event.seq)
                yield event
            if len(rows) >= limit:
                continue
            status = await self.status()
            if status.completed:
                while True:
                    trailing = await self.events(after_seq=cursor, limit=limit)
                    for row in trailing:
                        event = RunEvent.from_payload(row)
                        cursor = max(cursor, event.seq)
                        yield event
                    if len(trailing) < limit:
                        break
                return
            await asyncio.sleep(poll_interval)

    async def cancel(self) -> RunStatus:
        payload = await self.client.rpc("runs.cancel", {"runId": self.run_id})
        return RunStatus.from_payload(payload)

    async def answer(self, question_id: str, answer: str) -> None:
        await self.client.rpc(
            "runs.answer",
            {"questionId": question_id, "answer": answer},
        )
