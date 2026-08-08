from __future__ import annotations

import asyncio
import builtins
import hashlib
import re
import uuid
from dataclasses import dataclass, field
from typing import Any, Iterator, TYPE_CHECKING

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
class ToolInfo:
    name: str
    description: str
    input_schema: dict[str, Any]
    source: str

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "ToolInfo":
        return cls(
            name=payload["name"],
            description=payload.get("description", ""),
            input_schema=payload.get("inputSchema") or {},
            source=payload.get("source", "unknown"),
        )


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

    async def cancel(self) -> RunStatus:
        payload = await self.client.rpc("runs.cancel", {"runId": self.run_id})
        return RunStatus.from_payload(payload)

    async def answer(self, question_id: str, answer: str) -> None:
        await self.client.rpc(
            "runs.answer",
            {"questionId": question_id, "answer": answer},
        )
