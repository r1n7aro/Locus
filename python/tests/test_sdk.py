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
        if method == "unity.editor.status":
            return {
                "projectPath": values["project"],
                "checkoutId": "checkout-1",
                "workspaceGeneration": 7,
                "connected": False,
                "ready": False,
                "processState": "not_running",
                "processId": None,
                "editorPath": None,
                "launchMode": None,
                "headless": False,
                "semanticPhase": "quit",
                "mainThreadBlocked": False,
                "mainThread": {"state": "unknown"},
                "safety": {"canCallUnityApi": False},
                "serviceStatus": "running",
                "readiness": {"phase": "degraded", "revision": 2},
                "connection": {
                    "connected": False,
                    "controlChannelState": "disconnected",
                },
                "semantic": {
                    "phase": "quit",
                    "process": {"state": "not_running"},
                },
            }
        if method == "unity.editor.ensure":
            status = {
                "projectPath": values["project"],
                "checkoutId": "checkout-1",
                "workspaceGeneration": 7,
                "connected": True,
                "ready": True,
                "processState": "running",
                "processId": 4242,
                "editorPath": "E:/Unity/Editor/Unity.exe",
                "launchMode": values.get("mode", "interactive"),
                "headless": values.get("mode") == "headless",
                "semanticPhase": "editing",
                "mainThreadBlocked": False,
                "mainThread": {"state": "idle"},
                "safety": {"canCallUnityApi": True},
                "serviceStatus": "running",
                "readiness": {"phase": "ready", "revision": 3},
                "connection": {
                    "connected": True,
                    "controlChannelState": "ready",
                },
                "semantic": {
                    "phase": "editing",
                    "process": {"state": "running", "pid": 4242},
                },
            }
            return {
                "launched": True,
                "waitUntil": values["waitUntil"],
                "waitedMs": 1250,
                "launch": {
                    "editorPath": "E:/Unity/Editor/Unity.exe",
                    "projectPath": values["project"],
                    "projectVersion": "2022.3.58f1",
                    "processId": 4242,
                    "mode": values.get("mode", "interactive"),
                },
                "status": status,
            }
        if method == "unity.editor.restart":
            status = {
                "projectPath": values["project"],
                "checkoutId": "checkout-1",
                "workspaceGeneration": 7,
                "connected": True,
                "ready": True,
                "processState": "running",
                "processId": 5252,
                "editorPath": "E:/Unity/Editor/Unity.exe",
                "launchMode": values.get("mode", "interactive"),
                "headless": values.get("mode") == "headless",
                "semanticPhase": "editing",
                "mainThreadBlocked": False,
                "mainThread": {"state": "idle"},
                "safety": {"canCallUnityApi": True},
                "serviceStatus": "running",
                "readiness": {"phase": "ready", "revision": 4},
                "connection": {"connected": True, "controlChannelState": "ready"},
                "semantic": {
                    "phase": "editing",
                    "process": {"state": "running", "pid": 5252},
                },
            }
            return {
                "closedProcessIds": [4242],
                "forcedProcessIds": [],
                "waitUntil": values["waitUntil"],
                "waitedMs": 2250,
                "launch": {
                    "editorPath": "E:/Unity/Editor/Unity.exe",
                    "projectPath": values["project"],
                    "projectVersion": "2022.3.58f1",
                    "processId": 5252,
                    "mode": values.get("mode", "interactive"),
                },
                "status": status,
            }
        if method == "unity.dialog.get":
            return {
                "code": "unity_modal_dialog_blocked",
                "dialogId": "dialog-1",
                "project": values["project"],
                "title": "Scene changed",
                "message": "Reload it?",
                "choices": [
                    {"id": "choice-0", "label": "Reload"},
                    {"id": "choice-1", "label": "Keep Changes"},
                ],
                "mainThreadBlocked": True,
                "openedAtMs": 123,
            }
        if method == "unity.dialog.choose":
            return {
                "dialogId": values["dialogId"],
                "choiceId": values["choiceId"],
                "label": "Reload",
                "invoked": True,
            }
        if method == "unity.execution.wait":
            return "MODAL_PROBE:cancelled"
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
                "lastFastMode": True,
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

    async def test_agent_prompt_forwards_workspace_scope(self) -> None:
        client = _FakeClient()
        agent = locus.Agent(
            name="Scoped",
            id="scoped",
            system_prompt="Stay in the selected checkout.",
            client=client,
        )
        workspace_ref = locus.WorkspaceRef(
            checkout_id="checkout-1",
            expected_generation=7,
        )

        await agent.prompt("run", workspace_ref=workspace_ref)

        self.assertEqual(client.calls[0]["workspace_ref"], workspace_ref)

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

    async def test_reads_and_resolves_unity_dialog_without_tool_schema(self) -> None:
        client = _SdkSurfaceClient()

        dialog = await client.get_unity_dialog(project=r"F:\Project")
        assert dialog is not None
        result = await client.choose_unity_dialog(
            project=dialog.project,
            dialog_id=dialog.dialog_id,
            choice_id=dialog.choices[0].id,
        )
        output = await client.wait_unity_execution(
            project=dialog.project,
            execution_id="exec-1",
        )

        self.assertEqual(dialog.title, "Scene changed")
        self.assertTrue(dialog.main_thread_blocked)
        self.assertEqual(dialog.choices[1].label, "Keep Changes")
        self.assertTrue(result.invoked)
        self.assertEqual(output, "MODAL_PROBE:cancelled")
        self.assertIn(
            ("unity.dialog.get", {"project": r"F:\Project"}),
            client.calls,
        )
        self.assertIn(
            (
                "unity.execution.wait",
                {"project": r"F:\Project", "executionId": "exec-1"},
            ),
            client.calls,
        )
        self.assertIn(
            (
                "unity.dialog.choose",
                {
                    "project": r"F:\Project",
                    "dialogId": "dialog-1",
                    "choiceId": "choice-0",
                },
            ),
            client.calls,
        )

    async def test_queries_and_ensures_unity_editor_lifecycle(self) -> None:
        client = _SdkSurfaceClient()

        before = await client.get_unity_editor_status(project=r"F:\Project")
        ensured = await client.ensure_unity_editor(
            project=r"F:\Project",
            wait_until="ready",
            timeout=12.5,
        )

        self.assertEqual(before.process_state, "not_running")
        self.assertEqual(before.workspace_ref.checkout_id, "checkout-1")
        self.assertEqual(before.workspace_ref.expected_generation, 7)
        self.assertEqual(before.semantic_phase, "quit")
        self.assertFalse(before.is_running)
        self.assertTrue(ensured.launched)
        self.assertTrue(ensured.status.ready)
        self.assertTrue(ensured.status.is_running)
        self.assertEqual(ensured.status.readiness_phase, "ready")
        assert ensured.launch is not None
        self.assertEqual(ensured.launch.project_version, "2022.3.58f1")
        self.assertIn(
            ("unity.editor.status", {"project": r"F:\Project"}),
            client.calls,
        )
        self.assertIn(
            (
                "unity.editor.ensure",
                {
                    "project": r"F:\Project",
                    "mode": "interactive",
                    "waitUntil": "ready",
                    "timeoutMs": 12500,
                },
            ),
            client.calls,
        )

    async def test_unity_ensure_validates_wait_target_and_timeout(self) -> None:
        client = _SdkSurfaceClient()

        with self.assertRaises(ValueError):
            await client.ensure_unity_editor(project=r"F:\Project", wait_until="running")
        with self.assertRaises(ValueError):
            await client.ensure_unity_editor(project=r"F:\Project", timeout=0)
        with self.assertRaises(ValueError):
            await client.ensure_unity_editor(project=r"F:\Project", timeout=1801)
        with self.assertRaises(ValueError):
            await client.ensure_unity_editor(project=r"F:\Project", mode="background")

    async def test_restarts_unity_editor_and_reports_closed_processes(self) -> None:
        client = _SdkSurfaceClient()

        restarted = await client.restart_unity_editor(
            project=r"F:\Project",
            wait_until="connected",
            timeout=15,
        )

        self.assertEqual(restarted.closed_process_ids, (4242,))
        self.assertEqual(restarted.forced_process_ids, ())
        self.assertEqual(restarted.launch.process_id, 5252)
        self.assertTrue(restarted.status.connected)
        self.assertIn(
            (
                "unity.editor.restart",
                {
                    "project": r"F:\Project",
                    "mode": "interactive",
                    "waitUntil": "connected",
                    "timeoutMs": 15000,
                    "force": False,
                },
            ),
            client.calls,
        )

    async def test_status_exposes_main_thread_blocking_and_dialog(self) -> None:
        payload = {
            "projectPath": r"F:\Project",
            "checkoutId": "checkout-1",
            "workspaceGeneration": 7,
            "connected": True,
            "ready": False,
            "processState": "running",
            "processId": 4242,
            "launchMode": "interactive",
            "headless": False,
            "semanticPhase": "editing",
            "mainThreadBlocked": True,
            "blockingReason": "modal_dialog",
            "mainThread": {"state": "blocked"},
            "safety": {"canCallUnityApi": False, "recommendedAction": "resolve_dialog"},
            "blockingDialog": {
                "code": "unity_modal_dialog_blocked",
                "dialogId": "dialog-1",
                "project": r"F:\Project",
                "title": "Scene changed",
                "message": "Reload it?",
                "choices": [{"id": "choice-0", "label": "Reload"}],
                "mainThreadBlocked": True,
                "openedAtMs": 123,
            },
            "blockingDialogRecoverable": True,
            "connection": {},
            "semantic": {},
        }

        status = locus.UnityEditorStatus.from_payload(payload)

        self.assertTrue(status.main_thread_blocked)
        self.assertFalse(hasattr(status, "needs_user"))
        self.assertNotIn("needsUser", status.semantic)
        self.assertFalse(status.can_call_unity_api)
        self.assertTrue(status.blocking_dialog_recoverable)
        self.assertEqual(status.blocking_reason, "modal_dialog")
        assert status.blocking_dialog is not None
        self.assertEqual(status.blocking_dialog.title, "Scene changed")

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
