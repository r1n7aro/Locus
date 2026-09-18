"""Effective workspace rules for an installed Locus Agent."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True, slots=True)
class AgentRule:
    key: str
    file_name: str
    title: str
    enabled: bool
    order: int
    source: str
    read_only: bool
    updated_at: int
    plugin_id: str | None = None
    plugin_scope: str | None = None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "AgentRule":
        return cls(
            key=payload["key"], file_name=payload["fileName"], title=payload["title"],
            enabled=bool(payload["enabled"]), order=int(payload["order"]),
            source=payload["source"], read_only=bool(payload.get("readOnly", False)),
            updated_at=int(payload.get("updatedAt", 0)),
            plugin_id=payload.get("pluginId"), plugin_scope=payload.get("pluginScope"),
        )
