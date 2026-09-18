from __future__ import annotations

import os
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

import locus


class AgentRuleTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        environment = patch.dict(os.environ, {
            "LOCUS_CHECKOUT_ID": "checkout-a", "LOCUS_WORKSPACE_GENERATION": "7",
            "LOCUS_MATERIALIZATION_EPOCH": "2", "LOCUS_SDK_EXECUTION_DELEGATION": "write-gate",
        })
        environment.start()
        self.addCleanup(environment.stop)
        self.client = locus.Client(base_url="http://127.0.0.1/sdk", token="test")
        self.client.rpc = AsyncMock()
        self.agent = locus.Agent.from_payload({"id": "unity", "source": "app"}, self.client)
        self.rule = {"key": "default.md", "fileName": "default.md", "title": "Default",
                     "enabled": True, "order": 10, "source": "app", "readOnly": False, "updatedAt": 100}
        self.reference = {"checkoutId": "checkout-a", "expectedGeneration": 7, "expectedMaterializationEpoch": 2}

    async def test_lists_typed_rules_in_current_checkout(self):
        self.client.rpc.return_value = [self.rule]
        rules = await self.agent.list_rules()
        self.assertIsInstance(rules[0], locus.AgentRule)
        self.assertEqual(rules[0].file_name, "default.md")
        self.assertTrue(rules[0].enabled)
        self.assertEqual(rules[0].source, "app")
        self.client.rpc.assert_awaited_once_with("agents.rules.list", {
            "workspaceRef": self.reference, "agentId": "unity", "executionDelegation": "write-gate",
        })

    async def test_reads_markdown_and_saves_with_checkout_write_delegation(self):
        self.client.rpc.return_value = "# 项目规则\n使用本项目规范"
        self.assertEqual(await self.agent.read_rule("项目规则.md"), "# 项目规则\n使用本项目规范")
        self.assertEqual(self.client.rpc.call_args.args[0], "agents.rules.read")
        self.client.rpc.return_value = {**self.rule, "source": "project"}
        saved = await self.agent.save_rule("default.md", "# Local\nUse project conventions")
        self.assertEqual(saved.source, "project")
        params = self.client.rpc.call_args.args[1]
        self.assertEqual(params["content"], "# Local\nUse project conventions")
        self.assertEqual(params["workspaceRef"], self.reference)
        self.assertEqual(params["executionDelegation"], "write-gate")
        self.assertNotIn("enabled", params, "saving must preserve effective enabled state")

    async def test_disables_and_reenables_rules_in_explicit_workspace(self):
        other = locus.WorkspaceRef("checkout-b", expected_generation=9, expected_materialization_epoch=3)
        for enabled in [False, True]:
            self.client.rpc.return_value = {**self.rule, "enabled": enabled}
            rule = await self.agent.set_rule_enabled("default.md", enabled, worktree=SimpleNamespace(workspace_ref=other))
            self.assertEqual(rule.enabled, enabled)
            method, params = self.client.rpc.call_args.args
            self.assertEqual(method, "agents.rules.set_enabled")
            self.assertEqual(params["workspaceRef"], other.to_payload())
            self.assertEqual(params["enabled"], enabled)
        await self.agent.read_rule("default.md", workspace_ref=other)
        self.assertEqual(self.client.rpc.call_args.args[1]["workspaceRef"], other.to_payload())

    async def test_missing_or_ambiguous_scope_fails_before_rpc(self):
        with patch.dict(os.environ, {"LOCUS_CHECKOUT_ID": ""}):
            with self.assertRaisesRegex(ValueError, "workspace_ref"):
                await self.agent.save_rule("local.md", "# Local")
        with self.assertRaisesRegex(ValueError, "not both"):
            await self.agent.list_rules(workspace_ref=self.reference, worktree=self.reference)
        self.client.rpc.assert_not_awaited()

    async def test_inline_agents_and_non_boolean_enablement_are_rejected(self):
        inline = locus.Agent("Inline", id="unity", system_prompt="Local", client=self.client)
        with self.assertRaisesRegex(ValueError, "installed Agent"):
            await inline.save_rule("default.md", "Should not overwrite installed Unity")
        with self.assertRaisesRegex(TypeError, "bool"):
            await self.agent.set_rule_enabled("default.md", "false")
        self.client.rpc.assert_not_awaited()

    async def test_backend_errors_propagate_without_success_results(self):
        self.client.rpc.side_effect = locus.LocusRpcError("Stale checkout generation")
        with self.assertRaisesRegex(locus.LocusRpcError, "Stale checkout"):
            await self.agent.set_rule_enabled("default.md", False)


if __name__ == "__main__":
    unittest.main()
