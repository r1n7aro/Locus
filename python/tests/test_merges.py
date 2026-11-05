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
        if method == "merges.replan":
            return {"workspace_ref": {"checkoutId": "destination", "expectedGeneration": 8},
                    "job": {"id": "job-2", "snapshot": {"head": "head"}, "project_root": "C:/project", "state": "prepared"},
                    "report": {"replaced_job_id": params["job_id"], "carried": {"decisions": 2},
                               "dropped": [{"path": "state.json", "reason": "destination_changed"}],
                               "dropped_total": 1, "changed_paths": ["state.json"]}}
        return {"ready_to_apply": True, "plan_hash": "hash", "state": "prepared"}


class MergeTests(unittest.IsolatedAsyncioTestCase):
    async def test_replan_returns_a_new_job_bound_to_the_returned_destination(self):
        client = MergeClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}], workspace_ref=locus.WorkspaceRef("A"))
        self.assertIsNone(job.replan_report)
        replanned = await job.replan()
        method, params = client.calls[-1]
        self.assertEqual(method, "merges.replan")
        self.assertEqual(params["job_id"], "job-1")
        self.assertEqual(params["workspaceRef"], job.workspace_ref)
        self.assertEqual(replanned.id, "job-2")
        self.assertEqual(replanned.workspace_ref["expectedGeneration"], 8)
        self.assertEqual(replanned.replan_report.dropped[0]["path"], "state.json")
        self.assertEqual(replanned.replan_report.carried["decisions"], 2)
        await replanned.plan().preview()
        self.assertEqual(client.calls[-1][1]["job_id"], "job-2")

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

    async def test_set_text_is_a_plan_file_operation_with_explicit_line_endings(self):
        client = MergeClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}], workspace_ref=locus.WorkspaceRef("A"))
        plan = job.plan()
        await plan.files.set_text("Assets/Code/Motor.cs", "merged\n")
        method, params = client.calls[-1]
        self.assertEqual(method, "merges.files.set_text")
        self.assertEqual((params["path"], params["content"], params["eol"]),
                         ("Assets/Code/Motor.cs", "merged\n", "target"))
        await plan.files.set_text("notes.txt", "x", eol="exact")
        self.assertEqual(client.calls[-1][1]["eol"], "exact")
        for kwargs in ({"eol": "auto"},):
            with self.assertRaises(ValueError):
                await plan.files.set_text("notes.txt", "x", **kwargs)
        with self.assertRaises(ValueError):
            await plan.files.set_text("notes.txt", b"bytes")

    async def test_all_changes_follows_pages(self):
        class PagedClient(MergeClient):
            async def rpc(self, method, params=None, **kwargs):
                if method == "merges.changes":
                    self.calls.append((method, params))
                    offset = params["offset"]
                    changes = [{"id": f"c{i}", "status": "clean"} for i in range(offset, min(offset + 2, 5))]
                    return {"changes": changes, "total": 5,
                            "next_offset": offset + 2 if offset + 2 < 5 else None}
                return await super().rpc(method, params, **kwargs)

        client = PagedClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}], workspace_ref=locus.WorkspaceRef("A"))
        changes = await job.all_changes(page_size=2)
        self.assertEqual([c.id for c in changes], ["c0", "c1", "c2", "c3", "c4"])
        self.assertEqual([p["offset"] for m, p in client.calls if m == "merges.changes"], [0, 2, 4])
        with self.assertRaises(ValueError):
            await job.all_changes(page_size=0)

    async def test_restore_sends_the_object_and_its_commit(self):
        client = MergeClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}], workspace_ref=locus.WorkspaceRef("A"))
        await job.plan().objects.restore("Assets/Level.unity", object_id="302", commit="one")
        method, params = client.calls[-1]
        self.assertEqual(method, "merges.objects.restore")
        self.assertEqual((params["path"], params["object_id"], params["commit"]), ("Assets/Level.unity", "302", "one"))

    async def test_filters_go_to_the_server_and_the_summary_follows_pages(self):
        class SummaryClient(MergeClient):
            async def rpc(self, method, params=None, **kwargs):
                if method == "merges.change_summary":
                    self.calls.append((method, params))
                    files = [{"path": "a.unity", "statuses": {"conflict": 1}}] if params["offset"] == 0                         else [{"path": "b.unity", "statuses": {"clean": 3}}]
                    return {"files": files, "file_total": 2, "changes": 4, "statuses": {"clean": 3, "conflict": 1},
                            "next_offset": 1 if params["offset"] == 0 else None}
                return await super().rpc(method, params, **kwargs)

        client = SummaryClient()
        job = await client.merges.prepare(sources=[{"commits": ["one"]}], workspace_ref=locus.WorkspaceRef("A"))
        await job.all_changes(status="conflict", paths=["a.unity"])
        method, params = client.calls[-1]
        self.assertEqual(method, "merges.changes")
        self.assertEqual((params["status"], params["paths"]), ("conflict", ["a.unity"]))
        await job.changes()
        self.assertNotIn("status", client.calls[-1][1])
        with self.assertRaises(ValueError):
            await job.changes(paths="a.unity")
        summary = await job.change_summary()
        self.assertEqual([f.path for f in summary.files], ["a.unity", "b.unity"])
        self.assertEqual(summary.statuses["conflict"], 1)
        self.assertNotIn("next_offset", summary)
        self.assertEqual([p["offset"] for m, p in client.calls if m == "merges.change_summary"], [0, 1])


if __name__ == "__main__":
    unittest.main()
