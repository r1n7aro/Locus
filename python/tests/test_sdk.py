from __future__ import annotations

import asyncio
import json
import unittest
import urllib.request
from typing import Literal

import locus
from locus._models import Run


class _FakeClient:
    def __init__(self) -> None:
        self.calls: list[dict[str, object]] = []
        self.created = 0

    async def prompt_agent(self, agent_id: str, prompt: str, **kwargs):
        self.calls.append({"agent_id": agent_id, "prompt": prompt, **kwargs})
        session_id = kwargs.get("session_id")
        if session_id is None:
            self.created += 1
            session_id = f"session-{self.created}"
        return Run(run_id=f"run-{len(self.calls)}", session_id=session_id, client=self)


class _SdkSurfaceClient(locus.Client):
    def __init__(self) -> None:
        super().__init__(base_url="http://127.0.0.1/sdk", token="test-token")
        self.calls: list[tuple[str, dict[str, object]]] = []

    async def rpc(self, method: str, params=None, *, timeout=None):
        values = params or {}
        self.calls.append((method, values))
        if method == "models.list":
            return [
                {
                    "id": "openai/gpt-test",
                    "name": "GPT Test",
                    "provider": "openai_codex",
                    "available": True,
                    "contextWindow": 128000,
                    "supportedEfforts": ["low", "high"],
                    "isDefault": True,
                }
            ]
        if method == "tools.list":
            return [
                {
                    "name": "list",
                    "description": "List files.",
                    "inputSchema": {"type": "object"},
                    "source": "builtin",
                    "mutatesWorkspace": False,
                    "agentOnly": False,
                }
            ]
        if method == "tools.call":
            return {
                "name": values["name"],
                "output": "Assets/",
                "isError": False,
                "images": [],
                "workspacePath": "F:/Project",
            }
        if method == "workspace.get":
            return {
                "path": "F:/Project",
                "workspaceId": "workspace-1",
                "unityConnected": True,
            }
        if method == "sessions.list":
            return [
                {
                    "id": "session-1",
                    "title": "Workflow",
                    "agentId": "reviewer",
                    "sessionType": "chat",
                    "parentSessionId": None,
                    "updatedAt": 10,
                }
            ]
        if method == "sessions.get":
            return {
                "id": "session-1",
                "title": "Workflow",
                "agentId": "reviewer",
                "lastModelId": "openai/gpt-test",
                "lastEffort": "high",
                "sessionType": "chat",
                "parentSessionId": None,
                "latestCompletedRunId": "run-1",
                "createdAt": 1,
                "updatedAt": 10,
                "messages": [
                    {"id": "message-1", "role": "assistant", "content": "done", "createdAt": 2}
                ],
                "pendingInputs": [],
                "runtime": None,
            }
        if method == "agents.prompt":
            return {"runId": "run-2", "sessionId": values["sessionId"]}
        raise AssertionError(f"Unexpected RPC method: {method}")


class _RunEventClient:
    def __init__(self) -> None:
        self.event_calls = 0

    async def rpc(self, method: str, params=None, *, timeout=None):
        if method == "runs.events":
            self.event_calls += 1
            if self.event_calls == 1:
                return [
                    {
                        "sessionId": "session-1",
                        "runId": "run-1",
                        "seq": 1,
                        "eventType": "textDelta",
                        "payload": {"delta": "hello"},
                        "createdAt": 1,
                    }
                ]
            return [
                {
                    "sessionId": "session-1",
                    "runId": "run-1",
                    "seq": 2,
                    "eventType": "done",
                    "payload": {"fullText": "hello"},
                    "createdAt": 2,
                }
            ]
        if method == "runs.get":
            return {
                "runId": "run-1",
                "sessionId": "session-1",
                "status": "done",
                "completed": True,
                "text": "hello",
            }
        raise AssertionError(f"Unexpected RPC method: {method}")


class ToolSchemaTests(unittest.TestCase):
    def test_decorator_builds_object_schema_from_annotations(self) -> None:
        @locus.tool(description="Select a build.")
        def select_build(platform: Literal["windows", "mac"], retries: int = 2) -> dict:
            return {"platform": platform, "retries": retries}

        self.assertEqual(select_build.name, "select_build")
        self.assertEqual(select_build.input_schema["type"], "object")
        self.assertEqual(
            select_build.input_schema["properties"]["platform"]["enum"],
            ["windows", "mac"],
        )
        self.assertEqual(select_build.input_schema["properties"]["retries"]["type"], "integer")
        self.assertEqual(select_build.input_schema["required"], ["platform"])

    def test_positional_only_parameters_are_rejected(self) -> None:
        def unsupported(value: str, /) -> str:
            return value

        with self.assertRaises(TypeError):
            locus.tool(unsupported)


