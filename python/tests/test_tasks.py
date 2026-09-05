import unittest
from unittest.mock import patch

import locus


class TaskClient(locus.Client):
    def __init__(self):
        super().__init__(base_url="http://127.0.0.1/sdk", token="test", current_session_id="parent")
        self.calls = []

    async def rpc(self, method, params=None, **kwargs):
        self.calls.append((method, params))
        if method == "tasks.list":
            return [await self.rpc("tasks.get", {"taskId": "reviewer"})]
        if method == "tasks.send_message":
            return {"messageId": "m1", "taskId": params["taskId"], "status": "queued"}
        return {
            "taskId": params["taskId"], "sessionId": "parent", "toolName": "bash",
            "status": "cancelling" if method == "tasks.cancel" else "completed",
            "notify": True, "createdAt": 1, "updatedAt": 3, "finishedAt": 3,
            "output": "Exit code: 0\nresult", "outputPath": "C:/tasks/output.log",
            "isError": False,
        }


class TaskTests(unittest.IsolatedAsyncioTestCase):
    async def test_module_and_client_methods_use_direct_task_rpc(self):
        client = TaskClient()
        with patch.object(locus, "_default_client", client):
            result = await locus.get_task_status(" task_1 ")
            self.assertTrue(result.done)
            self.assertEqual(result.output, "Exit code: 0\nresult")
            self.assertEqual(result.output_path, "C:/tasks/output.log")
            cancelled = await locus.cancel_task(result.task_id)
            self.assertEqual(cancelled.status, "cancelling")
            self.assertFalse(cancelled.done)
        self.assertEqual(client.calls, [
            ("tasks.get", {"taskId": "task_1", "sessionId": "parent"}),
            ("tasks.cancel", {"taskId": "task_1", "sessionId": "parent"}),
        ])

    async def test_invalid_task_id_is_rejected_without_rpc(self):
        client = TaskClient()
        for method in [client.get_task_status, client.cancel_task]:
            with self.assertRaises(ValueError):
                await method("  ")
        self.assertEqual(client.calls, [])

    async def test_failed_task_is_a_result_not_an_rpc_failure(self):
        task = locus.TaskStatus.from_payload({
            "taskId": "failed", "sessionId": "parent", "toolName": "python",
            "status": "failed", "notify": False, "createdAt": 1, "updatedAt": 2,
            "output": "Traceback", "isError": True,
        })
        self.assertTrue(task.done)
        self.assertTrue(task.is_error)
        self.assertIsNone(task.output_path)


    async def test_list_resume_wait_and_message_use_current_session_without_tool_loading(self):
        client = TaskClient()
        with patch.object(locus, "_default_client", client):
            tasks = await locus.list_tasks()
            self.assertEqual(tasks[0].task_id, "reviewer")
            await locus.resume_task("reviewer", message="continue")
            result = await locus.wait_task("reviewer", timeout=0)
            self.assertTrue(result.done)
            receipt = await locus.send_message("parent", "finding")
            self.assertEqual(receipt.message_id, "m1")
            self.assertEqual(receipt.status, "queued")
        self.assertIn(("tasks.list", {"sessionId": "parent"}), client.calls)
        self.assertIn(("tasks.resume", {"sessionId": "parent", "taskId": "reviewer", "message": "continue"}), client.calls)
        self.assertIn(("tasks.wait", {"sessionId": "parent", "taskId": "reviewer", "timeoutMs": 0}), client.calls)
        self.assertIn(("tasks.send_message", {"sessionId": "parent", "taskId": "parent", "message": "finding"}), client.calls)
        self.assertTrue(all(method.startswith("tasks.") for method, _ in client.calls))

    async def test_task_apis_require_ambient_session(self):
        client = TaskClient()
        client.current_session_id = None
        for call in [lambda: client.list_tasks(), lambda: client.get_task_status("t1"),
                     lambda: client.cancel_task("t1"), lambda: client.resume_task("t1"),
                     lambda: client.wait_task("t1"), lambda: client.send_message("t1", "message")]:
            with self.assertRaises(locus.LocusSdkError):
                await call()
        self.assertEqual(client.calls, [])

    async def test_wait_timeout_returns_running_snapshot_without_cancellation(self):
        client = TaskClient()
        async def rpc(method, params, **kwargs):
            client.calls.append((method, params))
            return {"taskId": "t1", "sessionId": "parent", "toolName": "python", "status": "running",
                    "notify": True, "createdAt": 1, "updatedAt": 2}
        client.rpc = rpc
        self.assertFalse((await client.wait_task("t1", timeout=0)).done)
        self.assertEqual(len(client.calls), 1)
        self.assertEqual(client.calls[0][0], "tasks.wait")

    async def test_task_arguments_are_validated(self):
        client = TaskClient()
        for timeout in [-1, 301, float("nan"), float("inf")]:
            with self.assertRaises(ValueError):
                await client.wait_task("t1", timeout=timeout)
        for address, text in [("", "ok"), ("reviewer", " "), ("reviewer", "x" * 32001)]:
            with self.assertRaises(ValueError):
                await client.send_message(address, text)
        with self.assertRaises(ValueError):
            await client.resume_task(" ")
        self.assertEqual(client.calls, [])

    async def test_continuation_metadata_is_exposed(self):
        task = locus.TaskStatus.from_payload({
            "taskId": "reviewer", "sessionId": "parent", "toolName": "subagent",
            "status": "failed", "notify": True, "createdAt": 1, "updatedAt": 3,
            "attempt": 2, "startedAt": 2, "childSessionId": "child", "canResume": True,
        })
        self.assertEqual(task.attempt, 2)
        self.assertEqual(task.child_session_id, "child")
        self.assertTrue(task.can_resume)
