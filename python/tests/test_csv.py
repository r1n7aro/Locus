import os
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

import locus


PAYLOAD = {"filePath": "Locus/knowledge/design/items.csv", "revision": "pair-v1",
    "rowCount": 3, "columnCount": 2, "delimiter": ",", "view": {
        "schema": "locus.csv-view.v1", "headerRows": 1, "rowHeight": 28,
        "wrapText": False, "frozenColumns": 0,
        "columns": {"name": {"sourceIndex": 1, "header": "名称", "width": 220},
                    "id": {"sourceIndex": 0, "header": "编号", "width": 100, "hidden": True}},
        "columnOrder": ["id", "name"],
        "sort": [{"columnId": "id", "direction": "asc"}],
        "filters": [{"columnId": "name", "value": "动作"}],
    }}


class CsvTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        environment = patch.dict(os.environ, {"LOCUS_CHECKOUT_ID": "csv-a",
            "LOCUS_WORKSPACE_GENERATION": "3", "LOCUS_MATERIALIZATION_EPOCH": "7",
            "LOCUS_SDK_EXECUTION_DELEGATION": "owned-write-gate"})
        environment.start()
        self.addCleanup(environment.stop)
        self.client = locus.Client(base_url="http://127.0.0.1/sdk", token="test", current_session_id="session-a")
        self.client.rpc = AsyncMock(return_value=PAYLOAD)
        self.scope = {"checkoutId": "csv-a", "expectedGeneration": 3, "expectedMaterializationEpoch": 7}

    async def test_default_facade_reads_a_typed_snapshot_in_the_injected_checkout(self):
        with patch.object(locus, "_default_client", self.client):
            view = await locus.csv.read_view(PAYLOAD["filePath"])
        self.assertIsInstance(view, locus.CsvViewSnapshot)
        self.assertIsInstance(view.columns["name"], locus.CsvColumnView)
        self.assertEqual(view.columns["name"].source_index, 1)
        self.assertEqual(view.columns["name"].header, "名称")
        self.assertFalse(view.columns["name"].hidden)
        self.assertTrue(view.columns["id"].hidden)
        self.assertEqual(view.sort, [{"column_id": "id", "direction": "asc"}])
        self.assertEqual(view.filters, [{"column_id": "name", "value": "动作"}])
        self.client.rpc.assert_awaited_once_with("csv.read_view", {
            "workspaceRef": self.scope, "filePath": PAYLOAD["filePath"],
        })

    async def test_patch_forwards_version_batch_session_and_write_delegation(self):
        layout = {"wrap_text": True, "columns": [{"target": {"header": "名称"}, "width": 300}], "filters": []}
        with patch.object(locus, "_default_client", self.client):
            result = await locus.csv.patch_view(PAYLOAD["filePath"], layout, expected_revision="pair-v1")
        self.assertEqual(result.revision, "pair-v1")
        self.client.rpc.assert_awaited_once_with("csv.patch_view", {
            "workspaceRef": self.scope, "filePath": PAYLOAD["filePath"],
            "patch": layout, "expectedRevision": "pair-v1", "sessionId": "session-a",
            "executionDelegation": "owned-write-gate",
        })
        self.assertIsNot(self.client.rpc.call_args.args[1]["patch"], layout)

    async def test_explicit_worktree_keeps_generation_and_materialization_epoch(self):
        scope = locus.WorkspaceRef("csv-b", expected_generation=9, expected_materialization_epoch=2)
        worktree = SimpleNamespace(workspace_ref=scope)
        await self.client.csv.read_view("items.csv", worktree=worktree)
        await self.client.csv.patch_view("items.csv", {"row_height": 40}, expected_revision="v", workspace_ref=scope)
        for call in self.client.rpc.call_args_list:
            self.assertEqual(call.args[1]["workspaceRef"], scope.to_payload())

    async def test_invalid_scope_path_or_missing_revision_fails_before_rpc(self):
        with patch.dict(os.environ, {"LOCUS_CHECKOUT_ID": ""}):
            with self.assertRaises(ValueError):
                await self.client.csv.read_view("items.csv")
        with self.assertRaises(ValueError):
            await self.client.csv.read_view("items.csv", workspace_ref=self.scope, worktree=self.scope)
        with self.assertRaisesRegex(ValueError, "companion"):
            await self.client.csv.read_view("items.csv.view")
        with self.assertRaisesRegex(ValueError, "expected_revision"):
            await self.client.csv.patch_view("items.csv", {"row_height": 40}, expected_revision="")
        with self.assertRaisesRegex(ValueError, "dictionary"):
            await self.client.csv.patch_view("items.csv", [], expected_revision="v")
        self.client.rpc.assert_not_awaited()

    async def test_conflict_is_returned_without_retrying_or_claiming_success(self):
        self.client.rpc.side_effect = locus.LocusRpcError("csv.revision_changed: The CSV or its view changed")
        with self.assertRaisesRegex(locus.LocusRpcError, "revision_changed"):
            await self.client.csv.patch_view("items.csv", {"row_height": 40}, expected_revision="old")
        self.assertEqual(self.client.rpc.await_count, 1)

    async def test_row_and_conditional_style_batches_remain_sparse_in_the_rpc(self):
        rules = [
            {"id": "rows", "rows": [1, 100000], "style": {"font": "Arial", "size": 16,
                "bold": True, "color": "accent", "background": "subtle", "border": {"width": 2}}},
            {"id": "pending", "when": {"target": {"header": "名称"}, "op": "contains", "value": "待确认"},
                "style": {"background": "warning-soft"}},
            {"id": "cell", "rows": [1, 1], "columns": [{"source_index": 1}], "style": {"bold": False}},
        ]
        await self.client.csv.patch_view("items.csv", {"styles": {"upsert": rules}}, expected_revision="v1")
        transmitted = self.client.rpc.call_args.args[1]["patch"]["styles"]["upsert"]
        self.assertEqual(transmitted, rules)
        self.assertEqual(len(transmitted), 3)
        self.assertNotIn("columns", transmitted[0])


if __name__ == "__main__":
    unittest.main()
