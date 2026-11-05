"""Explicit, snapshot-bound Unity integration plans.

All parsing and structural writes run in Locus's Rust asset core. These APIs do
not expose YAML text editing. ``apply`` keeps HEAD/index intact; Unity validation
currently imports the applied destination and is required before asset commits.
"""
from __future__ import annotations

import os
from typing import Any, TYPE_CHECKING

from ._scope import workspace_payload as _workspace_payload
from ._assets import AssetResult, _operations

if TYPE_CHECKING:
    from ._client import Client
    from ._models import WorkspaceRef


class MergeResult(dict[str, Any]):
    """JSON result with attribute access for preview/status fields."""

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError as error:
            raise AttributeError(name) from error


class Merges:
    def __init__(self, client: Client) -> None:
        self._client = client

    async def prepare(
        self,
        *,
        sources: list[dict[str, Any]],
        destination: dict[str, Any] | None = None,
        workspace_ref: WorkspaceRef | dict[str, Any] | None = None,
        worktree: Any = None,
        project_id: str | None = None,
        target_state: str = "working_tree",
        paths: list[str] | None = None,
        mode: str = "structural",
    ) -> MergeJob:
        if target_state != "working_tree":
            raise ValueError("target_state must be working_tree; existing staged/unstaged changes are preserved")
        if mode not in {"structural", "files"}:
            raise ValueError("mode must be structural or files")
        if paths is not None and (not isinstance(paths, list) or not paths or
                any(not isinstance(path, str) or not path.strip() for path in paths)):
            raise ValueError("paths must be a nonempty list of exact repository-relative files")
        if mode == "files" and paths is None:
            raise ValueError("File replacement requires explicit paths")
        destination = dict(destination or {"kind": "checkout"})
        references = [item for item in (destination.pop("workspace_ref", None),
            destination.pop("base_workspace_ref", None), workspace_ref) if item is not None]
        if len(references) > 1:
            raise ValueError("Specify one destination workspace reference")
        reference = references[0] if references else None
        payload = await self._client.rpc("merges.prepare", {
            "workspaceRef": _workspace_payload(reference, worktree),
            "execution_delegation": os.environ.get("LOCUS_SDK_EXECUTION_DELEGATION"),
            "sources": sources, "project_id": project_id, "destination": destination,
            "paths": paths, "mode": mode,
        }, timeout=300)
        return MergeJob(self._client, payload["workspace_ref"], payload["job"])

    async def get(self, job_id: str, *, workspace_ref: WorkspaceRef | dict[str, Any] | None = None, worktree: Any = None) -> MergeJob:
        reference = _workspace_payload(workspace_ref, worktree)
        payload = await self._client.rpc("merges.get", {"workspaceRef": reference, "job_id": job_id})
        return MergeJob(self._client, reference, payload)


