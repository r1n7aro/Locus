from __future__ import annotations

import os
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

import locus


class SessionHistoryTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.environment = patch.dict(os.environ, {
            "LOCUS_CHECKOUT_ID": "checkout-a", "LOCUS_WORKSPACE_GENERATION": "7",
            "LOCUS_MATERIALIZATION_EPOCH": "2",
        })
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.client = locus.Client(base_url="http://127.0.0.1/sdk", token="test")
        self.reference = {"checkoutId": "checkout-a", "expectedGeneration": 7, "expectedMaterializationEpoch": 2}
        self.client.rpc = AsyncMock()

    async def test_lists_current_checkout_and_archive_filter(self):
        self.client.rpc.return_value = [{"id": "session-a", "title": "History", "updatedAt": 7}]
        rows = await self.client.list_sessions(archived=True, limit=10)
        self.assertEqual(rows[0].id, "session-a")
        params = self.client.rpc.call_args.args[1]
        self.assertEqual(params["workspaceRef"], self.reference)
        self.assertTrue(params["archived"])
        other = locus.WorkspaceRef("checkout-b", expected_generation=9)
        await self.client.list_running_sessions(worktree=SimpleNamespace(workspace_ref=other))
        params = self.client.rpc.call_args.args[1]
        self.assertEqual(params["workspaceRef"]["checkoutId"], "checkout-b")
        self.assertTrue(params["runningOnly"])

    async def test_search_returns_typed_hits_and_continuation(self):
        self.client.rpc.return_value = {
            "matches": [{"sessionId": "s", "sessionTitle": "Shader", "messageId": "m",
                         "messageRowId": 42, "role": "assistant", "field": "content", "excerpt": "命中 Shader"}],
            "nextCursor": "cursor-1", "scannedMessages": 12, "scannedBytes": 4096,
        }
        page = await self.client.search_sessions("Shader", archived=True, session_id="s", limit=1)
        self.assertIsInstance(page, locus.SessionSearchPage)
        self.assertEqual(page.matches[0].message_row_id, 42)
        self.assertTrue(page.has_more)
        self.assertEqual(page.scanned_messages, 12)
        self.assertEqual(page.scanned_bytes, 4096)
        params = self.client.rpc.call_args.args[1]
        self.assertEqual(params["workspaceRef"], self.reference)
        self.assertTrue(params["archived"])
        self.assertEqual(params["sessionId"], "s")
        self.client.rpc.return_value = {"matches": [], "nextCursor": "cursor-2"}
        progress = await self.client.search_sessions("Shader", archived=True, session_id="s", limit=1, cursor=page.next_cursor)
        self.assertEqual(self.client.rpc.call_args.args[1]["cursor"], "cursor-1")
        self.assertTrue(progress.has_more)
        self.client.rpc.return_value = {"matches": [], "nextCursor": None}
        end = await self.client.search_sessions("Shader", archived=True, session_id="s", cursor=progress.next_cursor)
        self.assertFalse(end.has_more)

    async def test_reads_pages_without_loading_full_session(self):
        self.client.rpc.side_effect = [
            {"messages": [{"id": "m", "role": "assistant", "content": "done", "createdAt": 3,
                           "toolCalls": [{"name": "read"}]}], "oldestMessageRowId": 10, "hasMoreHistory": True},
            {"messages": [], "hasMoreHistory": False},
        ]
        page = await self.client.read_session("s", limit=2)
        self.assertIsInstance(page, locus.SessionMessagePage)
        self.assertEqual(page.messages[0].raw["toolCalls"][0]["name"], "read")
        end = await self.client.read_session("s", before_row_id=page.oldest_message_row_id, limit=2)
        self.assertFalse(end.has_more_history)
        self.assertIsNone(end.oldest_message_row_id)
        self.assertEqual(self.client.rpc.call_args.args[1]["beforeRowId"], 10)
        self.assertTrue(all(call.args[0] == "sessions.read" for call in self.client.rpc.call_args_list))

    async def test_module_api_forwards_scope_and_cursor(self):
        with patch("locus._default_client", self.client):
            self.client.rpc.return_value = []
            await locus.list_sessions(archived=True, workspace_ref=locus.WorkspaceRef("explicit"))
            self.assertEqual(self.client.rpc.call_args.args[1]["workspaceRef"]["checkoutId"], "explicit")
            self.client.rpc.return_value = {"matches": [], "nextCursor": None}
            await locus.search_sessions("text", cursor="next")
            self.assertEqual(self.client.rpc.call_args.args[1]["cursor"], "next")
            self.client.rpc.return_value = {"messages": [], "hasMoreHistory": False}
            await locus.read_session("s", before_row_id=9)
            self.assertEqual(self.client.rpc.call_args.args[1]["beforeRowId"], 9)

    async def test_rejects_invalid_parameters_before_rpc(self):
        for query in ["", "  ", "x" * 1001, "a\0b"]:
            with self.assertRaises(ValueError):
                await self.client.search_sessions(query)
        for kwargs in [{"limit": 0}, {"limit": 101}, {"limit": True}, {"cursor": ""}, {"cursor": "x" * 4097}, {"cursor": 1}, {"session_id": " "}]:
            with self.assertRaises(ValueError):
                await self.client.search_sessions("q", **kwargs)
        for kwargs in [{"limit": 0}, {"limit": 1001}, {"before_row_id": 0}, {"before_row_id": True}, {"before_row_id": 2**63}]:
            with self.assertRaises(ValueError):
                await self.client.read_session("s", **kwargs)
        with self.assertRaises(ValueError):
            await self.client.read_session(" ")
        with self.assertRaises(ValueError):
            await self.client.list_sessions(worktree=locus.WorkspaceRef("a"), workspace_ref=locus.WorkspaceRef("b"))
        with patch.dict(os.environ, {"LOCUS_CHECKOUT_ID": ""}):
            with self.assertRaises(ValueError):
                await self.client.search_sessions("q")
            with self.assertRaises(ValueError):
                await self.client.read_session("s")
        self.client.rpc.assert_not_called()


if __name__ == "__main__":
    unittest.main()
