"""Checkout selectors shared by tools, lifecycle, worktrees and merge jobs."""
from __future__ import annotations

import os
from typing import Any


def workspace_payload(workspace_ref: Any = None, worktree: Any = None, *, required: bool = True) -> dict[str, Any] | None:
    if workspace_ref is not None and worktree is not None:
        raise ValueError("Specify either worktree or workspace_ref, not both")
    reference = worktree if worktree is not None else workspace_ref
    if reference is not None:
        reference = getattr(reference, "workspace_ref", reference)
        if hasattr(reference, "to_payload"):
            reference = reference.to_payload()
        if not isinstance(reference, dict) or not str(reference.get("checkoutId", "")).strip():
            raise ValueError("Use a Worktree, WorkspaceRef or workspace reference dictionary; resolve checkout IDs with worktrees.get()")
        return dict(reference)
    checkout = os.environ.get("LOCUS_CHECKOUT_ID", "").strip()
    if not checkout:
        if required:
            raise ValueError("Specify worktree or workspace_ref, or run in a Locus checkout session")
        return None
    generation = os.environ.get("LOCUS_WORKSPACE_GENERATION")
    epoch = os.environ.get("LOCUS_MATERIALIZATION_EPOCH")
    return {"checkoutId": checkout,
        "expectedGeneration": int(generation) if generation else None,
        "expectedMaterializationEpoch": int(epoch) if epoch is not None else None}


def unity_target(project: str | None, workspace_ref: Any = None, worktree: Any = None) -> dict[str, Any]:
    if project is not None:
        if workspace_ref is not None or worktree is not None:
            raise ValueError("Specify project or worktree/workspace_ref, not both")
        if not project.strip():
            raise ValueError("project cannot be empty")
        return {"project": project.strip()}
    return {"workspaceRef": workspace_payload(workspace_ref, worktree)}
