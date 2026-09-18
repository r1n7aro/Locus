import os
import json
import unittest
from unittest.mock import patch

import locus


class MergeClient(locus.Client):
    def __init__(self):
        super().__init__(base_url="http://127.0.0.1/sdk", token="test")
        self.calls = []

    async def rpc(self, method, params=None, **kwargs):
        self.calls.append((method, params))
        if method == "merges.prepare":
            return {"workspace_ref": {"checkoutId": "destination", "expectedGeneration": 7},
                    "job": {"id": "job-1", "snapshot": {"head": "head"}, "project_root": "C:/project", "state": "prepared"}}
        return {"ready_to_apply": True, "plan_hash": "hash", "state": "prepared"}


class MergeTests(unittest.IsolatedAsyncioTestCase):
    async def test_file_scope_and_snapshot_pagination(self):
        client = MergeClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}],
            workspace_ref=locus.WorkspaceRef("A"), paths=["Assets/Blockout.unity"], mode="files")
        self.assertEqual(client.calls[0][1]["paths"], ["Assets/Blockout.unity"])
        self.assertEqual(client.calls[0][1]["mode"], "files")
        await job.snapshot_page(kind="dependencies", offset=10, limit=25)
        self.assertEqual(client.calls[-1][0], "merges.snapshot")
        self.assertEqual(client.calls[-1][1]["kind"], "dependencies")
        self.assertEqual(client.calls[-1][1]["offset"], 10)
        for options in ({"mode": "files"}, {"paths": []}, {"paths": "Assets/a.asset"},
                        {"paths": [None]}, {"mode": "unknown"}):
            with self.assertRaises(ValueError):
                await client.merges.prepare(sources=[{"commits": ["one"]}], **options)
        with self.assertRaises(ValueError):
            await job.snapshot_page(kind="live")

    async def test_typed_i64_value_is_transported_without_float_or_string_conversion(self):
        from locus._merges import MergeJob

        client = locus.Client(base_url="http://127.0.0.1/sdk", token="test")
        job = MergeJob(client, {"checkoutId": "A", "expectedMaterializationEpoch": 7},
            {"id": "job-1", "snapshot": {}, "project_root": "C:/project", "state": "prepared"})
        requests = []

        class Response:
            def __init__(self, request):
                self.payload = json.loads(request.data)
                requests.append((request.data, self.payload))

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                pass

            def read(self):
                return json.dumps({"jsonrpc": "2.0", "id": self.payload["id"],
                    "result": {"references": [{"rid": "9007199254740993"}]}}).encode()

        with patch("urllib.request.urlopen", side_effect=lambda request, **_: Response(request)):
            await job.plan().fields.set("Assets/a.asset", object_id="9007199254740993",
                property_path="/MonoBehaviour/node", value={"rid": int("9007199254740993")})
            view = await job.inspect_asset("Assets/a.asset", object_id="9007199254740993")
        raw, payload = requests[0]
        self.assertIn(b'"rid": 9007199254740993', raw)
        self.assertIsInstance(payload["params"]["value"]["rid"], int)
        self.assertEqual(payload["params"]["object_id"], "9007199254740993")
        self.assertEqual(payload["params"]["workspaceRef"]["expectedMaterializationEpoch"], 7)
        self.assertEqual(view.references[0]["rid"], "9007199254740993")

    async def test_plan_is_bound_to_returned_destination_and_does_not_auto_include(self):
        client = MergeClient()
        with patch.dict(os.environ, {"LOCUS_CHECKOUT_ID": "source", "LOCUS_WORKSPACE_GENERATION": "3"}):
            job = await client.merges.prepare(sources=[{"commits": ["one", "three"]}],
                destination={"kind": "new_branch", "name": "integration", "location": "new_worktree"})
        self.assertEqual(len(client.calls), 1)
        self.assertEqual(client.calls[0][1]["workspaceRef"]["checkoutId"], "source")
        plan = await job.new_plan()
        await plan.include(change_ids=["health"])
        await plan.exclude(change_ids=["name"])
        await plan.defer(change_ids=["event"])
        await plan.files.take("Assets/model.fbx", version={"side": "source", "commit": "three"})
        preview = await plan.preview()
        await plan.apply(expected_plan_hash=preview.plan_hash)
        self.assertTrue(preview.ready_to_apply)
        for _, payload in client.calls[1:]:
            self.assertEqual(payload["workspaceRef"], {"checkoutId": "destination", "expectedGeneration": 7})
        self.assertEqual([method for method, _ in client.calls][-1], "merges.apply")
        self.assertFalse(any(method in {"merges.stage", "merges.commit"} for method, _ in client.calls))

    async def test_field_object_and_commit_controls_remain_explicit(self):
        client = MergeClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}], workspace_ref=locus.WorkspaceRef("A", 4))
        plan = job.plan()
        await plan.fields.set("Assets/a.asset", object_id="11400000", property_path="/MonoBehaviour/health", value=120)
        await plan.objects.take("Assets/a.asset", object_id="11400000", side="source", commit="one")
        await plan.stage(paths=["Assets/a.asset"], include_local_changes=True)
        await plan.validate(level="unity", paths=["Assets/a.asset"], include_local_changes=True)
        self.assertEqual(client.calls[-1][1]["paths"], ["Assets/a.asset"])
        await plan.commit(paths=["Assets/a.asset"], message="Selected source changes", include_local_changes=True)
        self.assertEqual(client.calls[-1][1]["paths"], ["Assets/a.asset"])
        self.assertTrue(client.calls[-1][1]["include_local_changes"])
        with self.assertRaises(ValueError):
            await plan.files.take("Assets/model.fbx")

    async def test_module_facade_uses_default_client(self):
        client = MergeClient()
        with patch.object(locus, "_default_client", client):
            job = await locus.merges.prepare(sources=[{"range": "base..tip"}], workspace_ref=locus.WorkspaceRef("A"))
            self.assertEqual(job.id, "job-1")

    async def test_inspection_is_readonly_snapshot_bound_and_preserves_exact_ids(self):
        client = MergeClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}], workspace_ref=locus.WorkspaceRef("A", 4))
        path = "/MonoBehaviour/references/RefIds/@rid=9007199254740993/data/health"
        await job.inspect_asset("Assets/a.asset", version="source", commit="one", object_id="11400000",
            property_path=path, offset=10, limit=50, reference_offset=20)
        method, payload = client.calls[-1]
        self.assertEqual(method, "merges.inspect_asset")
        self.assertEqual(payload["property_path"], path)
        self.assertEqual(payload["object_id"], "11400000")
        self.assertEqual(payload["workspaceRef"]["checkoutId"], "destination")
        self.assertEqual(payload["reference_offset"], 20)
        self.assertFalse(any(method == "merges.new_plan" for method, _ in client.calls))
        with self.assertRaises(ValueError):
            await job.inspect_asset("Assets/a.asset", version="live")


if __name__ == "__main__":
    unittest.main()
