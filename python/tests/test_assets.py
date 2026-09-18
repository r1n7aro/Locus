import json
import os
import unittest
from unittest.mock import patch

import locus


SCOPE = {"checkoutId": "A", "expectedGeneration": 4, "expectedMaterializationEpoch": 8}
FIELD = {"object_id": "9007199254740993", "property_path": "/MonoBehaviour/data"}


def setting(value):
    return {"op": "set", **FIELD, "value": value}


class AssetClient(locus.Client):
    def __init__(self):
        super().__init__(base_url="http://127.0.0.1/sdk", token="test")
        self.calls = []
        self.response = {"revision": "read-revision", "objects": [], "diagnostics": []}

    async def rpc(self, method, params=None, **kwargs):
        self.calls.append((method, params))
        return self.response


class AssetTests(unittest.IsolatedAsyncioTestCase):
    async def test_uint64_max_snapshot_round_trips_without_relaxing_signed_ids(self):
        client = AssetClient()
        value = locus.asset_unsigned_integer("18446744073709551615")
        client.response = {"revision": "read-revision", "diagnostics": [], "objects": [{
            "object_id": FIELD["object_id"], "fields": [{"property_path": FIELD["property_path"], "kind": "array", "value": [0, value]}],
        }]}
        assets = client.assets.backend("yaml", workspace_ref=SCOPE)
        snapshot = await assets.read("Assets/A.asset")
        await assets.apply("Assets/A.asset", [setting(snapshot.objects[0]["fields"][0]["value"])], expected_revision=snapshot.revision)
        wire = json.loads(json.dumps(client.calls[-1][1]))
        self.assertEqual(wire["operations"], [setting([0, {"kind": "uint64", "value": "18446744073709551615"}])])
        self.assertEqual(assets.unsigned_integer(0), {"kind": "uint64", "value": "0"})
        client.calls.clear()
        for invalid in [
            {"kind": "uint64", "value": "18446744073709551616"},
            {"kind": "uint64", "value": "-1"}, {"kind": "uint64", "value": 1},
            {"kind": "uint64", "value": "1", "extra": True},
            {"kind": "int64", "value": "18446744073709551615"},
            {"fileID": "18446744073709551615"}, {"rid": "18446744073709551615"},
        ]:
            with self.subTest(value=invalid), self.assertRaises(ValueError):
                await assets.preview("Assets/A.asset", [setting(invalid)])
        with self.assertRaises(ValueError):
            await assets.preview("Assets/A.asset", [{**setting(1), "object_id": "18446744073709551615"}])
        self.assertEqual(client.calls, [])

    def test_unsigned_integer_helper_validates_the_entire_uint64_range(self):
        for value in [True, -1, "-0", "01", "1.0", " 1", 1 << 64]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                locus.asset_unsigned_integer(value)
        self.assertEqual(locus.asset_unsigned_integer((1 << 64) - 1), {"kind": "uint64", "value": "18446744073709551615"})
        with self.assertRaises(ValueError):
            locus.asset_integer((1 << 64) - 1)

    async def test_backend_context_copies_and_binds_workspace_and_never_retargets(self):
        client = AssetClient()
        reference = dict(SCOPE)
        yaml = client.assets.backend("yaml", workspace_ref=reference)
        live = yaml.backend("live")
        reference["checkoutId"] = "B"
        await yaml.read("Assets/A.asset")
        await live.read("Assets/A.asset")
        self.assertEqual([params["backend"] for _, params in client.calls], ["yaml", "live"])
        self.assertTrue(all(params["workspaceRef"] == SCOPE for _, params in client.calls))
        with self.assertRaises(ValueError):
            await yaml.read("Assets/A.asset", workspace_ref={"checkoutId": "B"})

    async def test_default_facade_uses_yaml_and_freezes_session_scope_when_bound(self):
        client = AssetClient()
        with patch.object(locus, "_default_client", client), patch.dict(os.environ, {
            "LOCUS_CHECKOUT_ID": "A", "LOCUS_WORKSPACE_GENERATION": "4", "LOCUS_MATERIALIZATION_EPOCH": "8",
        }):
            assets = locus.assets.backend("yaml")
            with patch.dict(os.environ, {"LOCUS_CHECKOUT_ID": "B"}):
                result = await assets.read("Assets/A.asset")
            self.assertEqual(result.revision, "read-revision")
            self.assertEqual(client.calls[-1][1]["workspaceRef"], SCOPE)
            await locus.assets.read("Assets/A.asset")
            self.assertEqual(client.calls[-1][1]["backend"], "yaml")

    async def test_integer_and_reference_payloads_match_typescript_without_rounding(self):
        client = AssetClient()
        assets = client.assets.backend("yaml", workspace_ref=SCOPE)
        operation = setting({"exact": 9007199254740993, "integral": 2.0, "owner": {"fileID": 9007199254740993},
            "node": {"rid": assets.integer("9223372036854775807")}, "literal": "9007199254740993"})
        await assets.apply("Assets/A.asset", [operation], expected_revision="revision")
        wire = json.loads(json.dumps(client.calls[-1][1]))
        self.assertEqual(wire["operations"], [setting({
            "exact": {"kind": "int64", "value": "9007199254740993"}, "integral": 2,
            "owner": {"fileID": "9007199254740993"}, "node": {"rid": "9223372036854775807"},
            "literal": "9007199254740993",
        })])
        self.assertEqual(operation["value"]["exact"], 9007199254740993)
        self.assertIs(type(wire["operations"][0]["value"]["integral"]), int)

    async def test_rejects_lossy_or_invalid_values_before_dispatch(self):
        client = AssetClient()
        assets = client.assets.backend("yaml", workspace_ref=SCOPE)
        cycle = {}; cycle["self"] = cycle
        for value in [float(9007199254740993), float("nan"), float("inf"), 1 << 63, {"fileID": True}, {1: "value"}, object(), cycle]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                await assets.preview("Assets/A.asset", [setting(value)])
        self.assertEqual(client.calls, [])

    async def test_merge_destination_stages_common_operations_in_the_plan(self):
        client = AssetClient()
        job = locus.MergeJob(client, SCOPE, {"id": "job", "snapshot": {}, "project_root": "project", "state": "prepared"})
        snapshot = await job.assets.read("Assets/A.asset")
        await job.plan().assets.preview("Assets/A.asset", [setting(1)])
        await job.plan().assets.apply("Assets/A.asset", [setting(9007199254740993)], expected_revision=snapshot.revision)
        self.assertEqual([method for method, _ in client.calls], ["merges.assets.read", "merges.assets.preview", "merges.assets.apply"])
        params = client.calls[-1][1]
        self.assertEqual(params["persist"], "plan")
        self.assertEqual(params["destination"], "merge_plan")
        self.assertEqual(params["operations"][0]["value"], {"kind": "int64", "value": "9007199254740993"})
        with self.assertRaises(ValueError):
            await job.assets.apply("Assets/A.asset", [setting(1)], expected_revision="revision", persist="disk")

    async def test_checks_entire_batch_before_sending_one_transaction(self):
        client = AssetClient()
        assets = client.assets.backend("live", workspace_ref=SCOPE)
        entries = [{"path": "Assets/A.asset", "expected_revision": "A", "operations": [setting(1)]},
                   {"path": "Assets/B.asset", "operations": [setting(2)]}]
        with self.assertRaises(ValueError):
            await assets.apply_batch(entries)
        self.assertEqual(client.calls, [])
        entries[1]["expected_revision"] = "B"
        with patch.dict(os.environ, {"LOCUS_SDK_EXECUTION_DELEGATION": "lease"}):
            await assets.apply_batch(entries)
        self.assertEqual(len(client.calls), 1)
        method, payload = client.calls[0]
        self.assertEqual(method, "assets.apply_batch")
        self.assertEqual(payload["entries"], entries)
        self.assertEqual(payload["persist"], "disk")
        self.assertEqual(payload["execution_delegation"], "lease")
        self.assertEqual(payload["workspaceRef"], SCOPE)

    async def test_requires_revisions_and_rejects_misspelled_operation_fields(self):
        client = AssetClient()
        assets = client.assets.backend("yaml", workspace_ref=SCOPE)
        with self.assertRaises(ValueError):
            await assets.apply("Assets/A.asset", [setting(1)], expected_revision="")
        for operation in [{**setting(1), "index": 2}, {**setting(1), "object_id": 11400000},
                          {**setting(1), "property_path": "speed"},
                          {"op": "array_remove", **FIELD, "index": True}]:
            with self.assertRaises(ValueError):
                await assets.preview("Assets/A.asset", [operation])
        self.assertEqual(client.calls, [])

    async def test_discover_and_preview_are_read_operations_and_keep_stable_paths(self):
        client = AssetClient()
        assets = client.assets.backend("live", workspace_ref=SCOPE)
        path = "/MonoBehaviour/references/RefIds/@rid=9007199254740993/data/x"
        await assets.discover("Assets/A.asset", property_path=path, object_id="11400000", offset=10, limit=20)
        await assets.preview_batch([{"path": "Assets/A.asset", "operations": [{"op": "array_resize", **FIELD, "size": 4, "value": 0}]}])
        self.assertEqual(client.calls[0][0], "assets.discover")
        self.assertEqual(client.calls[0][1]["property_path"], path)
        self.assertEqual(client.calls[1][0], "assets.preview_batch")

    def test_integer_helper_enforces_decimal_and_signed_64_bit_bounds(self):
        for value in [True, "01", "-0", "1.0", " 1", 1 << 63, -(1 << 63) - 1]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                locus.asset_integer(value)
        self.assertEqual(locus.asset_integer(-(1 << 63)), {"kind": "int64", "value": "-9223372036854775808"})


if __name__ == "__main__":
    unittest.main()
