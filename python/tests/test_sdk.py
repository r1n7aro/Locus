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


if __name__ == "__main__":
    unittest.main()
