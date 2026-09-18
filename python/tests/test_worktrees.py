from __future__ import annotations

import asyncio
import os
import unittest
from unittest.mock import patch

import locus


def record(name="a", epoch=1, assignment=None):
    return {"checkoutId": name, "projectId": "project", "root": f"F:/pool/{name}",
        "repoRoot": f"F:/pool/{name}", "projectRelativePath": "", "branch": "refs/heads/test",
        "headOid": "abc", "materializationEpoch": epoch, "managed": True,
        "lifecycle": "active", "dirty": False, "poolSlot": assignment is not None,
        "assignmentId": assignment, "editorVersion": "6000.5.8f1", "lastError": None,
        "workspaceRef": {"checkoutId": name, "expectedGeneration": 7, "expectedMaterializationEpoch": epoch}}


class Client(locus.Client):
    def __init__(self):
        super().__init__(base_url="http://127.0.0.1/sdk", token="test")
        self.calls = []
        self.epoch = 1

    async def rpc(self, method, params=None, *, timeout=None):
        params = params or {}
        self.calls.append((method, params, timeout))
        if method == "worktrees.list":
            return [record()]
        if method in {"worktrees.create", "worktrees.get", "worktrees.import", "worktrees.release"}:
            return record(epoch=self.epoch)
        if method == "worktrees.acquire":
            return {"worktree": record(epoch=self.epoch, assignment="job"), "reused": self.epoch > 1, "preservedLibrary": self.epoch > 1}
        if method == "worktrees.remove":
            return None
        if method == "tools.list":
            return [{"name":"unity_execute"}]
        if method in {"worktrees.discover", "worktrees.operations"}:
            return []
        if method == "tools.call":
            await asyncio.sleep(0)
            return {"name": params["name"], "output": params["workspaceRef"]["checkoutId"], "isError": False}
        if method == "workspace.get":
            return {"path": "F:/pool/a", "checkoutId": "a", "projectId": "project", "workspaceGeneration": 7, "materializationEpoch": 1}
        if method == "merges.prepare":
            return {"workspace_ref": params["workspaceRef"], "job": {"id": "merge", "snapshot": {}, "project_root": "F:/pool/a", "state": "prepared"}}
        if method == "merges.get":
            return {"id": "merge", "snapshot": {}, "project_root": "F:/pool/a", "state": "prepared"}
        if method == "unity.dialog.get":
            return None
        if method == "unity.execution.wait":
            return "done"
        status = {"checkoutId": params["workspaceRef"]["checkoutId"], "projectPath": "F:/pool/a"}
        if method == "unity.editor.status":
            return status
        if method in {"unity.editor.ensure", "unity.editor.restart", "unity.editor.close"}:
            return {"status": status, "launch": {}, "closedProcessIds": [123], "forcedProcessIds": []}
        if method == "unity.dialog.choose":
            return {"dialogId": "d", "choiceId": "c", "invoked": True, "status": "invoked"}
        raise AssertionError(method)


class WorktreeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.client = Client()
        self.env = patch.dict(os.environ, {"LOCUS_CHECKOUT_ID": "source", "LOCUS_WORKSPACE_GENERATION": "3",
            "LOCUS_MATERIALIZATION_EPOCH": "2", "LOCUS_SDK_EXECUTION_DELEGATION": "grant"})
        self.env.start()
        self.addCleanup(self.env.stop)

    async def test_create_and_pool_assignment_keep_epoch_bound_handles(self):
        wt = await self.client.worktrees.create(destination="F:/pool/a", branch="codex/a")
        params = self.client.calls[-1][1]
        self.assertFalse(params["includeDirty"])
        self.assertEqual(params["workspaceRef"]["checkoutId"], "source")
        self.assertEqual(params["executionDelegation"], "grant")
        first = await self.client.worktrees.acquire(pool_root="F:/pool", commit="HEAD", max_slots=2)
        self.assertEqual(first.worktree.assignment_id, "job")
        await self.client.worktrees.release(first.worktree)
        self.assertEqual(self.client.calls[-1][1]["assignmentId"], "job")
        self.client.epoch = 2
        second = await self.client.worktrees.acquire(pool_root="F:/pool", commit="next", max_slots=2)
        self.assertTrue(second.reused and second.preserved_library)
        self.assertEqual(first.worktree.workspace_ref.expected_materialization_epoch, 1)
        self.assertEqual(second.worktree.workspace_ref.expected_materialization_epoch, 2)
        await self.client.worktrees.remove(wt)
        self.assertEqual(self.client.calls[-1][1]["worktree"], wt.workspace_ref.to_payload())

    async def test_parallel_calls_have_independent_targets_and_ambient_directory(self):
        cwd = os.getcwd()
        a, b = [locus.Worktree.from_payload(record(name)) for name in ("a", "b")]
        results = await asyncio.gather(*(self.client.call_tool("unity_execute", {"readonly": True}, worktree=wt) for wt in (a, b)))
        self.assertEqual([result.output for result in results], ["a", "b"])
        self.assertEqual(os.getcwd(), cwd)
        self.assertEqual(os.environ["LOCUS_CHECKOUT_ID"], "source")
        tool = locus.ToolInfo.from_payload({"name": "unity_execute"}, self.client)
        await tool.call({"readonly": False}, worktree=b)
        self.assertEqual(self.client.calls[-1][1]["workspaceRef"], b.workspace_ref.to_payload())
        self.assertEqual(self.client.calls[-1][1]["executionDelegation"], "grant")
        await self.client.call_tool("read", {"filePath": "x"})
        self.assertEqual(self.client.calls[-1][1]["workspaceRef"]["checkoutId"], "source")

    async def test_every_unity_operation_accepts_the_same_worktree_reference(self):
        wt = locus.Worktree.from_payload(record())
        for method, args in [("get_unity_editor_status", {}), ("ensure_unity_editor", {}),
            ("restart_unity_editor", {}), ("close_unity_editor", {}), ("get_unity_dialog", {}),
            ("choose_unity_dialog", {"dialog_id": "d", "choice_id": "c"}),
            ("wait_unity_execution", {"execution_id": "e"})]:
            result = await getattr(self.client, method)(worktree=wt, **args)
            self.assertEqual(self.client.calls[-1][1]["workspaceRef"], wt.workspace_ref.to_payload())
            self.assertNotIn("project", self.client.calls[-1][1])
            if method == "close_unity_editor":
                self.assertEqual(result.closed_process_ids, (123,))

    async def test_merge_and_discovery_share_the_target_contract(self):
        wt = locus.Worktree.from_payload(record())
        job = await self.client.merges.prepare(sources=[{"commits": ["abc"]}], worktree=wt)
        self.assertEqual(job.workspace_ref, wt.workspace_ref.to_payload())
        await self.client.get_unity_editor_status(workspace_ref=job.workspace_ref)
        await self.client.merges.get(job.id, worktree=wt)
        await self.client.list_tools(worktree=wt)
        self.assertEqual(self.client.calls[-1][1]["workspaceRef"], wt.workspace_ref.to_payload())
        tool = await self.client.get_tool("unity_execute", worktree=wt)
        await tool.call({"readonly":True})
        self.assertEqual(self.client.calls[-1][1]["workspaceRef"], wt.workspace_ref.to_payload())
        info = await self.client.get_workspace(worktree=wt)
        self.assertEqual(info.workspace_ref, wt.workspace_ref)
        self.assertEqual((await self.client.worktrees.list())[0].checkout_id, "a")
        self.assertEqual((await self.client.worktrees.get("a")).root, "F:/pool/a")

    async def test_ambiguous_or_unresolved_selectors_fail_before_rpc(self):
        wt = locus.Worktree.from_payload(record())
        with self.assertRaises(ValueError):
            await self.client.call_tool("read", worktree=wt, workspace_ref=wt.workspace_ref)
        with self.assertRaises(ValueError):
            await self.client.ensure_unity_editor(project="F:/other", worktree=wt)
        with self.assertRaises(ValueError):
            await self.client.ensure_unity_editor(worktree="a")
        with self.assertRaises(ValueError):
            await self.client.worktrees.release(wt)
        for size in (0, -1, True, 1.5):
            with self.assertRaises(ValueError):
                await self.client.worktrees.acquire(pool_root="F:/pool", commit="HEAD", max_slots=size)
        self.assertEqual(self.client.calls, [])

    async def test_module_facades_forward_worktree(self):
        with patch("locus._default_client", self.client):
            wt = await locus.worktrees.get("a")
            await locus.close_unity_editor(worktree=wt)
            await locus.get_unity_editor_status(worktree=wt)
            await locus.call_tool("unity_execute", {"readonly": True}, worktree=wt)
            self.assertEqual(self.client.calls[-1][1]["workspaceRef"], wt.workspace_ref.to_payload())


if __name__ == "__main__":
    unittest.main()