class MergeJob:
    def __init__(self, client: Client, workspace_ref: dict[str, Any], payload: dict[str, Any]) -> None:
        self._client = client
        self.workspace_ref = workspace_ref
        self.id = payload["id"]
        self.snapshot = payload["snapshot"]
        self.destination = payload["project_root"]
        self.state = payload["state"]
        self.mode = payload.get("mode", "structural")
        self.paths = payload.get("paths")
        self.prepare_metrics = MergeResult(payload.get("prepare_metrics") or {})
        self.replan_report: MergeResult | None = None
        self.assets = MergeAssets(self)

    async def replan(self) -> MergeJob:
        """Re-prepare this job on the current destination, keeping its decisions.

        Use it after ``stale`` (the checkout changed after the snapshot). The new
        job uses this job's exact source commits and bases, so change IDs stay
        the same. A decision is carried over only when every file it touches has
        identical destination bytes; ``replan_report["dropped"]`` lists the rest
        and ``changed_paths`` the source files whose destination changed. The
        previous job is removed once the new one is saved. Preview the new plan
        before applying it.
        """
        payload = await self._client.rpc("merges.replan", {
            "workspaceRef": self.workspace_ref, "job_id": self.id,
            "execution_delegation": os.environ.get("LOCUS_SDK_EXECUTION_DELEGATION"),
        }, timeout=300)
        job = MergeJob(self._client, payload["workspace_ref"], payload["job"])
        job.replan_report = MergeResult(payload.get("report") or {})
        return job

    async def _call(self, action: str, params: dict[str, Any] | None = None, *, timeout: float = 300) -> MergeResult:
        payload = await self._client.rpc("merges." + action, {
            "workspaceRef": self.workspace_ref, "job_id": self.id,
            "execution_delegation": os.environ.get("LOCUS_SDK_EXECUTION_DELEGATION"), **(params or {}),
        }, timeout=timeout)
        return MergeResult(payload)

    @staticmethod
    def _filters(status: str | list[str] | None, paths: list[str] | None) -> dict[str, Any]:
        if isinstance(paths, str):
            raise ValueError("paths must be a list of exact repository-relative file paths")
        return {key: value for key, value in (("status", status), ("paths", paths)) if value is not None}

    async def changes(self, *, status: str | list[str] | None = None, paths: list[str] | None = None,
        offset: int = 0, limit: int = 100) -> MergeResult:
        """One catalog page, optionally filtered on the server by ``status`` and exact ``paths``.

        ``total`` counts the matching changes, ``catalog_total`` the whole catalog.
        """
        return await self._call("changes", {"offset": offset, "limit": limit, **self._filters(status, paths)})

    async def all_changes(self, *, status: str | list[str] | None = None, paths: list[str] | None = None,
        page_size: int = 1000) -> list[MergeResult]:
        """Every matching catalog change, following ``next_offset`` across pages.

        Statuses: ``clean`` (include applies; for text ``merge`` names a line
        merge), ``already_applied``, ``conflict`` (with ``reason``; text adds
        ``hunks``) and ``explicit_choice`` (file mode). ``status="conflict"``
        reads only the changes that need a decision.
        """
        if not 1 <= page_size <= 1000:
            raise ValueError("page_size must be between 1 and 1000")
        items: list[MergeResult] = []
        offset: int | None = 0
        while offset is not None:
            page = await self.changes(status=status, paths=paths, offset=offset, limit=page_size)
            items.extend(MergeResult(change) for change in page.get("changes", []))
            offset = page.get("next_offset")
        return items

    async def change_summary(self) -> MergeResult:
        """Per-file rollup of the whole catalog, following pages.

        Returns ``files`` (each with ``path``, ``file_kind``, ``commits``,
        ``changes``, ``statuses``, ``kinds``, ``destination_changed`` and
        ``outside_destination_scope``), ``file_total``, ``changes`` and total
        ``statuses``.
        """
        summary: MergeResult | None = None
        offset: int | None = 0
        while offset is not None:
            page = await self._call("change_summary", {"offset": offset, "limit": 1000})
            if summary is None:
                summary = page
            else:
                summary["files"].extend(page.get("files", []))
            offset = page.get("next_offset")
        assert summary is not None
        summary.pop("next_offset", None)
        summary["files"] = [MergeResult(item) for item in summary.get("files", [])]
        return summary

    async def snapshot_page(self, *, kind: str = "target", offset: int = 0, limit: int = 100) -> MergeResult:
        """Page immutable file identities; ``snapshot`` contains only summary metadata."""
        if kind not in {"target", "dependencies"}:
            raise ValueError("kind must be target or dependencies")
        return await self._call("snapshot", {"kind": kind, "offset": offset, "limit": limit})

    async def new_plan(self, *, default: str = "keep_target") -> MergePlan:
        await self._call("new_plan", {"default": default})
        return MergePlan(self)

    async def inspect_asset(self, path: str, *, version: str = "target", commit: str | None = None,
        object_id: str | None = None, property_path: str | None = None, offset: int = 0,
        reference_offset: int = 0, limit: int = 100, scalar_limit: int = 4096) -> MergeResult:
        """Inspect frozen objects, all fields and references without reading/editing YAML.

        Field paths match ``fields.set`` (including stable managed-reference IDs).
        ``scalar_text`` retains lexical spelling; inspect ``scalar_style`` before
        interpreting it. Reference paths address this immutable snapshot.
        """
        if version not in {"target", "source", "base", "result"}:
            raise ValueError("version must be target, source, base, or result")
        return await self._call("inspect_asset", {"path": path, "version": version,
            "commit": commit, "object_id": object_id, "property_path": property_path,
            "offset": offset, "reference_offset": reference_offset, "limit": limit,
            "scalar_limit": scalar_limit})

    def plan(self) -> MergePlan:
        """Resume the saved plan without clearing its decisions."""
        return MergePlan(self)