class AgentSessionTests(unittest.IsolatedAsyncioTestCase):
    async def test_agent_reuses_created_session_by_default(self) -> None:
        client = _FakeClient()

        @locus.tool
        def policy() -> dict[str, bool]:
            return {"require_tests": True}

        agent = locus.Agent(
            name="Reviewer",
            id="reviewer",
            system_prompt="Review code.",
            tools=["read", policy],
            client=client,
        )

        first = await agent.prompt("first")
        second = await agent.prompt("second")

        self.assertEqual(first.session_id, "session-1")
        self.assertEqual(second.session_id, "session-1")
        self.assertEqual(agent.session_id, "session-1")
        self.assertIsNone(client.calls[0]["session_id"])
        self.assertEqual(client.calls[1]["session_id"], "session-1")
        self.assertEqual(
            client.calls[0]["agent_spec"]["systemPrompt"],
            client.calls[1]["agent_spec"]["systemPrompt"],
        )
        self.assertEqual(
            client.calls[0]["agent_spec"]["pythonTools"][0]["callbackKey"],
            client.calls[1]["agent_spec"]["pythonTools"][0]["callbackKey"],
        )
        agent.close()

    async def test_new_session_explicitly_breaks_sticky_session(self) -> None:
        client = _FakeClient()
        agent = locus.Agent(
            name="Reviewer",
            id="reviewer",
            system_prompt="Review code.",
            client=client,
        )
        first = await agent.prompt("first")
        second = await agent.prompt("second", new_session=True)

        self.assertEqual(first.session_id, "session-1")
        self.assertEqual(second.session_id, "session-2")
        self.assertEqual(agent.session_id, "session-2")

    async def test_python_tool_callback_runs_on_originating_event_loop(self) -> None:
        origin_loop = asyncio.get_running_loop()

        @locus.tool
        async def current_loop(value: int) -> dict[str, object]:
            """Report the active loop and value."""
            return {
                "same_loop": asyncio.get_running_loop() is origin_loop,
                "value": value * 2,
            }

        agent = locus.Agent(
            name="Callback",
            id="callback",
            system_prompt="Use the callback.",
            tools=[current_loop],
            client=_FakeClient(),
        )
        spec = agent._agent_spec()
        assert spec is not None
        tool_spec = spec["pythonTools"][0]
        body = json.dumps(
            {
                "toolKey": tool_spec["callbackKey"],
                "arguments": {"value": 4},
            }
        ).encode("utf-8")
        request = urllib.request.Request(
            spec["callbackUrl"],
            data=body,
            method="POST",
            headers={
                "Authorization": f"Bearer {spec['callbackToken']}",
                "Content-Type": "application/json",
            },
        )

        def invoke() -> dict[str, object]:
            with urllib.request.urlopen(request, timeout=5) as response:
                return json.loads(response.read().decode("utf-8"))

        payload = await asyncio.to_thread(invoke)
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["result"], {"same_loop": True, "value": 8})
        agent.close()


class SdkCoverageTests(unittest.IsolatedAsyncioTestCase):
    async def test_discovers_models_workspace_and_direct_tools(self) -> None:
        client = _SdkSurfaceClient()

        models = await client.list_models()
        workspace = await client.get_workspace()
        listing = await client.get_tool("list")
        result = await listing.call({"path": "."}, timeout=10)

        self.assertEqual(models[0].id, "openai/gpt-test")
        self.assertEqual(models[0].supported_efforts, ("low", "high"))
        self.assertTrue(models[0].is_default)
        self.assertEqual(workspace.path, "F:/Project")
        self.assertTrue(workspace.unity_connected)
        self.assertEqual(result.output, "Assets/")
        self.assertEqual(result.workspace_path, "F:/Project")
        self.assertIn(("models.list", {"availableOnly": True}), client.calls)

    async def test_loads_and_continues_persisted_sessions(self) -> None:
        client = _SdkSurfaceClient()

        summaries = await client.list_sessions(limit=10)
        session = await summaries[0].load()
        run = await session.prompt("continue")

        self.assertEqual(session.messages[0].role, "assistant")
        self.assertEqual(session.messages[0].content, "done")
        self.assertEqual(run.session_id, "session-1")
        prompt_call = next(call for call in client.calls if call[0] == "agents.prompt")
        self.assertEqual(prompt_call[1]["sessionId"], "session-1")

    async def test_tool_and_run_errors_are_explicit(self) -> None:
        failed_tool = locus.ToolCallResult(name="build", output="compile failed", is_error=True)
        with self.assertRaises(locus.LocusToolError):
            failed_tool.raise_for_error()

        failed_run = locus.RunResult(
            run_id="run-error",
            session_id="session-error",
            status="error",
            completed=True,
            error="provider unavailable",
        )
        with self.assertRaises(locus.LocusRunError):
            failed_run.raise_for_error()

    async def test_event_stream_drains_terminal_events(self) -> None:
        run = Run(run_id="run-1", session_id="session-1", client=_RunEventClient())

        events = [event async for event in run.event_stream()]

        self.assertEqual([event.seq for event in events], [1, 2])
        self.assertEqual(events[-1].type, "done")


if __name__ == "__main__":
    unittest.main()