class _Scope:
    def __init__(self, plan: MergePlan, name: str) -> None:
        self._plan, self._name = plan, name

    async def include(self, path: str, **selector: Any) -> MergeResult:
        return await self._plan._call(self._name + ".include", {"path": path, **selector})

    async def exclude(self, path: str, **selector: Any) -> MergeResult:
        return await self._plan._call(self._name + ".exclude", {"path": path, **selector})

    async def take(self, path: str, *, version: str | dict[str, Any] | None = None, side: str | None = None, **selector: Any) -> MergeResult:
        if self._name == "files":
            if version is None:
                raise ValueError("Binary/whole-file selection requires version=target/source/base (and commit for multiple source variants)")
            return await self._plan._call("files.take", {"path": path, "version": version, **selector})
        return await self._plan._call(self._name + ".take", {"path": path, "side": side or version or "source", **selector})


class _Files(_Scope):
    async def delete(self, path: str) -> MergeResult:
        return await self._plan._call("files.delete", {"path": path})

    async def move(self, path: str, destination: str) -> MergeResult:
        return await self._plan._call("files.move", {"path": path, "destination": destination})

    async def clear(self, path: str) -> MergeResult:
        """Remove a whole-file operation before using interior field selections."""
        return await self._plan._call("files.clear", {"path": path})

    async def set_text(self, path: str, content: str, *, eol: str = "target") -> MergeResult:
        """Resolve a code/plain-text change with merged content inside the plan.

        The text is stored in the job (bound to the plan hash, written by
        ``apply`` and restored by ``abort``) and supersedes include/resolve for
        this path. ``eol="target"`` converts LF to the destination's CRLF
        convention; ``eol="exact"`` writes the content unchanged. Unity YAML
        and binary files are rejected: use field/object operations or ``take``.
        """
        if not isinstance(content, str):
            raise ValueError("content must be a str")
        if eol not in {"target", "exact"}:
            raise ValueError("eol must be 'target' or 'exact'")
        return await self._plan._call("files.set_text", {"path": path, "content": content, "eol": eol})


class _Objects(_Scope):
    async def move(self, path: str, *, object_id: str, parent_id: str, position: int | None = None) -> MergeResult:
        """Reparent a Transform/RectTransform by exact fileID, updating both child lists atomically."""
        return await self._plan._call("objects.move", {"path": path, "object_id": object_id,
            "parent_id": parent_id, "position": position})

    async def clear(self, path: str, *, object_id: str) -> MergeResult:
        return await self._plan._call("objects.clear", {"path": path, "object_id": object_id})

    async def add(self, path: str, *, object_id: str, commit: str | None = None) -> MergeResult:
        return await self._plan._call("objects.add", {"path": path, "object_id": object_id, "commit": commit})

    async def delete(self, path: str, *, object_id: str, commit: str | None = None) -> MergeResult:
        return await self._plan._call("objects.delete", {"path": path, "object_id": object_id, "commit": commit})

    async def restore(self, path: str, *, object_id: str, commit: str | None = None) -> MergeResult:
        """Bring back an object the destination deleted, with what it needs to load.

        Restores the deleted part of the hierarchy it belongs to (its GameObject,
        components and children, up to the nearest object the destination kept)
        and adds it to that object's ``m_Component``/``m_Children`` or the scene's
        ``m_Roots``. ``result.restored`` lists the ``objects``, the
        ``resolved_change_ids`` and the ``attached`` list elements.
        """
        return await self._plan._call("objects.restore", {"path": path, "object_id": object_id, "commit": commit})


class _Fields(_Scope):
    async def delete(self, path: str, *, object_id: str, property_path: str, commit: str | None = None) -> MergeResult:
        return await self._plan._call("fields.delete", {"path": path, "object_id": object_id,
            "property_path": property_path, "commit": commit})

    async def set(self, path: str, *, object_id: str, property_path: str, value: Any, commit: str | None = None) -> MergeResult:
        """Set typed JSON; numeric fileID/rid values use Python int, while strings stay quoted."""
        return await self._plan._call("fields.set", {"path": path, "object_id": object_id,
            "property_path": property_path, "value": value, "commit": commit})


class MergePlan:
    def __init__(self, job: MergeJob) -> None:
        self.job = job
        self.files = _Files(self, "files")
        self.objects = _Objects(self, "objects")
        self.fields = _Fields(self, "fields")
        self.assets = MergeAssets(job)

    async def _call(self, action: str, params: dict[str, Any] | None = None, **kwargs: Any) -> MergeResult:
        return await self.job._call(action, params, **kwargs)

    async def include(self, *, change_ids: list[str] | None = None, **selector: Any) -> MergeResult:
        return await self._call("include", {**({"change_ids": change_ids} if change_ids is not None else {}), **selector})

    async def exclude(self, *, change_ids: list[str] | None = None, **selector: Any) -> MergeResult:
        return await self._call("exclude", {**({"change_ids": change_ids} if change_ids is not None else {}), **selector})

    async def defer(self, *, change_ids: list[str] | None = None, **selector: Any) -> MergeResult:
        return await self._call("defer", {**({"change_ids": change_ids} if change_ids is not None else {}), **selector})

    async def resolve(self, conflict_id: str, *, side: str) -> MergeResult:
        return await self._call("resolve", {"change_ids": [conflict_id], "side": side})

    async def preview(self) -> MergeResult:
        return await self._call("preview")

    async def check_dependencies(self) -> MergeResult:
        return await self._call("check_dependencies")

    async def validate(self, *, level: str = "static", paths: list[str] | None = None, include_local_changes: bool = False) -> MergeResult:
        """Validate applied state, or an exact commit subset in an isolated Unity pool checkout."""
        if paths is not None and level != "unity":
            raise ValueError("Explicit commit-scope paths require level='unity'")
        return await self._call("validate", {"level": level, "paths": paths,
            "include_local_changes": include_local_changes}, timeout=1800 if paths is not None else 900)

    async def apply(self, *, expected_plan_hash: str, index_policy: str = "preserve") -> MergeResult:
        return await self._call("apply", {"expected_plan_hash": expected_plan_hash, "index_policy": index_policy})

    async def stage(self, *, paths: list[str], include_local_changes: bool = False) -> MergeResult:
        return await self._call("stage", {"paths": paths, "include_local_changes": include_local_changes})

    async def commit(self, *, paths: list[str], message: str, include_local_changes: bool = False) -> MergeResult:
        return await self._call("commit", {"paths": paths, "message": message, "include_local_changes": include_local_changes})

    async def abort(self) -> MergeResult:
        return await self._call("abort")


class MergeAssets:
    """The shared asset operations applied to the merge plan's result snapshot.

    This destination persists selections in the plan. Only ``plan.apply`` writes
    project files; asset preview/apply here always report ``persisted=False``.
    """

    def __init__(self, job: MergeJob) -> None:
        self._job = job

    async def read(self, path: str) -> AssetResult:
        return AssetResult(await self._job._call("assets.read", {"path": path,
            "destination": "merge_plan", "persist": "plan"}))

    async def _edit(self, action: str, path: str, operations: list[dict[str, Any]], *,
                    expected_revision: str | None, persist: str) -> AssetResult:
        if persist != "plan":
            raise ValueError("Merge asset edits persist selections in the plan; use plan.apply() to write disk")
        if (action == "apply" or expected_revision is not None) and (not isinstance(expected_revision, str) or not expected_revision.strip()):
            raise ValueError("Merge asset edits require expected_revision from the current result snapshot")
        return AssetResult(await self._job._call("assets." + action, {"path": path,
            "operations": _operations(operations), "expected_revision": expected_revision,
            "destination": "merge_plan", "persist": persist}))

    async def preview(self, path: str, operations: list[dict[str, Any]], *,
                      expected_revision: str | None = None, persist: str = "plan") -> AssetResult:
        return await self._edit("preview", path, operations, expected_revision=expected_revision, persist=persist)

    async def apply(self, path: str, operations: list[dict[str, Any]], *,
                    expected_revision: str, persist: str = "plan") -> AssetResult:
        return await self._edit("apply", path, operations, expected_revision=expected_revision, persist=persist)
